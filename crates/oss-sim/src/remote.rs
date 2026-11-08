//! Ships driven by the network: other players, as the server reports them.
//!
//! Each client simulates only its own ship authoritatively. Other ships are placed
//! from position packets and then extrapolated by the normal physics between updates
//! (they bounce off walls but never respawn, warp or take damage locally); the weapons
//! they fire are recreated from weapon packets so every client sees the same shots.

use crate::fixed::{DIRECTION_HEADINGS, POS_SCALE, Pos, VEL_SCALE, Vec2, Vel, heading, scale_unit};
use crate::ship::{Ship, ShipState, toggle};
use crate::weapons::{ProjectileKind, shrap_seed, vel};
use crate::world::{Event, World, move_axis};

/// Longest a reported position is carried forward (1 s).
const MAX_EXTRAPOLATION: u32 = 100;
/// Ticks over which a small correction is blended in (Continuum uses 200 ms).
const LERP_TICKS: u32 = 20;
/// Corrections of four tiles or more on either axis jump instead of gliding.
const SNAP_DISTANCE: i64 = 64 * POS_SCALE;

/// What a weapon packet says was fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteWeaponKind {
    Bullet,
    BouncingBullet,
    Bomb,
    ProximityBomb,
    Repel,
    Decoy,
    Burst,
    Thor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteWeapon {
    pub kind: RemoteWeaponKind,
    /// 0..=3.
    pub level: u8,
    /// Multifire for bullets, mine for bombs.
    pub alternate: bool,
    pub shrap: u8,
    pub shrap_level: u8,
    pub shrap_bouncing: bool,
}

/// A position report for a remote ship, in wire units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemoteState {
    /// Pixels.
    pub x: i32,
    pub y: i32,
    /// Pixels per 10 s.
    pub x_speed: i32,
    pub y_speed: i32,
    /// 0..40.
    pub direction: u8,
    pub toggles: u8,
    pub bounty: u32,
    pub energy: Option<u32>,
}

impl World {
    pub fn ship_index(&self, id: u16) -> Option<usize> {
        self.ships.iter().position(|s| s.id == id)
    }

    /// Adds a network-driven ship. It stays dead (invisible) until its first position.
    pub fn add_remote_ship(&mut self, id: u16, kind: u8, freq: u16) -> usize {
        if let Some(i) = self.ship_index(id) {
            self.ships.remove(i);
        }
        let kind = kind.min(7);
        let mut ship = Ship::new(id, kind, freq, &self.settings.ships[kind as usize]);
        ship.takes_damage = false;
        ship.remote = true;
        ship.state = ShipState::Dead {
            respawn_tick: self.tick,
        };
        self.ships.push(ship);
        self.ships.len() - 1
    }

    pub fn remove_ship(&mut self, id: u16) {
        self.ships.retain(|s| s.id != id);
    }

    /// Places a remote ship from a position report; `age_ticks` is how old the report
    /// is, and the ship is moved forward along its velocity by that much.
    pub fn set_remote_state(&mut self, index: usize, st: &RemoteState, age_ticks: u32) {
        let Some(ship) = self.ships.get(index) else {
            return;
        };
        // Where the ship is now: the reported state carried forward `age` ticks,
        // bouncing off walls on the way.
        let radius = self.settings.ships[ship.kind as usize % 8].radius_pixels();
        let bounce = self.settings.bounce_factor.max(1) as i64;
        let mut probe = ship.clone();
        probe.pos = Pos::new(st.x as i64 * POS_SCALE, st.y as i64 * POS_SCALE);
        probe.vel = Vel::from_wire(st.x_speed, st.y_speed);
        probe.lerp_ticks = 0;
        let terrain = self.terrain();
        for _ in 0..age_ticks.min(MAX_EXTRAPOLATION) {
            move_axis(&terrain, &mut probe, radius, 0, bounce);
            move_axis(&terrain, &mut probe, radius, 1, bounce);
        }
        let (pos, v) = (probe.pos, probe.vel);

        let ship = &mut self.ships[index];
        let was_alive = ship.is_alive();
        ship.state = ShipState::Alive;
        ship.vel = v;
        let (dx, dy) = (pos.x - ship.pos.x, pos.y - ship.pos.y);
        let far = dx.abs() >= SNAP_DISTANCE || dy.abs() >= SNAP_DISTANCE;
        if !was_alive || far || st.toggles & toggle::FLASH != 0 {
            // Just spawned, warped or badly out of step: jump there.
            ship.pos = pos;
            ship.prev_pos = pos;
            ship.lerp_ticks = 0;
        } else {
            // Close: glide the difference in over the next 200 ms.
            ship.lerp = Vel {
                x: (dx / LERP_TICKS as i64) as i32,
                y: (dy / LERP_TICKS as i64) as i32,
            };
            ship.lerp_ticks = LERP_TICKS;
        }
        ship.rotation = (st.direction as u32 % 40) * 1000;
        ship.toggles = st.toggles;
        ship.bounty = st.bounty;
        if let Some(e) = st.energy {
            ship.energy = e.min(i32::MAX as u32 / 1000) as i32 * 1000;
        }
    }

    /// Marks a ship dead (after a kill reported by the server).
    pub fn kill_ship(&mut self, id: u16, killer: u16, events: &mut Vec<Event>) {
        let enter_delay = self.settings.enter_delay;
        let tick = self.tick;
        if let Some(ship) = self.ships.iter_mut().find(|s| s.id == id && s.is_alive()) {
            ship.state = ShipState::Dead {
                respawn_tick: tick.wrapping_add(enter_delay),
            };
            ship.vel = Vel::default();
            events.push(Event::ShipKilled {
                ship: id,
                killer,
                pos: ship.pos,
            });
        }
    }

    /// Recreates a weapon fired by ship `index` (already placed with
    /// [`World::set_remote_state`] at the moment of firing).
    pub fn fire_remote(&mut self, index: usize, w: RemoteWeapon, events: &mut Vec<Event>) {
        let Some(ship) = self.ships.get(index) else {
            return;
        };
        let s = self.settings.ships[ship.kind as usize];
        let tick = self.tick;
        let (id, freq) = (ship.id, ship.freq);
        let p = ship.pos.pixels();
        let origin = Pos::from_pixels(p.x, p.y);
        let base = {
            let v = ship.vel.to_wire();
            Vel::from_wire(v.x, v.y)
        };
        let direction = ship.direction();
        let fwd = DIRECTION_HEADINGS[direction as usize];
        let rotation = ship.rotation;
        let level = w.level.min(3);
        let add = |a: Vel, b: Vec2<i64>| Vel::new(a.x + b.x as i32, a.y + b.y as i32);

        match w.kind {
            RemoteWeaponKind::Bullet | RemoteWeaponKind::BouncingBullet => {
                let end = tick.wrapping_add(self.settings.bullet_alive_time);
                let bouncing = w.kind == RemoteWeaponKind::BouncingBullet;
                let spawn = |world: &mut World, pos: Pos, unit: Vec2<i32>| {
                    let v = add(
                        base,
                        scale_unit(unit, s.bullet_speed as i64 * VEL_SCALE as i64),
                    );
                    let mut pr =
                        world.new_projectile(id, freq, ProjectileKind::Bullet, level, pos, v, end);
                    pr.bouncing = bouncing;
                    world.projectiles.push(pr);
                };
                if s.double_barrel {
                    let off = scale_unit(
                        Vec2::new(-fwd.y, fwd.x),
                        s.radius_pixels() as i64 * 3 / 4 * POS_SCALE,
                    );
                    spawn(self, Pos::new(origin.x - off.x, origin.y - off.y), fwd);
                    spawn(self, Pos::new(origin.x + off.x, origin.y + off.y), fwd);
                } else {
                    spawn(self, origin, fwd);
                }
                if w.alternate {
                    let spread = s.multi_fire_angle as i64 * 40_000 / (111 * 360);
                    let b = direction as i64 * 1000;
                    spawn(self, origin, heading(b + spread));
                    spawn(self, origin, heading(b - spread));
                }
                events.push(Event::FiredGun {
                    ship: id,
                    level,
                    multifire: w.alternate,
                });
            }
            RemoteWeaponKind::Bomb | RemoteWeaponKind::ProximityBomb | RemoteWeaponKind::Thor => {
                let thor = w.kind == RemoteWeaponKind::Thor;
                let mine = w.alternate && !thor;
                let (kind, v, life) = if thor {
                    let v = add(
                        base,
                        scale_unit(fwd, s.bomb_speed as i64 * VEL_SCALE as i64),
                    );
                    (ProjectileKind::Thor, v, self.settings.bomb_alive_time)
                } else if mine {
                    (
                        ProjectileKind::Mine,
                        Vel::default(),
                        self.settings.mine_alive_time,
                    )
                } else {
                    let v = add(
                        base,
                        scale_unit(fwd, s.bomb_speed as i64 * VEL_SCALE as i64),
                    );
                    (ProjectileKind::Bomb, v, self.settings.bomb_alive_time)
                };
                let mut pr = self.new_projectile(
                    id,
                    freq,
                    kind,
                    if thor { 0 } else { level },
                    origin,
                    v,
                    tick.wrapping_add(life),
                );
                pr.bounces_left = if kind == ProjectileKind::Bomb {
                    s.bomb_bounce_count
                } else {
                    0
                };
                pr.emp = s.emp_bomb && !thor;
                pr.proximity = thor || w.kind == RemoteWeaponKind::ProximityBomb;
                pr.shrap = w.shrap;
                pr.shrap_level = w.shrap_level.min(3);
                pr.shrap_bouncing = w.shrap_bouncing;
                pr.rng_seed = shrap_seed(&pr);
                self.projectiles.push(pr);
                events.push(match (thor, mine) {
                    (true, _) => Event::ThorFired { ship: id },
                    (_, true) => Event::LaidMine { ship: id, level },
                    _ => Event::FiredBomb { ship: id, level },
                });
            }
            RemoteWeaponKind::Burst => {
                let count = s.burst_shrapnel.max(1) as i64;
                let end = tick.wrapping_add(self.settings.bullet_alive_time);
                for i in 0..count {
                    let v = vel(scale_unit(
                        heading(i * 40_000 / count),
                        s.burst_speed as i64 * VEL_SCALE as i64,
                    ));
                    let mut pr =
                        self.new_projectile(id, freq, ProjectileKind::Burst, 0, origin, v, end);
                    pr.bouncing = true;
                    self.projectiles.push(pr);
                }
                events.push(Event::BurstFired { ship: id });
            }
            RemoteWeaponKind::Decoy => {
                let end = tick.wrapping_add(self.settings.decoy_alive_time);
                let mut pr =
                    self.new_projectile(id, freq, ProjectileKind::Decoy, 0, origin, base, end);
                pr.launch_rotation = rotation;
                pr.bouncing = true;
                self.projectiles.push(pr);
                events.push(Event::DecoyLaunched { ship: id });
            }
            RemoteWeaponKind::Repel => {
                self.repel(index, events);
                events.push(Event::Repelled {
                    ship: id,
                    pos: origin,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArenaSettings, ShipInput, TileMap};

    fn world() -> World {
        let mut s = ArenaSettings::default();
        for ship in &mut s.ships {
            ship.initial_bounty = 0;
        }
        s.exact_damage = true;
        let mut w = World::new(TileMap::empty(), s, 1);
        w.prize_seed = 0;
        w
    }

    #[test]
    fn remote_ship_follows_reports_and_extrapolates() {
        let mut w = world();
        let i = w.add_remote_ship(7, 2, 1);
        assert!(!w.ships[i].is_alive());
        let st = RemoteState {
            x: 8000,
            y: 8000,
            x_speed: 1000,
            direction: 10,
            ..Default::default()
        };
        w.set_remote_state(i, &st, 10);
        // 10 ticks old at 1000 px/10s = 10 px further on.
        assert_eq!(w.ships[i].pos.pixels(), Vec2::new(8010, 8000));
        w.step(&[]);
        assert_eq!(w.ships[i].pos.pixels(), Vec2::new(8011, 8000));
        // Dead remote ships stay dead until the next report.
        let mut ev = Vec::new();
        w.kill_ship(7, 3, &mut ev);
        for _ in 0..500 {
            w.step(&[]);
        }
        assert!(!w.ships[w.ship_index(7).unwrap()].is_alive());
    }

    #[test]
    fn remote_bullets_hit_the_local_ship() {
        let mut w = world();
        let me = w.add_ship(1, 0, 0);
        w.ships[me].pos = Pos::from_pixels(8000, 8000);
        w.ships[me].energy = 100_000;
        let r = w.add_remote_ship(2, 0, 1);
        w.set_remote_state(
            r,
            &RemoteState {
                x: 7900,
                y: 8000,
                direction: 10,
                ..Default::default()
            },
            0,
        );
        let mut ev = Vec::new();
        let gun = RemoteWeapon {
            kind: RemoteWeaponKind::Bullet,
            level: 0,
            alternate: false,
            shrap: 0,
            shrap_level: 0,
            shrap_bouncing: false,
        };
        w.fire_remote(r, gun, &mut ev);
        assert_eq!(w.projectiles.len(), 1);
        let mut hit = false;
        for _ in 0..100 {
            hit |= w.step(&[ShipInput::default()]).iter().any(|e| {
                matches!(
                    e,
                    Event::ShipKilled {
                        ship: 1,
                        killer: 2,
                        ..
                    }
                )
            });
        }
        assert!(hit, "200 damage kills a ship with 100 energy");
    }
}
