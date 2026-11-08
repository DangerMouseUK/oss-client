//! Flags as the client sees them: where they lie, who owns them, and when touching one
//! should ask the server for it.
//!
//! Two kinds, as in Continuum. **Dropped** flags (warzone/carry games) are placed by
//! S2C `0x12`, vanish when someone picks one up (`0x13`), and count toward the
//! carrier's total. **Turf** flags sit on the map's flag tiles (`0x22` lists their
//! owners by flag id, in the order the tiles appear in the `.lvl` file); touching one
//! claims it for your team and it stays where it is.
//!
//! Touch rules follow nullspace (MIT): ships on the flag's team ignore it; carriers at
//! the `CarryFlags` limit ignore dropped flags; a touched dropped flag is hidden for
//! 3 s; requests repeat at most every 0.2 s. One deliberate difference: nullspace only
//! checks touches when `CarryFlags` is non-zero, which would make turf flags (normally
//! `CarryFlags = 0`) unclaimable, so turf claims are checked regardless.

/// `freq` value for a flag nobody owns.
pub const NEUTRAL: u16 = 0xFFFF;
/// Most flags tracked (ASSS's limit is 255).
const MAX_FLAGS: usize = 256;
/// A touched dropped flag is hidden this long (s), as in Continuum.
const HIDE_AFTER_TOUCH: f32 = 3.0;
/// Least time between pickup requests for one flag (s).
const REQUEST_INTERVAL: f32 = 0.2;

#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    /// Tile position.
    pub x: i32,
    pub y: i32,
    /// Owning team, or [`NEUTRAL`].
    pub owner: u16,
    /// Lying on the map (not carried, not off the map).
    pub on_map: bool,
    pub turf: bool,
    hidden_until: f32,
    last_request: f32,
}

impl Flag {
    fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            owner: NEUTRAL,
            on_map: false,
            turf: false,
            hidden_until: 0.0,
            last_request: f32::NEG_INFINITY,
        }
    }

    /// Drawn and touchable now.
    pub fn visible(&self, clock: f32) -> bool {
        self.on_map && clock >= self.hidden_until
    }
}

/// The ship touching flags: its centre in pixels, radius, team and carried count.
#[derive(Debug, Clone, Copy)]
pub struct Toucher {
    pub x: i32,
    pub y: i32,
    pub radius: i32,
    pub freq: u16,
    pub carried: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Flags {
    pub flags: Vec<Flag>,
    /// The map's flag tiles, in file order (turf flag `i` is at `static_tiles[i]`).
    static_tiles: Vec<(i32, i32)>,
}

impl Flags {
    pub fn new(static_tiles: Vec<(i32, i32)>) -> Self {
        Self {
            flags: Vec::new(),
            static_tiles,
        }
    }

    fn slot(&mut self, id: u16) -> Option<&mut Flag> {
        let i = id as usize;
        if i >= MAX_FLAGS {
            return None;
        }
        if self.flags.len() <= i {
            self.flags.resize(i + 1, Flag::new());
        }
        self.flags.get_mut(i)
    }

    /// S2C `0x12`: flag `id` lies at tile (x, y). Off-map coordinates take it away.
    pub fn location(&mut self, id: u16, x: u16, y: u16, freq: u16) {
        if let Some(f) = self.slot(id) {
            f.on_map = x < 1024 && y < 1024;
            (f.x, f.y, f.owner) = (x as i32, y as i32, freq);
            f.turf = false;
            f.hidden_until = 0.0;
        }
    }

    /// S2C `0x13`: someone on team `freq` picked flag `id` up (claimed it, for turf
    /// flags or when flags aren't carried).
    pub fn pickup(&mut self, id: u16, freq: u16, carry_flags: u8) {
        if let Some(f) = self.slot(id) {
            if f.turf || carry_flags == 0 {
                f.owner = freq;
            } else {
                f.on_map = false;
            }
        }
    }

    /// S2C `0x22`: owners of every turf flag, by id (-1 = none).
    pub fn turf(&mut self, owners: &[i16]) {
        for (i, &owner) in owners.iter().enumerate().take(MAX_FLAGS) {
            let Some(&(x, y)) = self.static_tiles.get(i) else {
                break;
            };
            if let Some(f) = self.slot(i as u16) {
                (f.x, f.y) = (x, y);
                f.owner = if owner < 0 { NEUTRAL } else { owner as u16 };
                f.on_map = true;
                f.turf = true;
            }
        }
    }

    /// S2C `0x14`: the game was won; dropped flags go until the server places them.
    pub fn victory(&mut self) {
        for f in self.flags.iter_mut().filter(|f| !f.turf) {
            f.on_map = false;
        }
    }

    /// Flags your ship touches that you should ask for. Marks them requested (and
    /// hides dropped ones, as Continuum does while the server decides).
    pub fn touched(&mut self, me: Toucher, carry_flags: u8, clock: f32) -> Vec<u16> {
        let limit = match carry_flags {
            0 | 1 => u32::MAX,
            n => n as u32 - 1,
        };
        let r = me.radius + 1;
        let mut out = Vec::new();
        for (id, f) in self.flags.iter_mut().enumerate() {
            if !f.visible(clock) || f.owner == me.freq {
                continue;
            }
            if !f.turf && (carry_flags == 0 || me.carried >= limit) {
                continue;
            }
            let (fx, fy) = (f.x * 16, f.y * 16);
            let hit = me.x + r > fx && me.x - r < fx + 16 && me.y + r > fy && me.y - r < fy + 16;
            if !hit {
                continue;
            }
            if !f.turf {
                f.hidden_until = clock + HIDE_AFTER_TOUCH;
            }
            if clock - f.last_request >= REQUEST_INTERVAL {
                f.last_request = clock;
                out.push(id as u16);
            }
        }
        out
    }

    /// Flags owned by `freq` (for the HUD).
    pub fn owned_by(&self, freq: u16) -> usize {
        self.flags
            .iter()
            .filter(|f| f.owner == freq && (f.on_map || f.turf))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(x: i32, y: i32, freq: u16, carried: u32) -> Toucher {
        Toucher {
            x,
            y,
            radius: 14,
            freq,
            carried,
        }
    }

    #[test]
    fn dropped_flags_are_requested_hidden_and_carried() {
        let mut f = Flags::default();
        f.location(2, 100, 200, NEUTRAL);
        assert!(f.flags[2].visible(0.0));
        assert!(!f.flags[0].visible(0.0));
        let on = ship(100 * 16 + 8, 200 * 16 + 8, 1, 0);
        // CarryFlags 0: dropped flags can't be picked up.
        assert!(f.touched(on, 0, 1.0).is_empty());
        assert_eq!(f.touched(on, 1, 1.0), [2]);
        assert!(!f.flags[2].visible(1.5), "hidden while the server decides");
        assert!(f.touched(on, 1, 1.1).is_empty());
        f.pickup(2, 1, 1);
        assert!(!f.flags[2].on_map);
        // A carrier at the limit (CarryFlags 3 = two flags) leaves flags alone.
        f.location(3, 10, 10, NEUTRAL);
        let full = ship(10 * 16 + 8, 10 * 16 + 8, 1, 2);
        assert!(f.touched(full, 3, 9.0).is_empty());
        assert_eq!(f.touched(Toucher { carried: 1, ..full }, 3, 9.0), [3]);
        // Off the map.
        f.location(3, 0xFFFF, 0xFFFF, NEUTRAL);
        assert!(!f.flags[3].on_map);
    }

    #[test]
    fn turf_flags_stay_and_change_owner() {
        let mut f = Flags::new(vec![(5, 6), (50, 60)]);
        f.turf(&[-1, 1, 0]);
        assert_eq!(f.flags.len(), 2, "no tile for a third flag");
        assert_eq!((f.flags[1].x, f.flags[1].y, f.flags[1].owner), (50, 60, 1));
        // Your own team's flag is ignored; an enemy one is claimed (even with
        // CarryFlags 0) and stays visible.
        let at = |x: i32, y: i32, freq| ship(x * 16 + 8, y * 16 + 8, freq, 0);
        assert!(f.touched(at(50, 60, 1), 0, 1.0).is_empty());
        assert_eq!(f.touched(at(50, 60, 0), 0, 1.0), [1]);
        assert!(f.flags[1].visible(1.1));
        assert!(f.touched(at(50, 60, 0), 0, 1.1).is_empty(), "rate limited");
        assert_eq!(f.touched(at(50, 60, 0), 0, 1.3), [1]);
        f.pickup(1, 0, 0);
        assert_eq!(f.flags[1].owner, 0);
        assert!(f.flags[1].on_map);
        assert_eq!(f.owned_by(0), 1);
        f.victory();
        assert!(f.flags[1].on_map, "turf flags stay after a win");
    }
}
