//! Ship state: stats, items, capabilities, toggles and timers.

use crate::fixed::{ENERGY_SCALE, Pos, Vel};
use crate::settings::ShipSettings;

/// One tick of player intent. Fields marked "edge" are true only on the tick the key
/// went down; the rest are held state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShipInput {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub afterburner: bool,
    pub gun: bool,
    pub bomb: bool,
    pub mine: bool,
    pub repel: bool,
    pub burst: bool,
    pub thor: bool,
    pub decoy: bool,
    pub rocket: bool,
    pub brick: bool,
    /// Edge.
    pub portal: bool,
    /// Edge.
    pub warp: bool,
    /// Edge.
    pub toggle_multifire: bool,
    /// Edge.
    pub toggle_stealth: bool,
    /// Edge.
    pub toggle_cloak: bool,
    /// Edge.
    pub toggle_xradar: bool,
    /// Edge.
    pub toggle_antiwarp: bool,
}

/// Life state of a ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipState {
    Alive,
    /// Dead; respawns on `respawn_tick`.
    Dead {
        respawn_tick: u32,
    },
}

/// Current (upgradeable) ship stats in settings units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShipStats {
    pub rotation: u32,
    pub thrust: u32,
    pub speed: u32,
    pub recharge: u32,
    pub max_energy: u32,
}

impl ShipStats {
    pub const fn initial(s: &ShipSettings) -> Self {
        Self {
            rotation: min(s.initial_rotation, s.maximum_rotation),
            thrust: min(s.initial_thrust, s.maximum_thrust),
            speed: min(s.initial_speed, s.maximum_speed),
            recharge: min(s.initial_recharge, s.maximum_recharge),
            max_energy: min(s.initial_energy, s.maximum_energy),
        }
    }
}

const fn min(a: u32, b: u32) -> u32 {
    if a < b { a } else { b }
}

/// Counted items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Items {
    pub repels: u8,
    pub bursts: u8,
    pub decoys: u8,
    pub thors: u8,
    pub bricks: u8,
    pub rockets: u8,
    pub portals: u8,
}

impl Items {
    pub fn initial(s: &ShipSettings) -> Self {
        Self {
            repels: s.initial_repel.min(s.repel_max),
            bursts: s.initial_burst.min(s.burst_max),
            decoys: s.initial_decoy.min(s.decoy_max),
            thors: s.initial_thor.min(s.thor_max),
            bricks: s.initial_brick.min(s.brick_max),
            rockets: s.initial_rocket.min(s.rocket_max),
            portals: s.initial_portal.min(s.portal_max),
        }
    }
}

/// Capability bits (what the ship *can* do, mostly earned from prizes).
pub mod cap {
    pub const STEALTH: u8 = 1 << 0;
    pub const CLOAK: u8 = 1 << 1;
    pub const XRADAR: u8 = 1 << 2;
    pub const ANTIWARP: u8 = 1 << 3;
    pub const MULTIFIRE: u8 = 1 << 4;
    pub const BOUNCING_BULLETS: u8 = 1 << 5;
    pub const PROXIMITY: u8 = 1 << 6;
}

/// Toggle bits (what is switched on). Same bit layout as Continuum's position packet
/// status byte for the first four.
pub mod toggle {
    pub const STEALTH: u8 = 1 << 0;
    pub const CLOAK: u8 = 1 << 1;
    pub const XRADAR: u8 = 1 << 2;
    pub const ANTIWARP: u8 = 1 << 3;
    /// Brief flash shown when cloak breaks or a ship warps.
    pub const FLASH: u8 = 1 << 4;
    pub const SAFETY: u8 = 1 << 5;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ship {
    pub id: u16,
    /// Ship type 0..=7 (Warbird..Shark).
    pub kind: u8,
    pub freq: u16,
    pub state: ShipState,
    pub pos: Pos,
    pub vel: Vel,
    /// Orientation in 1/40000 turn, 0 = up, clockwise.
    pub rotation: u32,
    /// Energy in 1/1000 points.
    pub energy: i32,
    pub stats: ShipStats,
    /// Gun and bomb level, 0 = none, 1..=3.
    pub guns: u8,
    pub bombs: u8,
    /// Shrapnel pieces per bomb.
    pub shrapnel: u8,
    pub items: Items,
    /// [`cap`] bits.
    pub caps: u8,
    /// [`toggle`] bits.
    pub toggles: u8,
    pub multifire: bool,
    pub bounty: u32,
    /// Whether this world applies damage to this ship (true for the local ship; remote
    /// ships report their own damage over the network).
    pub takes_damage: bool,
    /// Driven by network updates rather than local input: never respawns, warps or
    /// picks up prizes on its own.
    pub remote: bool,
    /// Remote ships: a correction blended in over the next `lerp_ticks` ticks (added to
    /// the position each tick) instead of jumping to the reported position.
    pub lerp: Vel,
    pub lerp_ticks: u32,
    /// Afterburners were burning energy on the last tick.
    pub afterburning: bool,
    /// -1 reverse, 0 none, 1 forward thrust on the last tick (for exhaust effects).
    pub thrusting: i8,
    pub next_gun_tick: u32,
    pub next_bomb_tick: u32,
    pub next_repel_tick: u32,
    /// Timed effects (tick they end; in the past = inactive).
    pub super_end_tick: u32,
    pub shield_end_tick: u32,
    pub shutdown_end_tick: u32,
    pub rocket_end_tick: u32,
    pub emp_end_tick: u32,
    pub repelled_end_tick: u32,
    pub fake_antiwarp_end_tick: u32,
    /// Active portal: where it is and when it expires.
    pub portal: Option<(Pos, u32)>,
    /// Position/rotation before the last tick, for render interpolation.
    pub prev_pos: Pos,
    pub prev_rotation: u32,
}

impl Ship {
    /// A fresh ship with initial stats (not yet placed or prized).
    pub fn new(id: u16, kind: u8, freq: u16, s: &ShipSettings) -> Self {
        let stats = ShipStats::initial(s);
        Self {
            id,
            kind,
            freq,
            state: ShipState::Alive,
            pos: Pos::default(),
            vel: Vel::default(),
            rotation: 0,
            energy: stats.max_energy as i32 * ENERGY_SCALE,
            stats,
            guns: s.initial_guns.min(s.max_guns),
            bombs: s.initial_bombs.min(s.max_bombs),
            shrapnel: 0,
            items: Items::initial(s),
            caps: 0,
            toggles: 0,
            multifire: false,
            bounty: 0,
            takes_damage: true,
            remote: false,
            lerp: Vel::default(),
            lerp_ticks: 0,
            afterburning: false,
            thrusting: 0,
            next_gun_tick: 0,
            next_bomb_tick: 0,
            next_repel_tick: 0,
            super_end_tick: 0,
            shield_end_tick: 0,
            shutdown_end_tick: 0,
            rocket_end_tick: 0,
            emp_end_tick: 0,
            repelled_end_tick: 0,
            fake_antiwarp_end_tick: 0,
            portal: None,
            prev_pos: Pos::default(),
            prev_rotation: 0,
        }
    }

    /// One of the 40 discrete directions (0 = up) used for thrust, firing and sprites.
    pub const fn direction(&self) -> u8 {
        (self.rotation / 1000 % 40) as u8
    }

    pub const fn is_alive(&self) -> bool {
        matches!(self.state, ShipState::Alive)
    }

    /// Energy in whole points.
    pub const fn energy_points(&self) -> i32 {
        self.energy / ENERGY_SCALE
    }

    pub const fn has(&self, capability: u8) -> bool {
        self.caps & capability != 0
    }

    pub const fn is_on(&self, t: u8) -> bool {
        self.toggles & t != 0
    }
}

/// `a` is at or after `b` (wrapping tick compare).
pub const fn tick_reached(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

/// `a` is strictly after `b`.
pub const fn tick_after(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

/// Whether a timed effect ending at `end` is still running at `now`.
pub const fn active(now: u32, end: u32) -> bool {
    tick_after(end, now)
}

/// The later of two ticks.
pub const fn later(a: u32, b: u32) -> u32 {
    if tick_after(b, a) { b } else { a }
}
