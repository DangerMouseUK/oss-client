//! Weapons and items: firing, projectile flight, proximity triggers, explosions,
//! shrapnel, repel and damage.
//!
//! Behaviour follows the Continuum client as documented by the nullspace client (MIT).

use crate::fixed::{
    DIRECTION_HEADINGS, ENERGY_SCALE, POS_SCALE, Pos, VEL_SCALE, Vec2, Vel, VieRng, heading, isqrt,
    scale_unit,
};
use crate::map::tile;
use crate::ship::{ShipInput, ShipState, active, cap, later, tick_after, tick_reached, toggle};
use crate::world::{Event, Terrain, World};

/// Half-size in pixels of a weapon's hit box against ships.
const WEAPON_HIT_RADIUS: i32 = 4;
/// Ticks after a repel/burst/thor/decoy/warp before another can be used.
const REPEL_DELAY: u32 = 50;
/// Shrapnel younger than this many ticks is "inactive": weak, and vanishes on walls.
const INACTIVE_SHRAP_TICKS: u32 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectileKind {
    Bullet,
    Shrapnel,
    Bomb,
    Mine,
    Thor,
    /// Burst bullets: harmless until their first wall bounce.
    Burst,
    /// Decoy: a fake ship that drifts with its owner's launch velocity.
    Decoy,
}

/// A proximity bomb that has sensed a target and is waiting to go off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxTrigger {
    pub target: u16,
    /// Largest axis distance to the target seen so far (pixels).
    pub closest: i32,
    pub sensor_end_tick: u32,
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
    pub spawn_tick: u32,
    pub end_tick: u32,
    pub prev_pos: Pos,
    /// Bullets/shrapnel that bounce off walls forever.
    pub bouncing: bool,
    /// Wall bounces left before a bomb explodes on impact.
    pub bounces_left: u8,
    pub emp: bool,
    pub proximity: bool,
    /// Shrapnel released when this bomb explodes.
    pub shrap: u8,
    pub shrap_level: u8,
    pub shrap_bouncing: bool,
    /// Burst: armed (has hit a wall).
    pub armed: bool,
    pub prox: Option<ProxTrigger>,
    /// Seed for this bomb's shrapnel directions.
    pub rng_seed: u32,
    /// Decoy: the owner's rotation at launch.
    pub launch_rotation: u32,
}

impl Projectile {
    pub fn is_bomb_like(&self) -> bool {
        matches!(
            self.kind,
            ProjectileKind::Bomb | ProjectileKind::Mine | ProjectileKind::Thor
        )
    }
}

/// What a projectile did this tick.
enum Outcome {
    Continue,
    /// Disappear silently (expired, wormhole, inactive shrapnel on a wall).
    Vanish,
    /// Explode at `at`; `target` is a ship hit directly.
    Explode {
        at: Pos,
        target: Option<u16>,
    },
}

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
    Repel,
    Burst,
    Thor,
    Decoy,
}

impl World {
    /// Whether ship `index` is blocked from warping by an enemy's antiwarp (or the
    /// settle delay after its own warp).
    pub fn is_antiwarped(&self, index: usize) -> bool {
        let Some(me) = self.ships.get(index) else {
            return false;
        };
        if active(self.tick, me.fake_antiwarp_end_tick) {
            return true;
        }
        let range = self.settings.anti_warp_pixels as i64;
        let p = me.pos.pixels();
        self.ships.iter().any(|s| {
            s.is_alive() && s.freq != me.freq && s.is_on(toggle::ANTIWARP) && {
                let q = s.pos.pixels();
                let (dx, dy) = ((q.x - p.x) as i64, (q.y - p.y) as i64);
                dx * dx + dy * dy <= range * range
            }
        })
    }

    pub(crate) fn fire_weapons(&mut self, index: usize, input: ShipInput, events: &mut Vec<Event>) {
        let tick = self.tick;
        let ship = &self.ships[index];
        let s = self.settings.ships[ship.kind as usize];
        let p = ship.pos.pixels();
        let in_safe = self.map.tile_at_pixel(p.x, p.y) == tile::SAFE;
        let can_fastshoot = !ship.afterburning || !s.disable_fast_shooting;
        let bomb_ready = tick_after(tick, ship.next_bomb_tick);
        let (id, freq) = (ship.id, ship.freq);

        // --- Items that don't fire a normal weapon ---------------------------------
        if input.rocket
            && bomb_ready
            && !active(tick, ship.rocket_end_tick)
            && ship.items.rockets > 0
        {
            let ship = &mut self.ships[index];
            ship.items.rockets -= 1;
            ship.rocket_end_tick = tick.wrapping_add(s.rocket_time);
            events.push(Event::RocketStarted { ship: id });
        }
        if input.brick && bomb_ready && !in_safe && self.ships[index].items.bricks > 0 {
            let ship = &mut self.ships[index];
            ship.items.bricks -= 1;
            ship.next_bomb_tick = later(ship.next_bomb_tick, tick.wrapping_add(s.bomb_fire_delay));
            ship.next_gun_tick = later(ship.next_gun_tick, tick.wrapping_add(s.bomb_fire_delay));
            let (x, y, dir) = (p.x.div_euclid(16), p.y.div_euclid(16), ship.direction());
            events.push(Event::BrickDropped {
                ship: id,
                x,
                y,
                direction: dir,
            });
            if self.local_bricks {
                self.drop_brick_line(x, y, dir, freq);
            }
        }
        self.portal_and_warp(index, input, events);

        // --- Pick at most one weapon this tick --------------------------------------
        let ship = &self.ships[index];
        let mut shot = None;
        if input.repel && tick_after(tick, ship.next_repel_tick) && ship.items.repels > 0 {
            shot = Some(Shot::Repel);
        } else if input.burst && bomb_ready && ship.items.bursts > 0 {
            shot = Some(Shot::Burst);
        } else if input.thor && bomb_ready && ship.items.thors > 0 && can_fastshoot {
            shot = Some(Shot::Thor);
        } else if input.decoy && bomb_ready && ship.items.decoys > 0 {
            shot = Some(Shot::Decoy);
        } else if input.gun
            && tick_after(tick, ship.next_gun_tick)
            && ship.guns > 0
            && can_fastshoot
        {
            let level = ship.guns - 1;
            let multi = ship.multifire && ship.has(cap::MULTIFIRE);
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
        } else if input.mine && bomb_ready && ship.bombs > 0 {
            let level = ship.bombs - 1;
            let cost = (s.landmine_fire_energy + s.landmine_fire_energy_upgrade * level as u32)
                as i32
                * ENERGY_SCALE;
            let own = self
                .projectiles
                .iter()
                .filter(|pr| pr.owner == id && pr.kind == ProjectileKind::Mine)
                .count();
            let team = self
                .projectiles
                .iter()
                .filter(|pr| pr.freq == freq && pr.kind == ProjectileKind::Mine)
                .count();
            let stacked = self.projectiles.iter().any(|pr| {
                pr.kind == ProjectileKind::Mine && pr.freq == freq && pr.pos.pixels() == p
            });
            if cost < ship.energy
                && own < s.max_mines as usize
                && team < self.settings.team_max_mines as usize
                && !stacked
            {
                shot = Some(Shot::Bomb {
                    level,
                    cost,
                    mine: true,
                });
            }
        } else if input.bomb && bomb_ready && ship.bombs > 0 && can_fastshoot {
            let level = ship.bombs - 1;
            let cost = (s.bomb_fire_energy + s.bomb_fire_energy_upgrade * level as u32) as i32
                * ENERGY_SCALE;
            let mut ok = cost < ship.energy;
            if ok && ship.has(cap::PROXIMITY) && self.settings.bomb_safety {
                // Proximity bombs can't be fired with an enemy already in range.
                let prox = (self.settings.proximity_distance + level as u32) as i64 * 16;
                ok = !self.ships.iter().any(|o| {
                    o.is_alive() && o.freq != freq && {
                        let q = o.pos.pixels();
                        let (dx, dy) = ((q.x - p.x) as i64, (q.y - p.y) as i64);
                        dx * dx + dy * dy <= prox * prox
                    }
                });
            }
            if ok {
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

        // Projectiles start from what the network would see (whole pixels, whole
        // px/10s), so local and remote copies of a shot match.
        let ship = &self.ships[index];
        let origin = Pos::from_pixels(p.x, p.y);
        let base_vel = {
            let w = ship.vel.to_wire();
            Vel::from_wire(w.x, w.y)
        };
        let direction = ship.direction();
        let fwd = DIRECTION_HEADINGS[direction as usize];
        let rotation = ship.rotation;
        let free = active(tick, ship.super_end_tick);
        let shrap = if ship.guns > 0 {
            (
                ship.shrapnel,
                ship.guns - 1,
                ship.has(cap::BOUNCING_BULLETS),
            )
        } else {
            (0, 0, false)
        };
        let bouncing_bullets = ship.has(cap::BOUNCING_BULLETS);
        let proximity = ship.has(cap::PROXIMITY);
        let radius = s.radius_pixels();
        let mut energy_cost = 0;

        match shot {
            Shot::Repel => {
                let ship = &mut self.ships[index];
                ship.items.repels -= 1;
                ship.next_repel_tick = tick.wrapping_add(REPEL_DELAY);
                self.item_delay(index, s.bomb_fire_delay);
                self.repel(index, events);
                events.push(Event::Repelled {
                    ship: id,
                    pos: origin,
                });
            }
            Shot::Burst => {
                self.ships[index].items.bursts -= 1;
                self.item_delay(index, s.bomb_fire_delay);
                let count = s.burst_shrapnel.max(1) as i64;
                let end = tick.wrapping_add(self.settings.bullet_alive_time);
                for i in 0..count {
                    let dir = heading(i * 40_000 / count);
                    let v = scale_unit(dir, s.burst_speed as i64 * VEL_SCALE as i64);
                    let mut pr = self.new_projectile(
                        id,
                        freq,
                        ProjectileKind::Burst,
                        0,
                        origin,
                        vel(v),
                        end,
                    );
                    pr.bouncing = true;
                    self.projectiles.push(pr);
                }
                events.push(Event::BurstFired { ship: id });
            }
            Shot::Thor => {
                self.ships[index].items.thors -= 1;
                self.item_delay(index, s.bomb_fire_delay);
                let v = scale_unit(fwd, s.bomb_speed as i64 * VEL_SCALE as i64);
                let end = tick.wrapping_add(self.settings.bomb_alive_time);
                let v = Vel::new(base_vel.x + v.x as i32, base_vel.y + v.y as i32);
                let mut pr = self.new_projectile(id, freq, ProjectileKind::Thor, 0, origin, v, end);
                pr.proximity = true;
                (pr.shrap, pr.shrap_level, pr.shrap_bouncing) = shrap;
                pr.rng_seed = shrap_seed(&pr);
                self.projectiles.push(pr);
                events.push(Event::ThorFired { ship: id });
            }
            Shot::Decoy => {
                self.ships[index].items.decoys -= 1;
                self.item_delay(index, s.bomb_fire_delay);
                let end = tick.wrapping_add(self.settings.decoy_alive_time);
                let mut pr =
                    self.new_projectile(id, freq, ProjectileKind::Decoy, 0, origin, base_vel, end);
                pr.launch_rotation = rotation;
                pr.bouncing = true;
                self.projectiles.push(pr);
                events.push(Event::DecoyLaunched { ship: id });
            }
            Shot::Gun {
                level,
                multi,
                cost,
                delay,
            } => {
                energy_cost = cost;
                let end = tick.wrapping_add(self.settings.bullet_alive_time);
                let spawn = |w: &mut World, pos: Pos, unit: Vec2<i32>| {
                    let v = scale_unit(unit, s.bullet_speed as i64 * VEL_SCALE as i64);
                    let v = Vel::new(base_vel.x + v.x as i32, base_vel.y + v.y as i32);
                    let mut pr =
                        w.new_projectile(id, freq, ProjectileKind::Bullet, level, pos, v, end);
                    pr.bouncing = bouncing_bullets;
                    w.projectiles.push(pr);
                };
                if s.double_barrel {
                    // Barrels sit 3/4 of the radius either side of the nose.
                    let off =
                        scale_unit(Vec2::new(-fwd.y, fwd.x), radius as i64 * 3 / 4 * POS_SCALE);
                    spawn(self, Pos::new(origin.x - off.x, origin.y - off.y), fwd);
                    spawn(self, Pos::new(origin.x + off.x, origin.y + off.y), fwd);
                } else {
                    spawn(self, origin, fwd);
                }
                if multi {
                    // MultiFireAngle is in 1/111 degree; convert to 1/40000 turn.
                    let spread = s.multi_fire_angle as i64 * 40_000 / (111 * 360);
                    let base = direction as i64 * 1000;
                    spawn(self, origin, heading(base + spread));
                    spawn(self, origin, heading(base - spread));
                }
                let ship = &mut self.ships[index];
                ship.next_gun_tick = tick.wrapping_add(delay);
                ship.next_bomb_tick = later(ship.next_bomb_tick, ship.next_gun_tick);
                events.push(Event::FiredGun {
                    ship: id,
                    level,
                    multifire: multi,
                });
            }
            Shot::Bomb { level, cost, mine } => {
                energy_cost = cost;
                let (kind, v, end) = if mine {
                    (
                        ProjectileKind::Mine,
                        Vel::default(),
                        tick.wrapping_add(self.settings.mine_alive_time),
                    )
                } else {
                    // Recoil is applied before the bomb inherits the ship's velocity.
                    let recoil = scale_unit(fwd, s.bomb_thrust as i64 * VEL_SCALE as i64);
                    let ship = &mut self.ships[index];
                    ship.vel.x -= recoil.x as i32;
                    ship.vel.y -= recoil.y as i32;
                    let v = scale_unit(fwd, s.bomb_speed as i64 * VEL_SCALE as i64);
                    let v = Vel::new(
                        base_vel.x - recoil.x as i32 + v.x as i32,
                        base_vel.y - recoil.y as i32 + v.y as i32,
                    );
                    (
                        ProjectileKind::Bomb,
                        v,
                        tick.wrapping_add(self.settings.bomb_alive_time),
                    )
                };
                let mut pr = self.new_projectile(id, freq, kind, level, origin, v, end);
                pr.bounces_left = if mine { 0 } else { s.bomb_bounce_count };
                pr.emp = s.emp_bomb;
                pr.proximity = proximity;
                (pr.shrap, pr.shrap_level, pr.shrap_bouncing) = shrap;
                pr.rng_seed = shrap_seed(&pr);
                self.projectiles.push(pr);
                let delay = if mine {
                    s.landmine_fire_delay
                } else {
                    s.bomb_fire_delay
                };
                let ship = &mut self.ships[index];
                ship.next_bomb_tick = tick.wrapping_add(delay);
                if !s.emp_bomb {
                    ship.next_gun_tick = later(ship.next_gun_tick, ship.next_bomb_tick);
                    ship.next_repel_tick = tick.wrapping_add(REPEL_DELAY);
                }
                events.push(if mine {
                    Event::LaidMine { ship: id, level }
                } else {
                    Event::FiredBomb { ship: id, level }
                });
            }
        }

        let ship = &mut self.ships[index];
        if !free {
            ship.energy -= energy_cost;
        }
        if ship.is_on(toggle::CLOAK) {
            // Firing breaks cloak.
            ship.toggles = (ship.toggles & !toggle::CLOAK) | toggle::FLASH;
        }
    }

    /// Item use (repel, burst, thor, decoy) blocks weapons for the bomb delay.
    fn item_delay(&mut self, index: usize, delay: u32) {
        let tick = self.tick;
        let ship = &mut self.ships[index];
        ship.next_bomb_tick = later(ship.next_bomb_tick, tick.wrapping_add(delay));
        ship.next_gun_tick = later(ship.next_gun_tick, tick.wrapping_add(delay));
        ship.next_repel_tick = tick.wrapping_add(REPEL_DELAY);
    }

    fn portal_and_warp(&mut self, index: usize, input: ShipInput, events: &mut Vec<Event>) {
        let tick = self.tick;
        let id = self.ships[index].id;
        if input.portal && self.ships[index].items.portals > 0 {
            if self.is_antiwarped(index) {
                events.push(Event::Antiwarped { ship: id });
            } else {
                let ship = &mut self.ships[index];
                ship.items.portals -= 1;
                ship.portal = Some((ship.pos, tick.wrapping_add(self.settings.warp_point_delay)));
                events.push(Event::PortalPlaced {
                    ship: id,
                    pos: ship.pos,
                });
            }
        }
        if !input.warp || input.portal {
            return;
        }
        if self.is_antiwarped(index) {
            events.push(Event::Antiwarped { ship: id });
            return;
        }
        let settle = self.settings.anti_warp_settle_delay;
        let from = self.ships[index].pos;
        if let Some((to, _)) = self.ships[index].portal.take() {
            let ship = &mut self.ships[index];
            ship.pos = to;
            ship.prev_pos = to;
            ship.toggles |= toggle::FLASH;
            ship.next_bomb_tick = tick.wrapping_add(REPEL_DELAY);
            ship.fake_antiwarp_end_tick = tick.wrapping_add(settle);
            events.push(Event::Warped { ship: id, from, to });
        } else if tick_after(tick, self.ships[index].next_bomb_tick) {
            let full = self.ships[index].stats.max_energy as i32 * ENERGY_SCALE;
            if self.ships[index].energy < full {
                events.push(Event::WarpNeedsEnergy { ship: id });
            } else {
                let mut ship = self.ships[index].clone();
                self.place_at_spawn(&mut ship);
                ship.energy = ENERGY_SCALE;
                ship.toggles |= toggle::FLASH;
                ship.fake_antiwarp_end_tick = tick.wrapping_add(settle);
                ship.next_bomb_tick = tick.wrapping_add(REPEL_DELAY);
                let to = ship.pos;
                self.ships[index] = ship;
                events.push(Event::Warped { ship: id, from, to });
            }
        }
    }

    /// Pushes enemy weapons and ships away from ship `index`.
    pub(crate) fn repel(&mut self, index: usize, _events: &mut [Event]) {
        let tick = self.tick;
        let me = &self.ships[index];
        let (center, freq) = (me.pos.pixels(), me.freq);
        let reach = self.settings.repel_distance as i32;
        let speed = self.settings.repel_speed as i64 * VEL_SCALE as i64;
        let push = |p: Vec2<i32>| -> Option<Vel> {
            let (dx, dy) = (p.x - center.x, p.y - center.y);
            if dx.abs() > reach || dy.abs() > reach {
                return None;
            }
            let len = isqrt(dx as i64 * dx as i64 + dy as i64 * dy as i64).max(1);
            Some(Vel::new(
                (dx as i64 * speed / len) as i32,
                (dy as i64 * speed / len) as i32,
            ))
        };
        let (bomb_time, bullet_time, decoy_time) = (
            self.settings.bomb_alive_time,
            self.settings.bullet_alive_time,
            self.settings.decoy_alive_time,
        );
        for pr in &mut self.projectiles {
            if pr.freq == freq {
                continue;
            }
            if let Some(v) = push(pr.pos.pixels()) {
                pr.vel = v;
                if pr.kind == ProjectileKind::Mine {
                    pr.kind = ProjectileKind::Bomb;
                }
                let life = match pr.kind {
                    ProjectileKind::Bomb | ProjectileKind::Thor | ProjectileKind::Mine => bomb_time,
                    ProjectileKind::Decoy => decoy_time,
                    _ => bullet_time,
                };
                pr.end_tick = tick.wrapping_add(life);
            }
        }
        let repel_time = self.settings.repel_time;
        for ship in &mut self.ships {
            if ship.freq == freq || !ship.is_alive() {
                continue;
            }
            let p = ship.pos.pixels();
            if self.map.tile_at_pixel(p.x, p.y) == tile::SAFE {
                continue;
            }
            if let Some(v) = push(p) {
                ship.vel = v;
                ship.repelled_end_tick = tick.wrapping_add(repel_time);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_projectile(
        &mut self,
        owner: u16,
        freq: u16,
        kind: ProjectileKind,
        level: u8,
        pos: Pos,
        vel: Vel,
        end_tick: u32,
    ) -> Projectile {
        let id = self.next_projectile_id;
        self.next_projectile_id = self.next_projectile_id.wrapping_add(1);
        Projectile {
            id,
            owner,
            freq,
            kind,
            level,
            pos,
            vel,
            spawn_tick: self.tick,
            end_tick,
            prev_pos: pos,
            bouncing: false,
            bounces_left: 0,
            emp: false,
            proximity: false,
            shrap: 0,
            shrap_level: 0,
            shrap_bouncing: false,
            armed: false,
            prox: None,
            rng_seed: 0,
            launch_rotation: 0,
        }
    }

    pub(crate) fn step_projectiles(&mut self, events: &mut Vec<Event>) {
        let mut i = 0;
        while i < self.projectiles.len() {
            match self.step_projectile(i) {
                Outcome::Continue => i += 1,
                Outcome::Vanish => {
                    self.projectiles.swap_remove(i);
                }
                Outcome::Explode { at, target } => {
                    let pr = self.projectiles.swap_remove(i);
                    self.explode(&pr, at, target, events);
                }
            }
        }
    }

    fn step_projectile(&mut self, i: usize) -> Outcome {
        let tick = self.tick;
        let terrain = Terrain {
            map: &self.map,
            bricks: &self.bricks,
        };
        let pr = &mut self.projectiles[i];
        pr.prev_pos = pr.pos;
        if tick_reached(tick, pr.end_tick) {
            return Outcome::Vanish;
        }
        let before = pr.pos;

        if pr.kind != ProjectileKind::Mine {
            let walls = pr.kind != ProjectileKind::Thor;
            let hit_x = move_axis(&terrain, pr, 0, walls);
            let hit_y = move_axis(&terrain, pr, 1, walls);
            if hit_x || hit_y {
                let age = tick.wrapping_sub(pr.spawn_tick);
                match pr.kind {
                    ProjectileKind::Shrapnel if age <= INACTIVE_SHRAP_TICKS => {
                        return Outcome::Vanish;
                    }
                    ProjectileKind::Bullet | ProjectileKind::Shrapnel if !pr.bouncing => {
                        return Outcome::Explode {
                            at: pr.pos,
                            target: None,
                        };
                    }
                    ProjectileKind::Bomb => {
                        if pr.bounces_left == 0 {
                            return Outcome::Explode {
                                at: pr.pos,
                                target: None,
                            };
                        }
                        pr.bounces_left -= 1;
                    }
                    ProjectileKind::Burst => pr.armed = true,
                    _ => {}
                }
            }
            let p = pr.pos.pixels();
            if self.map.tile_at_pixel(p.x, p.y) == tile::WORMHOLE {
                return Outcome::Vanish;
            }
        }

        let pr = &self.projectiles[i];
        if pr.kind == ProjectileKind::Decoy || (pr.kind == ProjectileKind::Burst && !pr.armed) {
            return Outcome::Continue;
        }

        // A triggered proximity bomb waits until the target starts moving away or the
        // sensor delay runs out, then explodes where it was.
        if let Some(trig) = pr.prox {
            let Some(target) = self
                .ships
                .iter()
                .find(|s| s.id == trig.target && s.is_alive())
            else {
                return Outcome::Explode {
                    at: before,
                    target: None,
                };
            };
            let (w, t) = (pr.pos.pixels(), target.pos.pixels());
            let off = (w.x - t.x).abs().max((w.y - t.y).abs());
            if off > trig.closest || tick_reached(tick, trig.sensor_end_tick) {
                return Outcome::Explode {
                    at: before,
                    target: Some(trig.target),
                };
            }
            if let Some(t) = self.projectiles[i].prox.as_mut() {
                t.closest = off;
            }
            return Outcome::Continue;
        }

        let wp = pr.pos.pixels();
        let sense = if pr.proximity && pr.is_bomb_like() {
            let extra = if pr.kind == ProjectileKind::Thor {
                3
            } else {
                0
            };
            ((self.settings.proximity_distance + pr.level as u32 + extra) * 18) as i32 - 14
        } else {
            WEAPON_HIT_RADIUS
        };
        for ship in &self.ships {
            if !ship.is_alive() || ship.freq == pr.freq {
                continue;
            }
            let r = self.settings.ships[ship.kind as usize].radius_pixels();
            let sp = ship.pos.pixels();
            if !boxes_overlap(sp.x, sp.y, r, wp.x, wp.y, sense) {
                continue;
            }
            if sense != WEAPON_HIT_RADIUS {
                let fresh = tick == pr.spawn_tick.wrapping_add(1);
                let delay = if fresh {
                    0
                } else {
                    self.settings.bomb_explode_delay
                };
                let closest = (wp.x - sp.x).abs().max((wp.y - sp.y).abs());
                let target = ship.id;
                let direct = boxes_overlap(sp.x, sp.y, r, wp.x, wp.y, WEAPON_HIT_RADIUS);
                if direct {
                    return Outcome::Explode {
                        at: before,
                        target: Some(target),
                    };
                }
                self.projectiles[i].prox = Some(ProxTrigger {
                    target,
                    closest,
                    sensor_end_tick: tick.wrapping_add(delay),
                });
                return Outcome::Continue;
            }
            // Bombs explode where they were before entering the ship.
            let at = if pr.is_bomb_like() { before } else { pr.pos };
            return Outcome::Explode {
                at,
                target: Some(ship.id),
            };
        }
        Outcome::Continue
    }

    /// Ends a projectile at `at`, applying damage and releasing shrapnel.
    fn explode(&mut self, pr: &Projectile, at: Pos, direct: Option<u16>, events: &mut Vec<Event>) {
        events.push(Event::Explosion {
            pos: at,
            kind: pr.kind,
            level: pr.level,
            emp: pr.emp,
        });
        let a = &self.settings;
        let tick = self.tick;
        if !pr.is_bomb_like() {
            if let Some(target) = direct {
                let damage = match pr.kind {
                    ProjectileKind::Burst => a.burst_damage_level,
                    ProjectileKind::Shrapnel => {
                        if tick.wrapping_sub(pr.spawn_tick) <= INACTIVE_SHRAP_TICKS {
                            a.inactive_shrap_damage * pr.level as u32
                        } else {
                            (a.bullet_damage_level + a.bullet_damage_upgrade * pr.level as u32)
                                * a.shrapnel_damage_percent
                                / 1000
                        }
                    }
                    _ => a.bullet_damage_level + a.bullet_damage_upgrade * pr.level as u32,
                };
                let damage = self.randomise_bullet_damage(damage);
                self.damage(target, pr.owner, damage, 0, events);
            }
            return;
        }

        // Splash damage.
        let (mut bomb_dmg, mut level) = (a.bomb_damage_level as i64, pr.level as u32);
        if pr.kind == ProjectileKind::Thor {
            bomb_dmg += bomb_dmg * (level * level) as i64;
            level += 3;
        }
        if pr.emp {
            bomb_dmg = bomb_dmg * a.ebomb_damage_percent as i64 / 1000;
        }
        let owner_bounces = self
            .ships
            .iter()
            .find(|s| s.id == pr.owner)
            .is_some_and(|s| self.settings.ships[s.kind as usize].bomb_bounce_count > 0);
        if owner_bounces {
            bomb_dmg = bomb_dmg * a.bbomb_damage_percent as i64 / 1000;
        }
        let explode = (a.bomb_explode_pixels * (1 + level)) as i64;
        let emp_time = a.ebomb_shutdown_time as i64;
        if explode > 0 && bomb_dmg > 0 {
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
                    let emp = if pr.emp && ship.id != pr.owner {
                        (emp_time * dmg / bomb_dmg) as u32
                    } else {
                        0
                    };
                    hits.push((ship.id, dmg as u32, emp));
                }
            }
            for (id, dmg, emp) in hits {
                self.damage(id, pr.owner, dmg, emp, events);
            }
        }

        // Shrapnel.
        if pr.shrap > 0 {
            let count = pr.shrap as i64;
            let mut rng = VieRng::new(pr.rng_seed);
            let random = self.settings.shrapnel_random;
            let speed = self.settings.shrapnel_speed as i64 * VEL_SCALE as i64;
            let end = tick.wrapping_add(self.settings.bullet_alive_time);
            for i in 0..count {
                let angle = if random {
                    (rng.next_u32() % 40_000) as i64
                } else {
                    i * 40_000 / count
                };
                let v = vel(scale_unit(heading(angle), speed));
                let p = at.pixels();
                if self
                    .terrain()
                    .is_solid(p.x.div_euclid(16), p.y.div_euclid(16), pr.freq)
                {
                    continue;
                }
                let mut s = self.new_projectile(
                    pr.owner,
                    pr.freq,
                    ProjectileKind::Shrapnel,
                    pr.shrap_level,
                    at,
                    v,
                    end,
                );
                s.bouncing = pr.shrap_bouncing;
                self.projectiles.push(s);
            }
        }
    }

    /// Continuum randomises bullet damage unless `ExactDamage` is set:
    /// `sqrt(random % (damage² + 1))`.
    fn randomise_bullet_damage(&mut self, damage: u32) -> u32 {
        if self.settings.exact_damage || damage == 0 {
            return damage;
        }
        let d = damage as u64;
        let r = (self.rng.next_u32() as u64 * 1000) % (d * d + 1);
        isqrt(r as i64) as u32
    }

    pub(crate) fn damage(
        &mut self,
        target: u16,
        shooter: u16,
        damage: u32,
        emp: u32,
        events: &mut Vec<Event>,
    ) {
        let tick = self.tick;
        let enter_delay = self.settings.enter_delay;
        let Some(index) = self.ships.iter().position(|s| s.id == target) else {
            return;
        };
        let ship = &self.ships[index];
        if !ship.takes_damage || !ship.is_alive() {
            return;
        }
        let p = ship.pos.pixels();
        if self.map.tile_at_pixel(p.x, p.y) == tile::SAFE {
            return;
        }
        let mut damage = damage;
        if active(tick, ship.shield_end_tick) {
            let total = self.settings.ships[ship.kind as usize].shields_time.max(1);
            let left = ship.shield_end_tick.wrapping_sub(tick).min(total);
            damage -= (damage as u64 * left as u64 / total as u64) as u32;
        }
        if damage == 0 {
            return;
        }
        events.push(Event::ShipHit {
            ship: target,
            shooter,
            damage,
        });
        let ship = &mut self.ships[index];
        if emp > 0 {
            ship.emp_end_tick = later(ship.emp_end_tick, tick.wrapping_add(emp));
        }
        let dmg = damage as i32 * ENERGY_SCALE;
        if ship.energy > dmg {
            ship.energy -= dmg;
            return;
        }
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
        let pos = ship.pos;
        events.push(Event::ShipKilled {
            ship: target,
            killer: shooter,
            pos,
        });
        let bonus = self.settings.bounty_increase_for_kill;
        if let Some(k) = self.ships.iter_mut().find(|s| s.id == shooter) {
            k.bounty += bonus;
        }
    }
}

pub(crate) fn vel(v: Vec2<i64>) -> Vel {
    Vel::new(v.x as i32, v.y as i32)
}

/// Seed for a bomb's shrapnel spread, from its launch state (so every client that saw
/// the same weapon packet sprays the same shrapnel).
pub(crate) fn shrap_seed(pr: &Projectile) -> u32 {
    let p = pr.pos.pixels();
    let v = pr.vel.to_wire();
    (pr.shrap as u32)
        .wrapping_add(pr.level as u32)
        .wrapping_add((p.x as u32).wrapping_mul(1000))
        .wrapping_add((p.y as u32).wrapping_mul(1000))
        .wrapping_add(v.x as u32)
        .wrapping_add(v.y as u32)
        .wrapping_add(pr.freq as u32)
}

/// Inclusive-max box overlap test, matching Continuum's ship/weapon checks.
pub(crate) const fn boxes_overlap(ax: i32, ay: i32, ar: i32, bx: i32, by: i32, br: i32) -> bool {
    ax + ar >= bx - br && ax - ar < bx + br && ay + ar >= by - br && ay - ar < by + br
}

/// Moves a projectile along one axis; on entering a solid tile restores and reflects
/// that axis. Returns whether it hit a wall.
fn move_axis(terrain: &Terrain, pr: &mut Projectile, axis: usize, walls: bool) -> bool {
    let previous = pr.pos;
    if axis == 0 {
        pr.pos.x += pr.vel.x as i64;
    } else {
        pr.pos.y += pr.vel.y as i64;
    }
    if !walls {
        return false;
    }
    let p = pr.pos.pixels();
    if terrain.is_solid(p.x.div_euclid(16), p.y.div_euclid(16), pr.freq) {
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
