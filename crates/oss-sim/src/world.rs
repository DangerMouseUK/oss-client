//! The fixed-step world: map, ships, projectiles, greens, doors and bricks.
//!
//! Behaviour follows the Continuum client as documented by the nullspace client (MIT):
//! thrust/rotation/speed/afterburner/energy order, per-axis wall collision with
//! `16 / BounceFactor` damping, 40-direction thrust and firing.

use std::collections::BTreeMap;

use crate::fixed::VieRng;
use crate::fixed::{
    DIRECTION_HEADINGS, ENERGY_SCALE, Pos, ROTATION_UNITS, Rng, VEL_SCALE, Vel, isqrt, scale_unit,
};
use crate::map::{MAP_TILES, TileMap, is_solid_id, tile};
use crate::prize::Green;
use crate::settings::ArenaSettings;
use crate::ship::{Ship, ShipInput, ShipState, active, cap, tick_reached, toggle};
use crate::weapons::{Projectile, ProjectileKind};

/// Things that happened during a tick, for sound, effects, messages and networking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    FiredGun {
        ship: u16,
        level: u8,
        multifire: bool,
    },
    FiredBomb {
        ship: u16,
        level: u8,
    },
    LaidMine {
        ship: u16,
        level: u8,
    },
    BurstFired {
        ship: u16,
    },
    ThorFired {
        ship: u16,
    },
    DecoyLaunched {
        ship: u16,
    },
    Repelled {
        ship: u16,
        pos: Pos,
    },
    RocketStarted {
        ship: u16,
    },
    /// A brick was used at tile (x, y) by a ship facing `direction` (0..40).
    BrickDropped {
        ship: u16,
        x: i32,
        y: i32,
        direction: u8,
    },
    PortalPlaced {
        ship: u16,
        pos: Pos,
    },
    Warped {
        ship: u16,
        from: Pos,
        to: Pos,
    },
    /// Warp/portal refused because an enemy's antiwarp is in range.
    Antiwarped {
        ship: u16,
    },
    /// Warp refused: it needs full energy.
    WarpNeedsEnergy {
        ship: u16,
    },
    WallBounce {
        ship: u16,
        speed: i64,
    },
    /// A projectile ended at `pos`: bullets fizzle, bombs/mines/thors explode.
    Explosion {
        pos: Pos,
        kind: ProjectileKind,
        level: u8,
        emp: bool,
    },
    ShipHit {
        ship: u16,
        shooter: u16,
        damage: u32,
    },
    ShipKilled {
        ship: u16,
        killer: u16,
        pos: Pos,
    },
    Respawned {
        ship: u16,
    },
    MultifireToggled {
        ship: u16,
        on: bool,
    },
    /// A stealth/cloak/xradar/antiwarp toggle ([`toggle`] bit) changed.
    Toggled {
        ship: u16,
        toggle: u8,
        on: bool,
    },
    /// Prize `prize` (negative = bad) was applied to a ship.
    PrizeTaken {
        ship: u16,
        prize: i32,
    },
    /// A ship that takes damage (the local one) picked up the green on tile (x, y).
    GreenTaken {
        ship: u16,
        x: u16,
        y: u16,
        prize: i32,
    },
    /// Doors changed state (new door seed applied).
    DoorsChanged {
        seed: u8,
    },
}

/// A brick on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brick {
    pub freq: u16,
    pub end_tick: u32,
}

/// Map plus bricks: what's solid for a given team.
pub(crate) struct Terrain<'a> {
    pub map: &'a TileMap,
    pub bricks: &'a BTreeMap<(i32, i32), Brick>,
}

impl Terrain<'_> {
    /// Solid for `freq`: your own team's bricks let you (and your weapons) through.
    pub fn is_solid(&self, x: i32, y: i32, freq: u16) -> bool {
        let id = self.map.get(x, y);
        if id == tile::BRICK && self.bricks.get(&(x, y)).is_some_and(|b| b.freq == freq) {
            return false;
        }
        is_solid_id(id)
    }
}

/// The simulation state.
#[derive(Debug, Clone)]
pub struct World {
    pub tick: u32,
    pub map: TileMap,
    pub settings: ArenaSettings,
    pub ships: Vec<Ship>,
    pub projectiles: Vec<Projectile>,
    /// Tile around which ships spawn.
    pub spawn_center: (i32, i32),
    /// Spawn scatter radius in tiles.
    pub spawn_radius: i32,
    /// Seed shared with the server/other clients for greens (0 = no greens).
    pub prize_seed: u32,
    /// When true the world places bricks itself (offline); online, bricks arrive
    /// from the server via [`World::place_bricks`].
    pub local_bricks: bool,
    pub(crate) rng: Rng,
    pub(crate) next_projectile_id: u32,
    pub(crate) greens: Vec<Green>,
    pub(crate) green_ticks: u32,
    pub(crate) bricks: BTreeMap<(i32, i32), Brick>,
    /// Every door tile: (x, y, original door id 162..=169).
    doors: Vec<(i32, i32, u8)>,
    /// Door generator; shared with the server and other clients online.
    door_rng: VieRng,
    /// Tick of the last door change; doors change every `DoorDelay` ticks after it.
    last_door_tick: u32,
    /// Players in the arena (spectators included), for the green count and spawn
    /// area. `None` counts the ships in the world.
    pub player_count: Option<u32>,
    /// Top-left tiles of wormholes (for gravity).
    wormholes: Vec<(i32, i32)>,
}

/// Most door changes replayed at once when catching up.
const MAX_DOOR_CATCH_UP: u32 = 100_000;

impl World {
    pub fn new(map: TileMap, settings: ArenaSettings, seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let prize_seed = rng.next_u32() | 1;
        let door_rng = VieRng::new(rng.next_u32());
        let door_delay = settings.door_delay.max(1);
        let mut w = Self {
            tick: 0,
            map,
            settings,
            ships: Vec::new(),
            projectiles: Vec::new(),
            spawn_center: (512, 512),
            spawn_radius: 20,
            prize_seed,
            local_bricks: true,
            rng,
            next_projectile_id: 1,
            greens: Vec::new(),
            green_ticks: 0,
            bricks: BTreeMap::new(),
            doors: Vec::new(),
            door_rng,
            // Due on the first tick.
            last_door_tick: 0u32.wrapping_sub(door_delay),
            player_count: None,
            wormholes: Vec::new(),
        };
        w.spawn_radius = w.settings.warp_radius_limit.clamp(3, 512) as i32;
        for (i, &id) in w.map.cells().iter().enumerate() {
            let (x, y) = ((i % MAP_TILES) as i32, (i / MAP_TILES) as i32);
            if (tile::FIRST_DOOR..=tile::LAST_DOOR).contains(&id) {
                w.doors.push((x, y, id));
            }
            if id == tile::WORMHOLE
                && w.map.get(x - 1, y) != tile::WORMHOLE
                && w.map.get(x, y - 1) != tile::WORMHOLE
            {
                w.wormholes.push((x, y));
            }
        }
        w
    }

    pub(crate) fn terrain(&self) -> Terrain<'_> {
        Terrain {
            map: &self.map,
            bricks: &self.bricks,
        }
    }

    // --- Doors -----------------------------------------------------------------------

    /// Applies a door seed: door type `k` (tile `162 + k`) is closed when bit `k` is
    /// set, open otherwise. A door closing on a ship sends that ship back to spawn.
    pub fn apply_door_seed(&mut self, seed: u8) {
        for i in 0..self.doors.len() {
            let (x, y, id) = self.doors[i];
            let closed = seed & (1 << (id - tile::FIRST_DOOR)) != 0;
            let was = self.map.get(x, y);
            self.map.set(x, y, if closed { id } else { tile::FLAG });
            if closed && was == tile::FLAG {
                self.evict_ships_from(x, y);
            }
        }
    }

    /// Sends any ship overlapping tile (x, y) back to spawn.
    fn evict_ships_from(&mut self, x: i32, y: i32) {
        for s in 0..self.ships.len() {
            let ship = &self.ships[s];
            let r = self.settings.ships[ship.kind as usize].radius_pixels();
            let p = ship.pos.pixels();
            let hit = ship.is_alive()
                && p.x + r >= x * 16
                && p.x - r < x * 16 + 16
                && p.y + r >= y * 16
                && p.y - r < y * 16 + 16;
            if hit {
                let mut moved = self.ships[s].clone();
                self.place_at_spawn(&mut moved);
                self.ships[s] = moved;
            }
        }
    }

    /// Moves the clock to `tick` (online: the server's tick count), with the next door
    /// change due straight away.
    pub fn set_tick(&mut self, tick: u32) {
        self.tick = tick;
        self.last_door_tick = tick.wrapping_sub(self.settings.door_delay.max(1));
    }

    /// Follows the server's doors: `door_seed` from the security packet, which was
    /// current at world tick `seed_tick`. Doors catch up from then and change every
    /// `DoorDelay` ticks, the same as every other client.
    pub fn sync_doors(&mut self, door_seed: u32, seed_tick: u32) -> Option<Event> {
        self.door_rng = VieRng::new(door_seed);
        self.last_door_tick = seed_tick;
        self.update_doors()
    }

    /// Applies any door changes due by now.
    fn update_doors(&mut self) -> Option<Event> {
        if self.doors.is_empty() {
            return None;
        }
        let delay = self.settings.door_delay.max(1);
        let elapsed = self.tick.wrapping_sub(self.last_door_tick) as i32;
        if elapsed < delay as i32 {
            return None;
        }
        let count = elapsed as u32 / delay;
        // A timestamp far in the past (bad data) can't stall the game: skip ahead.
        let skip = count.saturating_sub(MAX_DOOR_CATCH_UP);
        self.last_door_tick = self.last_door_tick.wrapping_add(skip * delay);
        let mut seed = 0;
        for _ in skip..count {
            seed = self.next_door_seed();
            self.last_door_tick = self.last_door_tick.wrapping_add(delay);
        }
        self.apply_door_seed(seed);
        Some(Event::DoorsChanged { seed })
    }

    /// Draws the next door seed for the arena's `DoorMode`.
    fn next_door_seed(&mut self) -> u8 {
        match self.settings.door_mode {
            -2 => self.door_rng.next_u32() as u8,
            -1 => {
                // Weighted: each door type stays closed with its own probability
                // (types 0 and 4 most often open, 3 and 7 least). The generator only
                // returns positive values, so these are plain remainders.
                let mut seed = 0u8;
                for (modulus, bits) in [
                    (2, 0x11),
                    (4, 0x02),
                    (8, 0x04),
                    (16, 0x08),
                    (4, 0x20),
                    (8, 0x40),
                    (16, 0x80),
                ] {
                    if !self.door_rng.next_u32().is_multiple_of(modulus) {
                        seed |= bits;
                    }
                }
                seed
            }
            m if m >= 0 => m.min(255) as u8,
            _ => self.door_rng.seed as u8,
        }
    }

    /// Door ids are 162..=169 when closed and [`tile::FLAG`] (170) when open.
    pub fn doors(&self) -> &[(i32, i32, u8)] {
        &self.doors
    }

    // --- Bricks ----------------------------------------------------------------------

    /// Places bricks for team `freq` on empty tiles (as the server tells us to).
    pub fn place_bricks(&mut self, tiles: &[(i32, i32)], freq: u16) {
        let end_tick = self.tick.wrapping_add(self.settings.brick_time);
        for &(x, y) in tiles {
            if self.map.get(x, y) != 0 {
                continue;
            }
            self.map.set(x, y, tile::BRICK);
            self.bricks.insert((x, y), Brick { freq, end_tick });
            self.evict_enemies_from_brick(x, y, freq);
        }
    }

    /// Places a brick the server reported: the straight line of tiles from (x1, y1)
    /// to (x2, y2), placed at server tick `start` (it expires `BrickTime` after that).
    /// Lines are capped at 64 tiles.
    pub fn place_server_brick(&mut self, from: (i32, i32), to: (i32, i32), freq: u16, start: u32) {
        let (dx, dy) = ((to.0 - from.0).signum(), (to.1 - from.1).signum());
        let len = (to.0 - from.0).abs().max((to.1 - from.1).abs()).min(63);
        let end_tick = start.wrapping_add(self.settings.brick_time);
        for k in 0..=len {
            let (x, y) = (from.0 + dx * k, from.1 + dy * k);
            if self.map.get(x, y) != 0 {
                continue;
            }
            self.map.set(x, y, tile::BRICK);
            self.bricks.insert((x, y), Brick { freq, end_tick });
            self.evict_enemies_from_brick(x, y, freq);
        }
    }

    /// Removes every brick (the server's empty brick list).
    pub fn clear_bricks(&mut self) {
        let tiles: Vec<(i32, i32)> = self.bricks.keys().copied().collect();
        for (x, y) in tiles {
            self.map.set(x, y, 0);
        }
        self.bricks.clear();
    }

    fn evict_enemies_from_brick(&mut self, x: i32, y: i32, freq: u16) {
        let enemies: Vec<usize> = (0..self.ships.len())
            .filter(|&i| self.ships[i].freq != freq)
            .collect();
        for s in enemies {
            let ship = &self.ships[s];
            let r = self.settings.ships[ship.kind as usize].radius_pixels();
            let p = ship.pos.pixels();
            if ship.is_alive()
                && p.x + r >= x * 16
                && p.x - r < x * 16 + 16
                && p.y + r >= y * 16
                && p.y - r < y * 16 + 16
            {
                let mut moved = self.ships[s].clone();
                self.place_at_spawn(&mut moved);
                self.ships[s] = moved;
            }
        }
    }

    /// Offline brick placement: a straight wall through tile (x, y), across the
    /// direction the ship faces, up to `BrickSpan`-ish 10 tiles each side, stopping at
    /// walls. (Online the server decides the shape.)
    pub fn drop_brick_line(&mut self, x: i32, y: i32, direction: u8, freq: u16) {
        // Facing mostly up/down → horizontal wall; mostly left/right → vertical wall.
        let d = direction % 20;
        let horizontal = !(5..15).contains(&d);
        let (dx, dy) = if horizontal { (1, 0) } else { (0, 1) };
        let mut tiles = vec![(x, y)];
        for sign in [-1, 1] {
            for k in 1..=10 {
                let (tx, ty) = (x + dx * k * sign, y + dy * k * sign);
                if self.map.get(tx, ty) != 0 {
                    break;
                }
                tiles.push((tx, ty));
            }
        }
        if self.map.get(x, y) == 0 {
            self.place_bricks(&tiles, freq);
        }
    }

    pub fn bricks(&self) -> impl Iterator<Item = (&(i32, i32), &Brick)> {
        self.bricks.iter()
    }

    fn expire_bricks(&mut self) {
        let tick = self.tick;
        let expired: Vec<(i32, i32)> = self
            .bricks
            .iter()
            .filter(|(_, b)| tick_reached(tick, b.end_tick))
            .map(|(&k, _)| k)
            .collect();
        for (x, y) in expired {
            self.bricks.remove(&(x, y));
            if self.map.get(x, y) == tile::BRICK {
                self.map.set(x, y, 0);
            }
        }
    }

    // --- Ships -----------------------------------------------------------------------

    /// Adds a ship of type `kind` (0..=7) at a spawn point with its spawn loadout and
    /// returns its index.
    pub fn add_ship(&mut self, id: u16, kind: u8, freq: u16) -> usize {
        let kind = kind.min(7);
        let mut ship = Ship::new(id, kind, freq, &self.settings.ships[kind as usize]);
        self.place_at_spawn(&mut ship);
        self.ships.push(ship);
        let index = self.ships.len() - 1;
        self.reset_loadout(index);
        index
    }

    pub(crate) fn place_at_spawn(&mut self, ship: &mut Ship) {
        let radius = self.settings.ships[ship.kind as usize].radius_pixels();
        let (cx, cy) = self.spawn_center;
        let mut spot = None;
        for _ in 0..64 {
            let r = self.spawn_radius.max(0) as u32;
            let dx = self.rng.below(2 * r + 1) as i32 - r as i32;
            let dy = self.rng.below(2 * r + 1) as i32 - r as i32;
            let (px, py) = ((cx + dx) * 16 + 8, (cy + dy) * 16 + 8);
            if !self.map.box_collides(px, py, radius)
                && self.map.tile_at_pixel(px, py) != tile::WORMHOLE
            {
                spot = Some((px, py));
                break;
            }
        }
        let (px, py) = spot
            .or_else(|| self.map.find_free_spot(cx, cy, radius, 512))
            .unwrap_or((cx * 16 + 8, cy * 16 + 8));
        ship.pos = Pos::from_pixels(px, py);
        ship.prev_pos = ship.pos;
        ship.vel = Vel::default();
        ship.rotation = self.rng.below(40) * 1000;
        ship.prev_rotation = ship.rotation;
    }

    /// Respawns a ship at a spawn point with its spawn loadout.
    pub fn respawn(&mut self, index: usize) {
        let Some(mut ship) = self.ships.get(index).cloned() else {
            return;
        };
        ship.state = ShipState::Alive;
        self.place_at_spawn(&mut ship);
        self.ships[index] = ship;
        self.reset_loadout(index);
    }

    /// Advances one tick. `inputs[i]` drives `ships[i]`; missing inputs mean no keys.
    pub fn step(&mut self, inputs: &[ShipInput]) -> Vec<Event> {
        self.tick = self.tick.wrapping_add(1);
        let mut events = Vec::new();

        for i in 0..self.ships.len() {
            let input = inputs.get(i).copied().unwrap_or_default();
            self.step_ship(i, input, &mut events);
        }
        self.step_projectiles(&mut events);
        self.step_greens(&mut events);
        self.expire_bricks();
        if let Some(e) = self.update_doors() {
            events.push(e);
        }
        events
    }

    fn step_ship(&mut self, index: usize, input: ShipInput, events: &mut Vec<Event>) {
        let tick = self.tick;
        let ship = &mut self.ships[index];
        ship.prev_pos = ship.pos;
        ship.prev_rotation = ship.rotation;
        ship.toggles &= !toggle::FLASH;

        if let ShipState::Dead { respawn_tick } = ship.state {
            if !ship.remote && tick_reached(tick, respawn_tick) {
                self.respawn(index);
                events.push(Event::Respawned {
                    ship: self.ships[index].id,
                });
            }
            return;
        }

        let s = self.settings.ships[ship.kind as usize];
        self.toggles(index, input, events);
        let ship = &mut self.ships[index];

        // --- Thrust and rotation -----------------------------------------------------
        let rocket = active(tick, ship.rocket_end_tick);
        let shutdown = active(tick, ship.shutdown_end_tick);
        let ab_cost = s.afterburner_energy as i32; // milli-energy per tick
        let wants_ab = input.afterburner && ship.energy > ab_cost && !rocket;
        let thrust = if shutdown {
            0
        } else if rocket {
            self.settings.rocket_thrust
        } else if wants_ab {
            s.maximum_thrust
        } else {
            ship.stats.thrust
        };
        let dir = DIRECTION_HEADINGS[ship.direction() as usize];
        let push = scale_unit(dir, thrust as i64 * VEL_SCALE as i64);
        ship.thrusting = 0;
        if rocket || (!input.backward && input.forward) {
            ship.vel.x = ship.vel.x.saturating_add(push.x as i32);
            ship.vel.y = ship.vel.y.saturating_add(push.y as i32);
            ship.thrusting = if shutdown { 0 } else { 1 };
        } else if input.backward {
            ship.vel.x = ship.vel.x.saturating_sub(push.x as i32);
            ship.vel.y = ship.vel.y.saturating_sub(push.y as i32);
            ship.thrusting = if shutdown { 0 } else { -1 };
        }

        // Engine shutdown leaves only a slow (1/10 turn per second) rotation.
        let rot = if shutdown {
            40
        } else {
            ship.stats.rotation % ROTATION_UNITS
        };
        if input.left {
            ship.rotation = (ship.rotation + ROTATION_UNITS - rot) % ROTATION_UNITS;
        }
        if input.right {
            ship.rotation = (ship.rotation + rot) % ROTATION_UNITS;
        }

        // --- Wormhole gravity ----------------------------------------------------------
        let mut gravity_speed = 0;
        if s.gravity != 0 {
            let p = ship.pos.pixels();
            for &(wx, wy) in &self.wormholes {
                // Continuum measures from the ship's tile to the wormhole's centre tile.
                let (cx, cy) = ((wx + 2) * 16 + 8, (wy + 2) * 16 + 8);
                let (dx, dy) = (
                    (p.x.div_euclid(16) * 16 - cx) as i64,
                    (p.y.div_euclid(16) * 16 - cy) as i64,
                );
                let dist_sq = dx * dx + dy * dy + 1;
                if dist_sq >= (s.gravity.unsigned_abs() as i64) * 1000 {
                    continue;
                }
                let pull = s.gravity as i64 * 1000 / dist_sq;
                let (tx, ty) = ((cx - p.x) as i64, (cy - p.y) as i64);
                let len = isqrt(tx * tx + ty * ty).max(1);
                ship.vel.x = ship
                    .vel
                    .x
                    .saturating_add((tx * pull * VEL_SCALE as i64 / len) as i32);
                ship.vel.y = ship
                    .vel
                    .y
                    .saturating_add((ty * pull * VEL_SCALE as i64 / len) as i32);
                if pull != 0 {
                    gravity_speed = s.gravity_top_speed;
                }
            }
        }

        // --- Top speed ------------------------------------------------------------------
        // Afterburners only count (speed and cost) while actually thrusting.
        let afterburning = wants_ab && ship.thrusting != 0;
        ship.afterburning = afterburning;
        let mut speed = if rocket {
            self.settings.rocket_speed
        } else if afterburning {
            s.maximum_speed
        } else {
            ship.stats.speed
        };
        speed = speed.max(gravity_speed);
        if active(tick, ship.repelled_end_tick) {
            speed = speed.max(self.settings.repel_speed);
        }
        clamp_speed(&mut ship.vel, speed as i64 * VEL_SCALE as i64);

        // --- Energy: afterburner, recharge, then toggle upkeep ------------------------
        if afterburning {
            ship.energy -= ab_cost;
        }
        if !active(tick, ship.emp_end_tick) {
            let max_energy = ship.stats.max_energy as i32 * ENERGY_SCALE;
            ship.energy = (ship.energy + ship.stats.recharge as i32).min(max_energy);
        }
        for (bit, cost) in [
            (toggle::XRADAR, s.xradar_energy),
            (toggle::STEALTH, s.stealth_energy),
            (toggle::CLOAK, s.cloak_energy),
            (toggle::ANTIWARP, s.anti_warp_energy),
        ] {
            if ship.is_on(bit) {
                let cost = cost as i32;
                if ship.energy > cost {
                    ship.energy -= cost;
                } else {
                    ship.toggles &= !bit;
                    events.push(Event::Toggled {
                        ship: ship.id,
                        toggle: bit,
                        on: false,
                    });
                }
            }
        }
        let p = ship.pos.pixels();
        if self.map.tile_at_pixel(p.x, p.y) == tile::SAFE {
            ship.toggles |= toggle::SAFETY;
        } else {
            ship.toggles &= !toggle::SAFETY;
        }
        if let Some((_, end)) = ship.portal
            && tick_reached(tick, end)
        {
            ship.portal = None;
        }

        // --- Weapons ----------------------------------------------------------------
        self.fire_weapons(index, input, events);

        // --- Movement and wall collision --------------------------------------------
        let bounce = self.settings.bounce_factor.max(1) as i64;
        let terrain = Terrain {
            map: &self.map,
            bricks: &self.bricks,
        };
        let ship = &mut self.ships[index];
        let radius = s.radius_pixels();
        let bx = move_axis(&terrain, ship, radius, 0, bounce);
        let by = move_axis(&terrain, ship, radius, 1, bounce);
        if ship.lerp_ticks > 0 {
            ship.lerp_ticks -= 1;
            if bx || by {
                ship.lerp_ticks = 0;
            }
        }
        if bx || by {
            events.push(Event::WallBounce {
                ship: ship.id,
                speed: ship.vel.length() / VEL_SCALE as i64,
            });
        }

        // Wormholes send you back to spawn with 20% energy left (Continuum behaviour).
        let p = ship.pos.pixels();
        if !ship.remote && self.map.tile_at_pixel(p.x, p.y) == tile::WORMHOLE {
            let energy = (ship.energy - ship.energy * 4 / 5).max(ENERGY_SCALE);
            let from = ship.pos;
            let mut moved = ship.clone();
            self.place_at_spawn(&mut moved);
            moved.energy = energy;
            let (id, to) = (moved.id, moved.pos);
            self.ships[index] = moved;
            events.push(Event::Warped { ship: id, from, to });
        }
    }

    fn toggles(&mut self, index: usize, input: ShipInput, events: &mut Vec<Event>) {
        let ship = &mut self.ships[index];
        if input.toggle_multifire && ship.has(cap::MULTIFIRE) {
            ship.multifire = !ship.multifire;
            events.push(Event::MultifireToggled {
                ship: ship.id,
                on: ship.multifire,
            });
        }
        for (pressed, capability, bit) in [
            (input.toggle_stealth, cap::STEALTH, toggle::STEALTH),
            (input.toggle_cloak, cap::CLOAK, toggle::CLOAK),
            (input.toggle_xradar, cap::XRADAR, toggle::XRADAR),
            (input.toggle_antiwarp, cap::ANTIWARP, toggle::ANTIWARP),
        ] {
            if pressed && ship.has(capability) {
                ship.toggles ^= bit;
                let on = ship.is_on(bit);
                if bit == toggle::CLOAK && !on {
                    ship.toggles |= toggle::FLASH;
                }
                events.push(Event::Toggled {
                    ship: ship.id,
                    toggle: bit,
                    on,
                });
            }
        }
    }
}

fn clamp_speed(vel: &mut Vel, max: i64) {
    let len = vel.length();
    if len > max && len > 0 {
        vel.x = (vel.x as i64 * max / len) as i32;
        vel.y = (vel.y as i64 * max / len) as i32;
    }
}

/// Moves a ship along one axis (0 = x, 1 = y); on hitting a wall restores the axis,
/// reverses and damps it, and damps the other axis. Returns whether it bounced.
pub(crate) fn move_axis(
    terrain: &Terrain,
    ship: &mut Ship,
    radius: i32,
    axis: usize,
    bounce: i64,
) -> bool {
    let v = if axis == 0 { ship.vel.x } else { ship.vel.y };
    let lerp = match (ship.lerp_ticks, axis) {
        (0, _) => 0,
        (_, 0) => ship.lerp.x,
        _ => ship.lerp.y,
    };
    let d = v as i64 + lerp as i64;
    if d == 0 {
        return false;
    }
    let previous = ship.pos;
    if axis == 0 {
        ship.pos.x += d;
    } else {
        ship.pos.y += d;
    }
    let p = ship.pos.pixels();
    let (along, across) = if axis == 0 { (p.x, p.y) } else { (p.y, p.x) };
    // Only the leading edge is tested, so a ship already overlapping a wall behind it
    // can still move away.
    let edge = if d > 0 {
        (along + radius).div_euclid(16)
    } else {
        (along - radius).div_euclid(16)
    };
    let (t0, t1) = (
        (across - radius).div_euclid(16),
        (across + radius).div_euclid(16),
    );
    let collided = (t0..=t1).any(|t| {
        if axis == 0 {
            terrain.is_solid(edge, t, ship.freq)
        } else {
            terrain.is_solid(t, edge, ship.freq)
        }
    });
    if !collided {
        return false;
    }
    ship.pos = previous;
    let damp = |c: i32| (c as i64 * 16 / bounce) as i32;
    if axis == 0 {
        ship.vel.x = -damp(ship.vel.x);
        ship.vel.y = damp(ship.vel.y);
    } else {
        ship.vel.y = -damp(ship.vel.y);
        ship.vel.x = damp(ship.vel.x);
    }
    true
}
