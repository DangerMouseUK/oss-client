//! Ships, projectiles and the fixed-step world that advances them.
//!
//! Behaviour follows the Continuum client as documented by the nullspace client (MIT):
//! thrust/rotation/speed/afterburner/energy order, per-axis wall collision with
//! `16 / BounceFactor` damping, 40-direction thrust and firing, point-collision weapons.

use crate::fixed::{
    DIRECTION_HEADINGS, ENERGY_SCALE, POS_SCALE, Pos, ROTATION_UNITS, Rng, VEL_SCALE, Vec2, Vel,
    heading, isqrt, scale_unit,
};
use crate::map::{TileMap, tile};
use crate::settings::{ArenaSettings, ShipSettings};

/// One tick of player intent.
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
    /// Edge-triggered: true only on the tick the multifire key went down.
    pub toggle_multifire: bool,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            rotation: s.initial_rotation,
            thrust: s.initial_thrust,
            speed: s.initial_speed,
            recharge: s.initial_recharge,
            max_energy: s.initial_energy,
        }
    }
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
    pub guns: u8,
    pub bombs: u8,
    pub multifire: bool,
    /// Whether this world applies damage to this ship (true for the local ship; remote
    /// ships report their own damage over the network).
    pub takes_damage: bool,
    /// Afterburners were burning energy on the last tick.
    pub afterburning: bool,
    /// -1 reverse, 0 none, 1 forward thrust on the last tick (for exhaust effects).
    pub thrusting: i8,
    pub next_gun_tick: u32,
    pub next_bomb_tick: u32,
    /// Position/rotation before the last tick, for render interpolation.
    pub prev_pos: Pos,
    pub prev_rotation: u32,
}

impl Ship {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectileKind {
    Bullet { bouncing: bool },
    Bomb { bounces_left: u8 },
    Mine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projectile {
    pub id: u32,
    pub owner: u16,
    pub freq: u16,
    pub kind: ProjectileKind,
    /// Weapon level 0..=3.
    pub level: u8,
    pub pos: Pos,
    pub vel: Vel,
    pub end_tick: u32,
    pub prev_pos: Pos,
}

/// Things that happened during a tick, for sound, effects and networking.
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
    WallBounce {
        ship: u16,
        speed: i64,
    },
    /// A projectile ended in an explosion at `pos` (bullets fizzle, bombs/mines explode).
    Explosion {
        pos: Pos,
        kind: ProjectileKind,
        level: u8,
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
}

/// Half-size in pixels of a weapon's hit box against ships.
const WEAPON_HIT_RADIUS: i32 = 4;

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
    rng: Rng,
    next_projectile_id: u32,
}

impl World {
    pub fn new(map: TileMap, settings: ArenaSettings, seed: u32) -> Self {
        Self {
            tick: 0,
            map,
            settings,
            ships: Vec::new(),
            projectiles: Vec::new(),
            spawn_center: (512, 512),
            spawn_radius: 20,
            rng: Rng::new(seed),
            next_projectile_id: 1,
        }
    }

    /// Adds a ship of type `kind` (0..=7) at a spawn point and returns its index.
    pub fn add_ship(&mut self, id: u16, kind: u8, freq: u16) -> usize {
        let kind = kind.min(7);
        let s = &self.settings.ships[kind as usize];
        let stats = ShipStats::initial(s);
        let mut ship = Ship {
            id,
            kind,
            freq,
            state: ShipState::Alive,
            pos: Pos::default(),
            vel: Vel::default(),
            rotation: 0,
            energy: stats.max_energy as i32 * ENERGY_SCALE,
            stats,
            guns: s.initial_guns,
            bombs: s.initial_bombs,
            multifire: false,
            takes_damage: true,
            afterburning: false,
            thrusting: 0,
            next_gun_tick: 0,
            next_bomb_tick: 0,
            prev_pos: Pos::default(),
            prev_rotation: 0,
        };
        self.place_at_spawn(&mut ship);
        self.ships.push(ship);
        self.ships.len() - 1
    }

    fn place_at_spawn(&mut self, ship: &mut Ship) {
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

    /// Respawns a ship at a spawn point with full energy and initial stats.
    pub fn respawn(&mut self, index: usize) {
        let Some(mut ship) = self.ships.get(index).cloned() else {
            return;
        };
        let s = self.settings.ships[ship.kind as usize];
        ship.stats = ShipStats::initial(&s);
        ship.energy = ship.stats.max_energy as i32 * ENERGY_SCALE;
        ship.guns = s.initial_guns;
        ship.bombs = s.initial_bombs;
        ship.state = ShipState::Alive;
        self.place_at_spawn(&mut ship);
        self.ships[index] = ship;
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
        events
    }

    fn step_ship(&mut self, index: usize, input: ShipInput, events: &mut Vec<Event>) {
        let tick = self.tick;
        let ship = &mut self.ships[index];
        ship.prev_pos = ship.pos;
        ship.prev_rotation = ship.rotation;

        if let ShipState::Dead { respawn_tick } = ship.state {
            if tick_reached(tick, respawn_tick) {
                self.respawn(index);
                events.push(Event::Respawned {
                    ship: self.ships[index].id,
                });
            }
            return;
        }

        let settings = self.settings.ships[ship.kind as usize];

        if input.toggle_multifire && settings.multifire {
            ship.multifire = !ship.multifire;
            events.push(Event::MultifireToggled {
                ship: ship.id,
                on: ship.multifire,
            });
        }

        // --- Thrust and rotation -----------------------------------------------------
        let ab_cost = settings.afterburner_energy as i32; // milli-energy per tick
        let wants_ab = input.afterburner && ship.energy > ab_cost;
        let thrust = if wants_ab {
            settings.maximum_thrust
        } else {
            ship.stats.thrust
        };
        let dir = DIRECTION_HEADINGS[ship.direction() as usize];
        let push = scale_unit(dir, thrust as i64 * VEL_SCALE as i64);
        ship.thrusting = 0;
        if input.backward {
            ship.vel.x = ship.vel.x.saturating_sub(push.x as i32);
            ship.vel.y = ship.vel.y.saturating_sub(push.y as i32);
            ship.thrusting = -1;
        } else if input.forward {
            ship.vel.x = ship.vel.x.saturating_add(push.x as i32);
            ship.vel.y = ship.vel.y.saturating_add(push.y as i32);
            ship.thrusting = 1;
        }

        let rot = ship.stats.rotation % ROTATION_UNITS;
        if input.left {
            ship.rotation = (ship.rotation + ROTATION_UNITS - rot) % ROTATION_UNITS;
        }
        if input.right {
            ship.rotation = (ship.rotation + rot) % ROTATION_UNITS;
        }

        // Afterburners only count (speed and cost) while actually thrusting.
        let afterburning = wants_ab && ship.thrusting != 0;
        ship.afterburning = afterburning;
        let speed = if afterburning {
            settings.maximum_speed
        } else {
            ship.stats.speed
        };
        clamp_speed(&mut ship.vel, speed as i64 * VEL_SCALE as i64);

        // --- Energy: afterburner cost, then recharge ---------------------------------
        if afterburning {
            ship.energy -= ab_cost;
        }
        let max_energy = ship.stats.max_energy as i32 * ENERGY_SCALE;
        ship.energy = (ship.energy + ship.stats.recharge as i32).min(max_energy);

        // --- Weapons ----------------------------------------------------------------
        self.fire_weapons(index, input, events);

        // --- Movement and wall collision --------------------------------------------
        let bounce = self.settings.bounce_factor.max(1) as i64;
        let ship = &mut self.ships[index];
        let radius = settings.radius_pixels();
        let bx = move_axis(&self.map, ship, radius, 0, bounce);
        let by = move_axis(&self.map, ship, radius, 1, bounce);
        if bx || by {
            events.push(Event::WallBounce {
                ship: ship.id,
                speed: ship.vel.length() / VEL_SCALE as i64,
            });
        }

        // Wormholes send you back to spawn with 20% energy left (Continuum behaviour).
        let p = ship.pos.pixels();
        if self.map.tile_at_pixel(p.x, p.y) == tile::WORMHOLE {
            let energy = (ship.energy - ship.energy * 4 / 5).max(ENERGY_SCALE);
            let mut s = ship.clone();
            self.place_at_spawn(&mut s);
            s.energy = energy;
            self.ships[index] = s;
        }
    }

    fn fire_weapons(&mut self, index: usize, input: ShipInput, events: &mut Vec<Event>) {
        let tick = self.tick;
        let arena = &self.settings;
        let ship = &self.ships[index];
        let s = arena.ships[ship.kind as usize];
        let p = ship.pos.pixels();
        let in_safe = self.map.tile_at_pixel(p.x, p.y) == tile::SAFE;

        enum Shot {
            Gun {
                level: u8,
                multi: bool,
                cost: i32,
                delay: u32,
            },
            Bomb {
                level: u8,
                cost: i32,
                mine: bool,
            },
        }
        let mut shot = None;

        if input.gun && tick_after(tick, ship.next_gun_tick) && ship.guns > 0 {
            let level = ship.guns - 1;
            let multi = ship.multifire && s.multifire;
            let (delay, per) = if multi {
                (s.multi_fire_delay, s.multi_fire_energy)
            } else {
                (s.bullet_fire_delay, s.bullet_fire_energy)
            };
            let cost = (per * (level as u32 + 1)) as i32 * ENERGY_SCALE;
            if cost < ship.energy {
                shot = Some(Shot::Gun {
                    level,
                    multi,
                    cost,
                    delay,
                });
            }
        }
        if input.mine && tick_after(tick, ship.next_bomb_tick) && ship.bombs > 0 {
            let level = ship.bombs - 1;
            let cost = (s.landmine_fire_energy + s.landmine_fire_energy_upgrade * level as u32)
                as i32
                * ENERGY_SCALE;
            let own_mines = self
                .projectiles
                .iter()
                .filter(|pr| pr.owner == ship.id && pr.kind == ProjectileKind::Mine)
                .count();
            if cost < ship.energy && own_mines < s.max_mines as usize {
                shot = Some(Shot::Bomb {
                    level,
                    cost,
                    mine: true,
                });
            }
        } else if input.bomb && tick_after(tick, ship.next_bomb_tick) && ship.bombs > 0 {
            let level = ship.bombs - 1;
            let cost = (s.bomb_fire_energy + s.bomb_fire_energy_upgrade * level as u32) as i32
                * ENERGY_SCALE;
            if cost < ship.energy {
                shot = Some(Shot::Bomb {
                    level,
                    cost,
                    mine: false,
                });
            }
        }

        let Some(shot) = shot else { return };
        if in_safe {
            // Continuum stops you dead instead of firing inside a safe zone.
            self.ships[index].vel = Vel::default();
            return;
        }

        // Projectiles inherit the position/velocity the network would see (whole pixels,
        // whole px/10s), so local and remote copies of a shot match.
        let origin = Pos::from_pixels(p.x, p.y);
        let base_vel = {
            let w = ship.vel.to_wire();
            Vel::from_wire(w.x, w.y)
        };
        let direction = ship.direction();
        let (owner, freq) = (ship.id, ship.freq);
        let radius = s.radius_pixels();

        match shot {
            Shot::Gun {
                level,
                multi,
                cost,
                delay,
            } => {
                let kind = ProjectileKind::Bullet {
                    bouncing: s.bouncing_bullets,
                };
                let end = tick.wrapping_add(arena.bullet_alive_time);
                let dir_angle = direction as i64 * 1000;
                let fwd = DIRECTION_HEADINGS[direction as usize];
                let mut spawn = |pos: Pos, unit: Vec2<i32>| {
                    let v = scale_unit(unit, s.bullet_speed as i64 * VEL_SCALE as i64);
                    let vel = Vel::new(base_vel.x + v.x as i32, base_vel.y + v.y as i32);
                    self.spawn_projectile(owner, freq, kind, level, pos, vel, end);
                };
                if s.double_barrel {
                    // Barrels sit 3/4 of the radius either side of the nose.
                    let off =
                        scale_unit(Vec2::new(-fwd.y, fwd.x), radius as i64 * 3 / 4 * POS_SCALE);
                    spawn(Pos::new(origin.x - off.x, origin.y - off.y), fwd);
                    spawn(Pos::new(origin.x + off.x, origin.y + off.y), fwd);
                } else {
                    spawn(origin, fwd);
                }
                if multi {
                    // MultiFireAngle is in 1/111 degree; convert to 1/40000 turn.
                    let spread = s.multi_fire_angle as i64 * 40_000 / (111 * 360);
                    spawn(origin, heading(dir_angle + spread));
                    spawn(origin, heading(dir_angle - spread));
                }
                let ship = &mut self.ships[index];
                ship.energy -= cost;
                ship.next_gun_tick = tick.wrapping_add(delay);
                ship.next_bomb_tick = later(ship.next_bomb_tick, ship.next_gun_tick);
                events.push(Event::FiredGun {
                    ship: owner,
                    level,
                    multifire: multi,
                });
            }
            Shot::Bomb { level, cost, mine } => {
                if mine {
                    let end = tick.wrapping_add(arena.mine_alive_time);
                    self.spawn_projectile(
                        owner,
                        freq,
                        ProjectileKind::Mine,
                        level,
                        origin,
                        Vel::default(),
                        end,
                    );
                    let ship = &mut self.ships[index];
                    ship.next_bomb_tick = tick.wrapping_add(s.landmine_fire_delay);
                    events.push(Event::LaidMine { ship: owner, level });
                } else {
                    let fwd = DIRECTION_HEADINGS[direction as usize];
                    // Recoil is applied before the bomb inherits the ship's velocity.
                    let recoil = scale_unit(fwd, s.bomb_thrust as i64 * VEL_SCALE as i64);
                    let ship_vel =
                        Vel::new(base_vel.x - recoil.x as i32, base_vel.y - recoil.y as i32);
                    let v = scale_unit(fwd, s.bomb_speed as i64 * VEL_SCALE as i64);
                    let vel = Vel::new(ship_vel.x + v.x as i32, ship_vel.y + v.y as i32);
                    let end = tick.wrapping_add(arena.bomb_alive_time);
                    let kind = ProjectileKind::Bomb {
                        bounces_left: s.bomb_bounce_count,
                    };
                    self.spawn_projectile(owner, freq, kind, level, origin, vel, end);
                    let ship = &mut self.ships[index];
                    ship.vel.x -= recoil.x as i32;
                    ship.vel.y -= recoil.y as i32;
                    ship.next_bomb_tick = tick.wrapping_add(s.bomb_fire_delay);
                    events.push(Event::FiredBomb { ship: owner, level });
                }
                let ship = &mut self.ships[index];
                ship.energy -= cost;
                ship.next_gun_tick = later(ship.next_gun_tick, ship.next_bomb_tick);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_projectile(
        &mut self,
        owner: u16,
        freq: u16,
        kind: ProjectileKind,
        level: u8,
        pos: Pos,
        vel: Vel,
        end_tick: u32,
    ) {
        let id = self.next_projectile_id;
        self.next_projectile_id = self.next_projectile_id.wrapping_add(1);
        self.projectiles.push(Projectile {
            id,
            owner,
            freq,
            kind,
            level,
            pos,
            vel,
            end_tick,
            prev_pos: pos,
        });
    }

    fn step_projectiles(&mut self, events: &mut Vec<Event>) {
        let tick = self.tick;
        let mut i = 0;
        while i < self.projectiles.len() {
            let pr = &mut self.projectiles[i];
            pr.prev_pos = pr.pos;
            if tick_reached(tick, pr.end_tick) {
                // Expired projectiles vanish without exploding.
                self.projectiles.swap_remove(i);
                continue;
            }

            let before = pr.pos;
            let mut remove = false;
            if pr.kind != ProjectileKind::Mine {
                let cx = projectile_axis(&self.map, pr, 0);
                let cy = projectile_axis(&self.map, pr, 1);
                if cx || cy {
                    match &mut pr.kind {
                        ProjectileKind::Bullet { bouncing: false } => remove = true,
                        ProjectileKind::Bomb { bounces_left } => {
                            if *bounces_left == 0 {
                                remove = true;
                            } else {
                                *bounces_left -= 1;
                            }
                        }
                        _ => {}
                    }
                }
                let p = pr.pos.pixels();
                if self.map.tile_at_pixel(p.x, p.y) == tile::WORMHOLE {
                    self.projectiles.swap_remove(i);
                    continue;
                }
            }

            if remove {
                let pr = self.projectiles.swap_remove(i);
                self.explode(&pr, pr.pos, None, events);
                continue;
            }

            // Ship hits.
            let pr = &self.projectiles[i];
            let wp = pr.pos.pixels();
            let mut hit = None;
            for ship in &self.ships {
                if !ship.is_alive() || ship.freq == pr.freq {
                    continue;
                }
                let r = self.settings.ships[ship.kind as usize].radius_pixels();
                let sp = ship.pos.pixels();
                if boxes_overlap(sp.x, sp.y, r, wp.x, wp.y, WEAPON_HIT_RADIUS) {
                    hit = Some(ship.id);
                    break;
                }
            }
            if let Some(target) = hit {
                let pr = self.projectiles.swap_remove(i);
                // Bombs explode where they were before entering the ship.
                let at = if matches!(pr.kind, ProjectileKind::Bullet { .. }) {
                    pr.pos
                } else {
                    before
                };
                self.explode(&pr, at, Some(target), events);
                continue;
            }
            i += 1;
        }
    }

    /// Ends a projectile at `at`, applying damage. `direct` is the ship a bullet hit.
    fn explode(&mut self, pr: &Projectile, at: Pos, direct: Option<u16>, events: &mut Vec<Event>) {
        events.push(Event::Explosion {
            pos: at,
            kind: pr.kind,
            level: pr.level,
        });
        let arena = &self.settings;
        match pr.kind {
            ProjectileKind::Bullet { .. } => {
                if let Some(target) = direct {
                    let damage =
                        arena.bullet_damage_level + arena.bullet_damage_upgrade * pr.level as u32;
                    self.damage(target, pr.owner, damage, events);
                }
            }
            ProjectileKind::Bomb { .. } | ProjectileKind::Mine => {
                let explode = (arena.bomb_explode_pixels * (1 + pr.level as u32)) as i64;
                let bomb_dmg = arena.bomb_damage_level as i64;
                if explode == 0 {
                    return;
                }
                let at_px = at.pixels();
                let dist_to = |p: Pos| {
                    let p = p.pixels();
                    let (dx, dy) = ((p.x - at_px.x) as i64, (p.y - at_px.y) as i64);
                    isqrt(dx * dx + dy * dy)
                };
                let shooter_dist = self
                    .ships
                    .iter()
                    .find(|s| s.id == pr.owner)
                    .map(|s| dist_to(s.pos));
                let mut hits = Vec::new();
                for ship in &self.ships {
                    if !ship.is_alive() || (ship.freq == pr.freq && ship.id != pr.owner) {
                        continue;
                    }
                    let d = dist_to(ship.pos);
                    if d >= explode {
                        continue;
                    }
                    let mut dmg = (explode - d) * bomb_dmg / explode;
                    if ship.id != pr.owner
                        && let Some(sd) = shooter_dist.filter(|&sd| sd < explode)
                    {
                        // Being close to your own bomb softens it for the target.
                        dmg -= (explode - sd) * bomb_dmg / explode / 2;
                    }
                    if dmg > 0 {
                        hits.push((ship.id, dmg as u32));
                    }
                }
                for (id, dmg) in hits {
                    self.damage(id, pr.owner, dmg, events);
                }
            }
        }
    }

    fn damage(&mut self, target: u16, shooter: u16, damage: u32, events: &mut Vec<Event>) {
        let tick = self.tick;
        let enter_delay = self.settings.enter_delay;
        let Some(ship) = self.ships.iter_mut().find(|s| s.id == target) else {
            return;
        };
        if !ship.takes_damage || !ship.is_alive() {
            return;
        }
        let p = ship.pos.pixels();
        if self.map.tile_at_pixel(p.x, p.y) == tile::SAFE {
            return;
        }
        events.push(Event::ShipHit {
            ship: target,
            shooter,
            damage,
        });
        let dmg = damage as i32 * ENERGY_SCALE;
        if ship.energy <= dmg {
            if target == shooter {
                // Your own bombs leave you on 1 energy rather than killing you.
                ship.energy = ENERGY_SCALE;
                return;
            }
            ship.energy = 0;
            ship.vel = Vel::default();
            ship.state = ShipState::Dead {
                respawn_tick: tick.wrapping_add(enter_delay),
            };
            events.push(Event::ShipKilled {
                ship: target,
                killer: shooter,
                pos: ship.pos,
            });
        } else {
            ship.energy -= dmg;
        }
    }
}

/// `a` is at or after `b` (wrapping tick compare).
const fn tick_reached(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

/// `a` is strictly after `b`.
const fn tick_after(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

/// The later of two ticks.
const fn later(a: u32, b: u32) -> u32 {
    if tick_after(b, a) { b } else { a }
}

fn clamp_speed(vel: &mut Vel, max: i64) {
    let len = vel.length();
    if len > max && len > 0 {
        vel.x = (vel.x as i64 * max / len) as i32;
        vel.y = (vel.y as i64 * max / len) as i32;
    }
}

/// Inclusive-max box overlap test, matching Continuum's ship/weapon checks.
const fn boxes_overlap(ax: i32, ay: i32, ar: i32, bx: i32, by: i32, br: i32) -> bool {
    ax + ar >= bx - br && ax - ar < bx + br && ay + ar >= by - br && ay - ar < by + br
}

/// Moves a ship along one axis (0 = x, 1 = y); on hitting a wall restores the axis,
/// reverses and damps it, and damps the other axis. Returns whether it bounced.
fn move_axis(map: &TileMap, ship: &mut Ship, radius: i32, axis: usize, bounce: i64) -> bool {
    let v = if axis == 0 { ship.vel.x } else { ship.vel.y };
    if v == 0 {
        return false;
    }
    let previous = ship.pos;
    if axis == 0 {
        ship.pos.x += v as i64;
    } else {
        ship.pos.y += v as i64;
    }
    let p = ship.pos.pixels();
    let (along, across) = if axis == 0 { (p.x, p.y) } else { (p.y, p.x) };
    // Only the leading edge is tested, so a ship already overlapping a wall behind it
    // can still move away.
    let edge = if v > 0 {
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
            map.is_solid(edge, t)
        } else {
            map.is_solid(t, edge)
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

/// Moves a projectile along one axis; on entering a solid tile restores and reflects
/// that axis. Returns whether it hit a wall.
fn projectile_axis(map: &TileMap, pr: &mut Projectile, axis: usize) -> bool {
    let previous = pr.pos;
    if axis == 0 {
        pr.pos.x += pr.vel.x as i64;
    } else {
        pr.pos.y += pr.vel.y as i64;
    }
    let p = pr.pos.pixels();
    if map.is_solid(p.x.div_euclid(16), p.y.div_euclid(16)) {
        pr.pos = previous;
        if axis == 0 {
            pr.vel.x = -pr.vel.x;
        } else {
            pr.vel.y = -pr.vel.y;
        }
        return true;
    }
    false
}
