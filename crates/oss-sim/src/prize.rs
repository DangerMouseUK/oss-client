//! Prizes ("greens"): weighted random generation, applying a prize to a ship, the
//! initial-bounty prizes a ship gets on spawn, and the greens that appear on the map.
//!
//! Generation uses [`VieRng`] seeded from the arena's prize seed so every client
//! produces the same greens; behaviour follows the Continuum client as documented by
//! the nullspace client (MIT).

use crate::fixed::{ENERGY_SCALE, VieRng};
use crate::map::MAP_TILES;
use crate::settings::{ArenaSettings, PRIZE_COUNT};
use crate::ship::{Items, Ship, ShipStats, active, cap, tick_after};
use crate::world::{Event, World};

/// Prize ids as sent on the wire (negative ids are the "bad" version).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Prize {
    QuickCharge = 1,
    Energy,
    Rotation,
    Stealth,
    Cloak,
    XRadar,
    Warp,
    Guns,
    Bombs,
    BouncingBullets,
    Thruster,
    TopSpeed,
    FullCharge,
    EngineShutdown,
    Multifire,
    Proximity,
    Super,
    Shields,
    Shrapnel,
    Antiwarp,
    Repel,
    Burst,
    Decoy,
    Thor,
    Multiprize,
    Brick,
    Rocket,
    Portal,
}

impl Prize {
    pub const ALL: [Prize; PRIZE_COUNT] = [
        Prize::QuickCharge,
        Prize::Energy,
        Prize::Rotation,
        Prize::Stealth,
        Prize::Cloak,
        Prize::XRadar,
        Prize::Warp,
        Prize::Guns,
        Prize::Bombs,
        Prize::BouncingBullets,
        Prize::Thruster,
        Prize::TopSpeed,
        Prize::FullCharge,
        Prize::EngineShutdown,
        Prize::Multifire,
        Prize::Proximity,
        Prize::Super,
        Prize::Shields,
        Prize::Shrapnel,
        Prize::Antiwarp,
        Prize::Repel,
        Prize::Burst,
        Prize::Decoy,
        Prize::Thor,
        Prize::Multiprize,
        Prize::Brick,
        Prize::Rocket,
        Prize::Portal,
    ];

    pub fn from_id(id: u8) -> Option<Prize> {
        Self::ALL.get((id as usize).checked_sub(1)?).copied()
    }

    /// The message Continuum shows when you pick this prize up.
    pub fn message(self, negative: bool) -> &'static str {
        use Prize::*;
        match (self, negative) {
            (QuickCharge, false) => "Charge rate increased.",
            (QuickCharge, true) => "Charge rate decreased.",
            (Energy, false) => "Maximum energy level increased.",
            (Energy, true) => "Maximum energy level decreased.",
            (Rotation, false) => "Rotation speed increased.",
            (Rotation, true) => "Rotation speed decreased.",
            (Stealth, false) => "Stealth available.",
            (Stealth, true) => "Stealth lost.",
            (Cloak, false) => "Cloak available.",
            (Cloak, true) => "Cloak lost.",
            (XRadar, false) => "X-Radar available.",
            (XRadar, true) => "X-Radar lost.",
            (Warp, _) => "Warp!",
            (Guns, false) => "Guns upgraded.",
            (Guns, true) => "Guns downgraded.",
            (Bombs, false) => "Bombs upgraded.",
            (Bombs, true) => "Bombs downgraded.",
            (BouncingBullets, false) => "Bouncing bullets.",
            (BouncingBullets, true) => "Bouncing bullets lost.",
            (Thruster, false) => "Thrusters upgraded.",
            (Thruster, true) => "Thrusters downgraded.",
            (TopSpeed, false) => "Top speed increased.",
            (TopSpeed, true) => "Top speed reduced.",
            (FullCharge, false) => "Full charge.",
            (FullCharge, true) => "Energy depleted.",
            (EngineShutdown, false) => "Engines shut-down.",
            (EngineShutdown, true) => "Engines shut-down (severe).",
            (Multifire, false) => "MultiFire bullets.",
            (Multifire, true) => "MultiFire lost.",
            (Proximity, false) => "Proximity bombs.",
            (Proximity, true) => "Proximity bombs lost.",
            (Super, _) => "Temporary SuperPower!",
            (Shields, _) => "Temporary Shields.",
            (Shrapnel, false) => "Shrapnel increased.",
            (Shrapnel, true) => "Shrapnel reduced.",
            (Antiwarp, false) => "AntiWarp available.",
            (Antiwarp, true) => "AntiWarp lost.",
            (Repel, false) => "Repeller increased.",
            (Repel, true) => "Repeller lost.",
            (Burst, false) => "Burst increased.",
            (Burst, true) => "Burst lost.",
            (Decoy, false) => "Decoy increased.",
            (Decoy, true) => "Decoy lost.",
            (Thor, false) => "Thor's hammer increased.",
            (Thor, true) => "Thor's hammer lost.",
            (Multiprize, _) => "MultiPrize!",
            (Brick, false) => "Brick increased.",
            (Brick, true) => "Brick lost.",
            (Rocket, false) => "Rocket increased.",
            (Rocket, true) => "Rocket lost.",
            (Portal, false) => "Portal increased.",
            (Portal, true) => "Portal lost.",
        }
    }

    /// Prizes never handed out as part of a random batch (initial bounty, multiprize).
    const fn excluded_from_batches(self) -> bool {
        matches!(
            self,
            Prize::FullCharge
                | Prize::EngineShutdown
                | Prize::Shields
                | Prize::Super
                | Prize::Warp
                | Prize::Brick
                | Prize::Multiprize
        )
    }
}

/// Picks a weighted random prize, advancing `seed`. Returns the prize id (negative for
/// a bad prize when allowed), or 0 when no prize has any weight.
pub fn generate_prize(seed: &mut u32, settings: &ArenaSettings, negative_allowed: bool) -> i32 {
    let total = settings.prize_weight_total();
    if total == 0 {
        return 0;
    }
    let mut rng = VieRng::new(*seed);
    let mut random = rng.next_u32();
    let mut result = 0;
    let mut weight = 0u32;
    for (i, &w) in settings.prize_weights.iter().enumerate() {
        weight += w as u32;
        if random % total < weight {
            random = rng.next_u32();
            let id = i as i32 + 1;
            let factor = settings.prize_negative_factor;
            result = if negative_allowed && factor != 0 && random.is_multiple_of(factor) {
                -id
            } else {
                id
            };
            break;
        }
    }
    *seed = random;
    result
}

/// A prize lying on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Green {
    /// Tile coordinates.
    pub x: u16,
    pub y: u16,
    /// Prize id (negative = bad prize).
    pub prize: i32,
    pub end_tick: u32,
}

/// Most greens on the map at once.
const MAX_GREENS: usize = 256;

impl World {
    /// Applies prize `id` (negative = bad version, 0 = a random good prize) to ship
    /// `index`. Picking up a positive prize raises bounty by one.
    pub fn apply_prize(&mut self, index: usize, id: i32, events: &mut Vec<Event>) {
        let Some(ship) = self.ships.get(index) else {
            return;
        };
        let ship_id = ship.id;
        if id == 0 {
            // A random prize, drawn without touching the shared prize seed.
            let mut seed = self.rng.next_u32();
            for _ in 0..9999 {
                let p = generate_prize(&mut seed, &self.settings, false);
                let skip = Prize::from_id(p.clamp(0, 255) as u8).is_none_or(|p| {
                    matches!(
                        p,
                        Prize::EngineShutdown
                            | Prize::Shields
                            | Prize::Super
                            | Prize::Multiprize
                            | Prize::Warp
                    )
                });
                if skip {
                    continue;
                }
                let bounty = self.ships[index].bounty;
                self.apply_prize(index, p, events);
                self.ships[index].bounty = bounty;
                return;
            }
            return;
        }
        let negative = id < 0;
        let Some(prize) = Prize::from_id(id.unsigned_abs().min(255) as u8) else {
            return;
        };
        if !negative {
            self.ships[index].bounty += 1;
        }
        let tick = self.tick;
        let s = self.settings.ships[self.ships[index].kind as usize];
        let ship = &mut self.ships[index];

        /// Moves a stat by `step` within `lo..=hi`.
        fn adjust(v: &mut u32, step: u32, lo: u32, hi: u32, negative: bool) {
            *v = if negative {
                v.saturating_sub(step).max(lo)
            } else {
                (*v + step).min(hi.max(lo))
            };
        }
        fn count(v: &mut u8, max: u8, negative: bool) {
            *v = if negative {
                v.saturating_sub(1)
            } else {
                v.saturating_add(1).min(max)
            };
        }
        let mut full_charge = None;
        let mut capability = |ship: &mut Ship, bit: u8, status: u8| {
            if negative {
                if !ship.has(bit) && status == 0 {
                    full_charge = Some(true);
                }
                ship.caps &= !bit;
            } else if status > 0 {
                ship.caps |= bit;
            } else {
                full_charge = Some(false);
            }
        };

        match prize {
            Prize::QuickCharge => adjust(
                &mut ship.stats.recharge,
                s.upgrade_recharge,
                s.initial_recharge,
                s.maximum_recharge,
                negative,
            ),
            Prize::Energy => adjust(
                &mut ship.stats.max_energy,
                s.upgrade_energy,
                s.initial_energy,
                s.maximum_energy,
                negative,
            ),
            Prize::Rotation => adjust(
                &mut ship.stats.rotation,
                s.upgrade_rotation,
                s.initial_rotation,
                s.maximum_rotation,
                negative,
            ),
            Prize::Thruster => adjust(
                &mut ship.stats.thrust,
                s.upgrade_thrust,
                s.initial_thrust,
                s.maximum_thrust,
                negative,
            ),
            Prize::TopSpeed => adjust(
                &mut ship.stats.speed,
                s.upgrade_speed,
                s.initial_speed,
                s.maximum_speed,
                negative,
            ),
            Prize::Stealth => capability(ship, cap::STEALTH, s.stealth_status),
            Prize::Cloak => capability(ship, cap::CLOAK, s.cloak_status),
            Prize::XRadar => capability(ship, cap::XRADAR, s.xradar_status),
            Prize::Antiwarp => capability(ship, cap::ANTIWARP, s.anti_warp_status),
            Prize::BouncingBullets => capability(ship, cap::BOUNCING_BULLETS, 1),
            Prize::Multifire => capability(ship, cap::MULTIFIRE, 1),
            Prize::Proximity => capability(ship, cap::PROXIMITY, 1),
            Prize::Guns => {
                ship.guns = if negative {
                    ship.guns.saturating_sub(1).max(s.initial_guns)
                } else {
                    (ship.guns + 1).min(s.max_guns)
                }
            }
            Prize::Bombs => {
                ship.bombs = if negative {
                    ship.bombs.saturating_sub(1).max(s.initial_bombs)
                } else {
                    (ship.bombs + 1).min(s.max_bombs)
                }
            }
            Prize::Shrapnel => {
                ship.shrapnel = if negative {
                    ship.shrapnel.saturating_sub(s.shrapnel_rate)
                } else {
                    ship.shrapnel
                        .saturating_add(s.shrapnel_rate)
                        .min(s.shrapnel_max)
                }
            }
            Prize::Repel => count(&mut ship.items.repels, s.repel_max, negative),
            Prize::Burst => count(&mut ship.items.bursts, s.burst_max, negative),
            Prize::Decoy => count(&mut ship.items.decoys, s.decoy_max, negative),
            Prize::Thor => count(&mut ship.items.thors, s.thor_max, negative),
            Prize::Brick => count(&mut ship.items.bricks, s.brick_max, negative),
            Prize::Rocket => count(&mut ship.items.rockets, s.rocket_max, negative),
            Prize::Portal => count(&mut ship.items.portals, s.portal_max, negative),
            Prize::FullCharge => full_charge = Some(negative),
            Prize::EngineShutdown => {
                let t = self.settings.engine_shutdown_time * if negative { 3 } else { 1 };
                ship.shutdown_end_tick = tick.wrapping_add(t);
            }
            Prize::Shields => ship.shield_end_tick = tick.wrapping_add(s.shields_time),
            Prize::Super => {
                let t = if s.super_time > 0 {
                    self.rng.next_u32() % s.super_time
                } else {
                    0
                };
                let end = tick.wrapping_add(t);
                if !active(tick, ship.super_end_tick) || tick_after(end, ship.super_end_tick) {
                    ship.super_end_tick = end;
                }
            }
            Prize::Warp => {
                if negative {
                    full_charge = Some(false);
                } else {
                    let mut moved = ship.clone();
                    self.place_at_spawn(&mut moved);
                    self.ships[index] = moved;
                }
            }
            Prize::Multiprize => {
                if !negative {
                    let saved = self.prize_seed;
                    let mut given = 0;
                    for _ in 0..9999 {
                        if given >= self.settings.multi_prize_count {
                            break;
                        }
                        let p = generate_prize(&mut self.prize_seed, &self.settings, false);
                        if p <= 0
                            || Prize::from_id(p as u8).is_none_or(Prize::excluded_from_batches)
                        {
                            continue;
                        }
                        let bounty = self.ships[index].bounty;
                        self.apply_prize(index, p, events);
                        self.ships[index].bounty = bounty;
                        given += 1;
                    }
                    self.prize_seed = saved;
                }
            }
        }

        if let Some(bad) = full_charge {
            let ship = &mut self.ships[index];
            ship.energy = if bad {
                ENERGY_SCALE
            } else {
                ship.stats.max_energy as i32 * ENERGY_SCALE
            };
        }
        events.push(Event::PrizeTaken {
            ship: ship_id,
            prize: id,
        });
    }

    /// Resets a ship to its spawn loadout: initial stats and items, status-2
    /// capabilities, then `InitialBounty` random prizes. The shared prize seed is left
    /// untouched so greens stay in sync between clients.
    pub fn reset_loadout(&mut self, index: usize) {
        let Some(ship) = self.ships.get_mut(index) else {
            return;
        };
        let s = self.settings.ships[ship.kind as usize];
        ship.stats = ShipStats::initial(&s);
        ship.items = Items::initial(&s);
        ship.guns = s.initial_guns.min(s.max_guns);
        ship.bombs = s.initial_bombs.min(s.max_bombs);
        ship.shrapnel = 0;
        ship.multifire = false;
        ship.toggles = 0;
        ship.bounty = 0;
        let now = self.tick;
        for t in [
            &mut ship.super_end_tick,
            &mut ship.shield_end_tick,
            &mut ship.shutdown_end_tick,
            &mut ship.rocket_end_tick,
            &mut ship.emp_end_tick,
            &mut ship.repelled_end_tick,
            &mut ship.fake_antiwarp_end_tick,
            &mut ship.next_gun_tick,
            &mut ship.next_bomb_tick,
            &mut ship.next_repel_tick,
        ] {
            *t = now;
        }
        ship.portal = None;
        ship.caps = 0;
        for (status, bit) in [
            (s.stealth_status, cap::STEALTH),
            (s.cloak_status, cap::CLOAK),
            (s.xradar_status, cap::XRADAR),
            (s.anti_warp_status, cap::ANTIWARP),
        ] {
            if status == 2 {
                ship.caps |= bit;
            }
        }

        if self.settings.prize_weight_total() > 0 {
            let saved = self.prize_seed;
            let mut scratch = Vec::new();
            let mut given = 0;
            for _ in 0..9999 {
                if given >= s.initial_bounty {
                    break;
                }
                let p = generate_prize(&mut self.prize_seed, &self.settings, false);
                let Some(prize) = Prize::from_id(p.clamp(0, 255) as u8) else {
                    continue;
                };
                if prize.excluded_from_batches() && prize != Prize::Multiprize {
                    continue;
                }
                self.apply_prize(index, p, &mut scratch);
                given += 1;
            }
            self.prize_seed = saved;
        }
        let ship = &mut self.ships[index];
        ship.energy = ship.stats.max_energy as i32 * ENERGY_SCALE;
        ship.bounty = s.initial_bounty;
    }

    /// Greens on the map.
    pub fn greens(&self) -> &[Green] {
        &self.greens
    }

    /// Restarts the green timer; done when the server sends a new seed so every
    /// client spawns greens on the same ticks.
    pub fn reset_green_timer(&mut self) {
        self.green_ticks = 0;
    }

    /// Removes the green on tile (x, y), if any (another player took it).
    pub fn remove_green_at(&mut self, x: u16, y: u16) -> bool {
        match self.greens.iter().position(|g| g.x == x && g.y == y) {
            Some(i) => {
                self.greens.swap_remove(i);
                true
            }
            None => false,
        }
    }

    /// Leaves prize `prize` where a ship died (tile coordinates), for `DeathPrizeTime`.
    pub fn spawn_death_green(&mut self, x: u16, y: u16, prize: i32) {
        if self.greens.len() >= MAX_GREENS || prize == 0 {
            return;
        }
        self.greens.push(Green {
            x: x.min(MAP_TILES as u16 - 1),
            y: y.min(MAP_TILES as u16 - 1),
            prize,
            end_tick: self.tick.wrapping_add(self.settings.death_prize_time),
        });
    }

    /// Pickups, expiry and spawning of greens; runs once per tick.
    pub(crate) fn step_greens(&mut self, events: &mut Vec<Event>) {
        let tick = self.tick;

        // Pickups: any ship touching a green removes it; the local ship gets the prize.
        let mut taken = Vec::new();
        for (si, ship) in self.ships.iter().enumerate() {
            if !ship.is_alive() {
                continue;
            }
            let r = self.settings.ships[ship.kind as usize].radius_pixels();
            let p = ship.pos.pixels();
            for (gi, g) in self.greens.iter().enumerate() {
                let (gx, gy) = (g.x as i32 * 16, g.y as i32 * 16);
                if p.x + r >= gx && p.x - r < gx + 16 && p.y + r >= gy && p.y - r < gy + 16 {
                    taken.push((si, gi));
                }
            }
        }
        let mut removed = vec![false; self.greens.len()];
        for (si, gi) in taken {
            if removed[gi] {
                continue;
            }
            removed[gi] = true;
            if self.ships[si].takes_damage {
                let g = self.greens[gi];
                events.push(Event::GreenTaken {
                    ship: self.ships[si].id,
                    x: g.x,
                    y: g.y,
                    prize: g.prize,
                });
                self.apply_prize(si, g.prize, events);
            }
        }
        let mut i = 0;
        self.greens.retain(|g| {
            let keep = !removed[i] && !tick_after(tick, g.end_tick);
            i += 1;
            keep
        });

        // Spawning.
        if self.prize_seed == 0 {
            return;
        }
        self.green_ticks += 1;
        if self.green_ticks < self.settings.prize_delay.max(1) {
            return;
        }
        self.green_ticks = 0;
        let a = &self.settings;
        let players = self.player_count.unwrap_or(self.ships.len() as u32);
        let max_greens = ((a.prize_factor * players) / 1000).min(MAX_GREENS as u32) as usize;
        let extent = (a.minimum_virtual + a.upgrade_virtual * players).clamp(3, MAP_TILES as u32);
        let exist_span = a.prize_max_exist.saturating_sub(a.prize_min_exist);
        let min_exist = a.prize_min_exist;
        for _ in 0..a.prize_hide_count {
            let mut rng = VieRng::new(self.prize_seed);
            let x = rng.next_u32() % (extent - 2) + 1 + (MAP_TILES as u32 - extent) / 2;
            let y = rng.next_u32() % (extent - 2) + 1 + (MAP_TILES as u32 - extent) / 2;
            self.prize_seed = rng.seed as u32;
            let prize = generate_prize(&mut self.prize_seed, &self.settings, true);
            let mut rng = VieRng::new(self.prize_seed);
            let duration = rng.next_u32() % (exist_span + 1) + min_exist;
            self.prize_seed = rng.seed as u32;
            if prize != 0 && self.greens.len() < max_greens && self.map.get(x as i32, y as i32) == 0
            {
                self.greens.push(Green {
                    x: x as u16,
                    y: y as u16,
                    prize,
                    end_tick: tick.wrapping_add(duration),
                });
            }
        }
    }
}
