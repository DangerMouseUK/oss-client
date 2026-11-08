use crate::fixed::{ENERGY_SCALE, ROTATION_UNITS};
use crate::*;

/// Stock settings without spawn prizes or random damage, so stats are predictable.
fn bare() -> ArenaSettings {
    let mut s = ArenaSettings::default();
    for ship in &mut s.ships {
        ship.initial_bounty = 0;
    }
    s.exact_damage = true;
    s
}

fn world_with(cells: Option<&[u8]>, settings: ArenaSettings) -> World {
    let map = cells.map(TileMap::from_cells).unwrap_or_default();
    let mut w = World::new(map, settings, 1);
    w.prize_seed = 0; // no greens unless a test wants them
    w
}

fn open_world() -> World {
    world_with(None, bare())
}

/// Gives ship `i` level-1 guns and bombs with multifire capability.
fn arm(w: &mut World, i: usize) {
    let s = &mut w.ships[i];
    s.guns = 1;
    s.bombs = 1;
    s.caps |= cap::MULTIFIRE;
}

/// Puts ship 0 at a pixel position, facing `direction` (0..40), at rest.
fn place(w: &mut World, x: i32, y: i32, direction: u32) {
    let s = &mut w.ships[0];
    s.pos = Pos::from_pixels(x, y);
    s.vel = Vel::default();
    s.rotation = direction * 1000;
}

fn hold(input: ShipInput, ticks: u32, w: &mut World) -> Vec<Event> {
    let mut all = Vec::new();
    for _ in 0..ticks {
        all.extend(w.step(&[input]));
    }
    all
}

fn key(f: impl FnOnce(&mut ShipInput)) -> ShipInput {
    let mut i = ShipInput::default();
    f(&mut i);
    i
}

fn wall_column(x: usize) -> Vec<u8> {
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    for y in 0..MAP_TILES {
        cells[y * MAP_TILES + x] = 1;
    }
    cells
}

// --- Flight -----------------------------------------------------------------------------

#[test]
fn thrust_accelerates_up_to_max_speed() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 10); // facing right
    hold(key(|i| i.forward = true), 1, &mut w);
    // One tick of thrust 15 = 15 px/10s.
    assert_eq!(w.ships[0].vel, Vel::from_wire(15, 0));
    hold(key(|i| i.forward = true), 1000, &mut w);
    assert_eq!(w.ships[0].vel, Vel::from_wire(2010, 0));
}

#[test]
fn afterburner_raises_speed_and_costs_energy() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 20); // facing down
    hold(
        key(|i| (i.forward, i.afterburner) = (true, true)),
        300,
        &mut w,
    );
    let s = &w.ships[0];
    assert!(
        s.vel.y > 2010 * VEL_SCALE,
        "afterburner should exceed normal speed"
    );
    assert!(s.vel.y <= 3250 * VEL_SCALE);
    assert!(s.energy < 1000 * ENERGY_SCALE);
    // Releasing afterburner snaps back to normal top speed.
    hold(key(|i| i.forward = true), 1, &mut w);
    assert_eq!(w.ships[0].vel.y, 2010 * VEL_SCALE);
}

#[test]
fn rotation_speed_matches_settings() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 0);
    // InitialRotation 200 = half a turn per second.
    hold(key(|i| i.right = true), 100, &mut w);
    assert_eq!(w.ships[0].rotation, ROTATION_UNITS / 2);
    hold(key(|i| i.left = true), 150, &mut w);
    assert_eq!(w.ships[0].rotation, ROTATION_UNITS * 3 / 4);
}

#[test]
fn energy_recharges_to_max() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    w.ships[0].energy = 0;
    // 400 per 10 s = 40 energy per second.
    hold(ShipInput::default(), 100, &mut w);
    assert_eq!(w.ships[0].energy_points(), 40);
    hold(ShipInput::default(), 5000, &mut w);
    assert_eq!(w.ships[0].energy_points(), 1000);
}

#[test]
fn wall_bounce_reverses_and_damps() {
    let cells = wall_column(520);
    let mut w = world_with(Some(&cells), bare());
    w.add_ship(0, 0, 0);
    place(&mut w, 516 * 16, 512 * 16, 10);
    w.ships[0].vel = Vel::from_wire(1900, 300);
    let mut bounced = false;
    for _ in 0..200 {
        if w.step(&[])
            .iter()
            .any(|e| matches!(e, Event::WallBounce { .. }))
        {
            bounced = true;
            break;
        }
    }
    assert!(bounced);
    let v = w.ships[0].vel;
    // x reversed and multiplied by 16/22, y damped by 16/22.
    assert_eq!(v.x, -(1900 * VEL_SCALE * 16 / 22));
    assert_eq!(v.y, 300 * VEL_SCALE * 16 / 22);
    assert!(w.ships[0].pos.pixels().x + 14 < 520 * 16);
}

#[test]
fn ship_never_tunnels_through_walls() {
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    for i in 500..=524 {
        for (x, y) in [(i, 500), (i, 524), (500, i), (524, i)] {
            cells[y * MAP_TILES + x] = 1;
        }
    }
    let mut w = world_with(Some(&cells), bare());
    w.spawn_radius = 5;
    w.add_ship(0, 0, 0);
    for t in 0..5000 {
        let i = key(|i| (i.forward, i.afterburner, i.right) = (true, true, t % 300 < 40));
        w.step(&[i]);
        let p = w.ships[0].pos.pixels();
        assert!(
            p.x > 500 * 16 + 16 && p.x < 524 * 16,
            "escaped at tick {t}: {p:?}"
        );
        assert!(
            p.y > 500 * 16 + 16 && p.y < 524 * 16,
            "escaped at tick {t}: {p:?}"
        );
    }
}

#[test]
fn rocket_overrides_thrust_and_speed() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 10);
    w.ships[0].items.rockets = 1;
    hold(key(|i| i.rocket = true), 1, &mut w);
    assert_eq!(w.ships[0].items.rockets, 0);
    hold(ShipInput::default(), 600, &mut w);
    // RocketSpeed 5500 is well above the normal top speed, with no keys held.
    assert_eq!(w.ships[0].vel.x, 5500 * VEL_SCALE);
    hold(ShipInput::default(), 500, &mut w);
    assert!(w.ships[0].vel.x <= 2010 * VEL_SCALE);
}

#[test]
fn engine_shutdown_stops_thrust() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 10);
    let mut ev = Vec::new();
    w.apply_prize(0, Prize::EngineShutdown as i32, &mut ev);
    hold(key(|i| i.forward = true), 100, &mut w);
    assert_eq!(w.ships[0].vel, Vel::default());
    hold(key(|i| i.forward = true), 700, &mut w);
    assert!(w.ships[0].vel.x > 0);
}

// --- Prizes -------------------------------------------------------------------------------

#[test]
fn prize_generation_follows_weights() {
    let mut s = bare();
    s.prize_weights = [0; PRIZE_COUNT];
    s.prize_weights[Prize::Guns as usize - 1] = 5;
    s.prize_negative_factor = 0;
    let mut seed = 12345;
    for _ in 0..50 {
        assert_eq!(generate_prize(&mut seed, &s, true), Prize::Guns as i32);
    }
    s.prize_weights = [0; PRIZE_COUNT];
    assert_eq!(generate_prize(&mut seed, &s, true), 0);
    // Every prize gets picked eventually with stock weights, and some are negative.
    let s = ArenaSettings::default();
    let mut seen = std::collections::HashSet::new();
    let mut negatives = 0;
    for _ in 0..20_000 {
        let p = generate_prize(&mut seed, &s, true);
        seen.insert(p.abs());
        negatives += (p < 0) as u32;
    }
    assert_eq!(seen.len(), 27); // Glue (engine shutdown) has weight 0
    assert!(negatives > 0);
}

#[test]
fn prizes_upgrade_within_limits() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    let mut ev = Vec::new();
    for _ in 0..10 {
        w.apply_prize(0, Prize::Guns as i32, &mut ev);
        w.apply_prize(0, Prize::TopSpeed as i32, &mut ev);
        w.apply_prize(0, Prize::Repel as i32, &mut ev);
    }
    let s = &w.ships[0];
    assert_eq!((s.guns, s.stats.speed, s.items.repels), (3, 3250, 3));
    assert_eq!(s.bounty, 30);
    w.apply_prize(0, -(Prize::Guns as i32), &mut ev);
    assert_eq!(w.ships[0].guns, 2);
    w.apply_prize(0, -(Prize::FullCharge as i32), &mut ev);
    assert_eq!(w.ships[0].energy, ENERGY_SCALE);
    // Cloak is status 0 for stock ships, so the prize becomes a full charge instead.
    w.apply_prize(0, Prize::Cloak as i32, &mut ev);
    assert!(!w.ships[0].has(cap::CLOAK));
    assert_eq!(w.ships[0].energy_points(), 1000);
    w.apply_prize(0, Prize::Stealth as i32, &mut ev);
    assert!(w.ships[0].has(cap::STEALTH));
}

#[test]
fn spawn_prizes_come_from_initial_bounty() {
    let mut w = world_with(None, ArenaSettings::default());
    w.prize_seed = 777;
    w.add_ship(0, 0, 0);
    let s = &w.ships[0];
    assert_eq!(s.bounty, 500);
    assert_eq!(
        w.prize_seed, 777,
        "spawn prizes must not disturb the shared seed"
    );
    // 500 stock prizes max out most upgrades.
    assert!(s.guns >= 2 && s.bombs >= 1, "{s:?}");
    assert!(s.stats.speed > 2010 && s.stats.max_energy > 1000);
    assert_eq!(s.energy_points(), s.stats.max_energy as i32);
    // Same seed, same loadout.
    let mut w2 = world_with(None, ArenaSettings::default());
    w2.prize_seed = 777;
    w2.add_ship(0, 0, 0);
    assert_eq!(w2.ships[0].stats, w.ships[0].stats);
    assert_eq!(w2.ships[0].items, w.ships[0].items);
}

#[test]
fn greens_spawn_near_centre_and_can_be_collected() {
    let mut w = world_with(None, bare());
    w.prize_seed = 4242;
    w.add_ship(0, 0, 0);
    place(&mut w, 0, 0, 0);
    hold(ShipInput::default(), 700, &mut w);
    let greens = w.greens().to_vec();
    assert!(!greens.is_empty());
    // One player: a 16-tile square around the centre, at most 10 greens.
    assert!(greens.len() <= 10);
    for g in &greens {
        assert!(
            (504..=520).contains(&g.x) && (504..=520).contains(&g.y),
            "{g:?}"
        );
    }
    let g = greens[0];
    place(&mut w, g.x as i32 * 16 + 8, g.y as i32 * 16 + 8, 0);
    let ev = w.step(&[]);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::PrizeTaken { ship: 0, .. }))
    );
    assert_eq!(w.greens().len(), greens.len() - 1);
}

// --- Weapons ------------------------------------------------------------------------------

#[test]
fn guns_fire_with_delay_and_cost() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    arm(&mut w, 0);
    place(&mut w, 8000, 8000, 0);
    let ev = hold(key(|i| i.gun = true), 100, &mut w);
    let shots = ev
        .iter()
        .filter(|e| matches!(e, Event::FiredGun { .. }))
        .count();
    // BulletFireDelay 25: fires on tick 1, then every 26 ticks (strictly-after rule).
    assert_eq!(shots, 4);
    assert!(w.projectiles[0].vel.y < 0, "bullet should travel up");
}

#[test]
fn multifire_spawns_three_bullets() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    arm(&mut w, 0);
    place(&mut w, 8000, 8000, 0);
    hold(key(|i| i.toggle_multifire = true), 1, &mut w);
    assert!(w.ships[0].multifire);
    hold(key(|i| i.gun = true), 1, &mut w);
    assert_eq!(w.projectiles.len(), 3);
}

#[test]
fn bullet_kills_enemy_and_enemy_respawns() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    w.add_ship(1, 0, 1);
    arm(&mut w, 0);
    place(&mut w, 8000, 8000, 10); // facing right
    w.ships[1].pos = Pos::from_pixels(8100, 8000);
    w.ships[1].energy = 100 * ENERGY_SCALE;
    let ev = hold(key(|i| i.gun = true), 80, &mut w);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::ShipHit {
            ship: 1,
            shooter: 0,
            damage: 200
        }
    )));
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::ShipKilled {
            ship: 1,
            killer: 0,
            ..
        }
    )));
    assert!(!w.ships[1].is_alive());
    assert_eq!(w.ships[0].bounty, 6);
    let ev = hold(ShipInput::default(), 250, &mut w);
    assert!(ev.iter().any(|e| matches!(e, Event::Respawned { ship: 1 })));
    assert!(w.ships[1].is_alive());
}

#[test]
fn bomb_hits_wall_explodes_and_sprays_shrapnel() {
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    for x in 500..530 {
        cells[480 * MAP_TILES + x] = 1;
    }
    let mut s = bare();
    s.shrapnel_random = false;
    let mut w = world_with(Some(&cells), s);
    w.add_ship(0, 0, 0);
    arm(&mut w, 0);
    w.ships[0].shrapnel = 8;
    place(&mut w, 512 * 16 + 8, 512 * 16 + 8, 0);
    let ev = hold(key(|i| i.bomb = true), 1, &mut w);
    assert!(ev.iter().any(|e| matches!(e, Event::FiredBomb { .. })));
    // Recoil pushes the ship backwards (down).
    assert!(w.ships[0].vel.y > 0);
    let mut exploded = false;
    for _ in 0..400 {
        let ev = w.step(&[]);
        if ev.iter().any(|e| {
            matches!(
                e,
                Event::Explosion {
                    kind: ProjectileKind::Bomb,
                    ..
                }
            )
        }) {
            exploded = true;
            break;
        }
    }
    assert!(exploded);
    let shrap: Vec<_> = w
        .projectiles
        .iter()
        .filter(|p| p.kind == ProjectileKind::Shrapnel)
        .collect();
    // Eight evenly spread pieces; those aimed into the wall die at once (inactive
    // shrapnel vanishes on walls), the rest fly away from it.
    assert!((5..=8).contains(&shrap.len()), "{}", shrap.len());
    assert!(shrap.iter().any(|p| p.vel.x == 0 && p.vel.y > 0));
    assert!(shrap.iter().all(|p| p.vel.y >= 0));
    hold(ShipInput::default(), 600, &mut w);
    assert!(w.projectiles.is_empty());
}

#[test]
fn proximity_bomb_triggers_near_enemy() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    w.add_ship(1, 0, 1);
    arm(&mut w, 0);
    w.ships[0].caps |= cap::PROXIMITY;
    place(&mut w, 8000, 8000, 10);
    // Enemy just above the bomb's path: never touched directly, but within prox range.
    w.ships[1].pos = Pos::from_pixels(8300, 8000 - 40);
    let mut ev = hold(key(|i| i.bomb = true), 1, &mut w);
    ev.extend(hold(ShipInput::default(), 400, &mut w));
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Explosion {
            kind: ProjectileKind::Bomb,
            ..
        }
    )));
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::ShipHit { ship: 1, .. }))
    );
}

#[test]
fn repel_pushes_enemy_bullets_and_ships() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    w.add_ship(1, 0, 1);
    arm(&mut w, 1);
    place(&mut w, 8000, 8000, 0);
    w.ships[0].items.repels = 1;
    w.ships[1].pos = Pos::from_pixels(8100, 8000);
    w.ships[1].rotation = 30_000; // facing left, at ship 0
    w.step(&[ShipInput::default(), key(|i| i.gun = true)]);
    assert_eq!(w.projectiles.len(), 1);
    let ev = w.step(&[key(|i| i.repel = true)]);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Repelled { ship: 0, .. }))
    );
    assert!(w.projectiles[0].vel.x > 0, "enemy bullet pushed back");
    assert!(w.ships[1].vel.x > 0, "enemy ship pushed away");
    assert_eq!(w.ships[0].items.repels, 0);
}

#[test]
fn burst_arms_on_first_bounce() {
    let cells = wall_column(520);
    let mut w = world_with(Some(&cells), bare());
    w.add_ship(0, 0, 0);
    w.ships[0].items.bursts = 1;
    place(&mut w, 516 * 16, 512 * 16, 0);
    w.step(&[key(|i| i.burst = true)]);
    assert_eq!(w.projectiles.len(), 24);
    assert!(w.projectiles.iter().all(|p| !p.armed));
    hold(ShipInput::default(), 40, &mut w);
    assert!(w.projectiles.iter().any(|p| p.armed));
}

#[test]
fn thor_passes_through_walls() {
    let cells = wall_column(520);
    let mut w = world_with(Some(&cells), bare());
    w.add_ship(0, 0, 0);
    w.ships[0].items.thors = 1;
    place(&mut w, 516 * 16, 512 * 16, 10);
    w.step(&[key(|i| i.thor = true)]);
    hold(ShipInput::default(), 100, &mut w);
    let t = w
        .projectiles
        .iter()
        .find(|p| p.kind == ProjectileKind::Thor)
        .expect("thor alive");
    assert!(t.pos.pixels().x > 521 * 16);
}

#[test]
fn portal_and_warp() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 3000, 3000, 0);
    w.ships[0].items.portals = 1;
    let ev = w.step(&[key(|i| i.portal = true)]);
    assert!(ev.iter().any(|e| matches!(e, Event::PortalPlaced { .. })));
    place(&mut w, 9000, 9000, 0);
    let ev = w.step(&[key(|i| i.warp = true)]);
    assert!(ev.iter().any(|e| matches!(e, Event::Warped { .. })));
    assert_eq!(w.ships[0].pos.pixels(), Vec2::new(3000, 3000));
    // Without a portal, warping needs full energy and drains it.
    hold(ShipInput::default(), 60, &mut w);
    w.ships[0].energy = 500 * ENERGY_SCALE;
    let ev = w.step(&[key(|i| i.warp = true)]);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::WarpNeedsEnergy { .. }))
    );
    w.ships[0].energy = 1000 * ENERGY_SCALE;
    let ev = w.step(&[key(|i| i.warp = true)]);
    assert!(ev.iter().any(|e| matches!(e, Event::Warped { .. })));
    assert!(w.ships[0].energy_points() <= 1);
}

#[test]
fn enemy_antiwarp_blocks_warp() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    w.add_ship(1, 0, 1);
    place(&mut w, 8000, 8000, 0);
    w.ships[1].pos = Pos::from_pixels(8200, 8000);
    w.ships[1].caps |= cap::ANTIWARP;
    w.step(&[ShipInput::default(), key(|i| i.toggle_antiwarp = true)]);
    assert!(w.ships[1].is_on(toggle::ANTIWARP));
    let ev = w.step(&[key(|i| i.warp = true)]);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Antiwarped { ship: 0 }))
    );
}

#[test]
fn toggles_drain_energy_and_switch_off_when_empty() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    w.ships[0].caps |= cap::STEALTH;
    w.ships[0].stats.recharge = 0;
    w.step(&[key(|i| i.toggle_stealth = true)]);
    assert!(w.ships[0].is_on(toggle::STEALTH));
    let before = w.ships[0].energy;
    hold(ShipInput::default(), 100, &mut w);
    // StealthEnergy 333 per 10 s = 33.3 energy per second.
    assert_eq!(before - w.ships[0].energy, 333 * 100);
    w.ships[0].energy = 100;
    let ev = hold(ShipInput::default(), 1, &mut w);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Toggled { on: false, .. }))
    );
    assert!(!w.ships[0].is_on(toggle::STEALTH));
}

#[test]
fn bricks_block_enemies_but_not_team() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    w.add_ship(1, 0, 1);
    place(&mut w, 512 * 16 + 8, 500 * 16 + 8, 0);
    w.drop_brick_line(512, 505, 0, 0);
    assert!(w.bricks().count() >= 21);
    // Friendly ship flies straight through its own team's bricks.
    w.ships[0].vel = Vel::from_wire(0, 2000);
    hold(ShipInput::default(), 150, &mut w);
    assert!(w.ships[0].pos.pixels().y > 506 * 16);
    // An enemy bounces.
    w.ships[1].pos = Pos::from_pixels(512 * 16 + 8, 500 * 16 + 8);
    w.ships[1].vel = Vel::from_wire(0, 2000);
    for _ in 0..150 {
        w.step(&[]);
        assert!(w.ships[1].pos.pixels().y < 505 * 16);
    }
    hold(ShipInput::default(), 12_000, &mut w);
    assert_eq!(w.bricks().count(), 0);
    assert_eq!(w.map.get(512, 505), 0);
}

#[test]
fn emp_bomb_stops_recharge() {
    let mut s = bare();
    s.ships[0].emp_bomb = true;
    let mut w = world_with(None, s);
    w.add_ship(0, 0, 0);
    w.add_ship(1, 1, 1);
    arm(&mut w, 0);
    place(&mut w, 8000, 8000, 10);
    w.ships[1].pos = Pos::from_pixels(8200, 8000);
    let mut ev = hold(key(|i| i.bomb = true), 1, &mut w);
    ev.extend(hold(ShipInput::default(), 150, &mut w));
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Explosion { emp: true, .. }))
    );
    let e = w.ships[1].energy;
    hold(ShipInput::default(), 10, &mut w);
    assert_eq!(w.ships[1].energy, e, "EMP stops recharge");
}

#[test]
fn deterministic_replay() {
    let run = || {
        let mut w = World::new(TileMap::empty(), ArenaSettings::sandbox(), 42);
        w.add_ship(0, 3, 0);
        w.add_ship(1, 5, 1);
        for t in 0u32..3000 {
            let a = key(|i| {
                i.forward = t % 7 < 4;
                i.right = t % 50 < 10;
                i.gun = t % 3 == 0;
                i.bomb = t % 200 == 0;
                i.afterburner = t % 400 < 100;
                i.burst = t % 700 == 5;
                i.repel = t % 900 == 3;
            });
            let b = key(|i| {
                i.backward = t % 5 < 2;
                i.left = t % 30 < 8;
                i.mine = t % 500 == 1;
                i.thor = t % 800 == 2;
            });
            w.step(&[a, b]);
        }
        (w.ships.clone(), w.projectiles.clone(), w.greens().to_vec())
    };
    assert_eq!(run(), run());
}

// --- Doors --------------------------------------------------------------------------------

#[test]
fn doors_follow_seed_bits() {
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    cells[100 * MAP_TILES + 100] = 162;
    cells[100 * MAP_TILES + 101] = 169;
    let mut settings = bare();
    settings.door_mode = 0b0000_0001;
    let mut w = world_with(Some(&cells), settings);
    assert_eq!(w.doors().len(), 2);
    w.step(&[]);
    assert_eq!(w.map.get(100, 100), 162); // closed (bit 0)
    assert_eq!(w.map.get(101, 100), tile::FLAG); // open (bit 7 clear)
    assert!(w.map.is_solid(100, 100) && !w.map.is_solid(101, 100));
    w.apply_door_seed(0x80);
    assert_eq!(w.map.get(100, 100), tile::FLAG);
    assert_eq!(w.map.get(101, 100), 169);
}

#[test]
fn closing_door_moves_ship_out() {
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    cells[100 * MAP_TILES + 100] = 162;
    let mut w = world_with(Some(&cells), bare());
    w.add_ship(0, 0, 0);
    w.apply_door_seed(0);
    w.ships[0].pos = Pos::from_pixels(100 * 16 + 8, 100 * 16 + 8);
    w.apply_door_seed(1);
    let p = w.ships[0].pos.pixels();
    assert!(!w.map.box_collides(p.x, p.y, 14));
}

fn all_doors_world(mode: i32, delay: u32) -> World {
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    for k in 0..8 {
        cells[100 * MAP_TILES + 100 + k] = 162 + k as u8;
    }
    let mut settings = bare();
    settings.door_mode = mode;
    settings.door_delay = delay;
    world_with(Some(&cells), settings)
}

fn door_tiles(w: &World) -> Vec<u8> {
    (0..8).map(|k| w.map.get(100 + k, 100)).collect()
}

#[test]
fn synced_doors_match_between_early_and_late_joiners() {
    for mode in [-1, -2] {
        // Both learn the same seed, current at tick 1000; one joined 500 ticks later.
        let mut early = all_doors_world(mode, 100);
        early.tick = 1000;
        early.sync_doors(0x1234_5678, 1000);
        let mut late = all_doors_world(mode, 100);
        late.tick = 1500;
        assert!(late.sync_doors(0x1234_5678, 1000).is_some());
        for _ in 0..500 {
            early.step(&[]);
        }
        assert_eq!(door_tiles(&early), door_tiles(&late), "mode {mode}");
        for _ in 0..1234 {
            early.step(&[]);
            late.step(&[]);
            assert_eq!(door_tiles(&early), door_tiles(&late), "mode {mode}");
        }
    }
}

#[test]
fn door_changes_come_from_the_shared_generator() {
    let mut w = all_doors_world(-2, 50);
    w.tick = 70;
    w.sync_doors(99, 70);
    let mut rng = VieRng::new(99);
    let mut changes = Vec::new();
    for _ in 0..200 {
        for e in w.step(&[]) {
            if let Event::DoorsChanged { seed } = e {
                changes.push((w.tick, seed));
            }
        }
    }
    let expected: Vec<_> = (1..=4)
        .map(|i| (70 + 50 * i, rng.next_u32() as u8))
        .collect();
    assert_eq!(changes, expected);
}

#[test]
fn bad_door_timestamp_does_not_stall() {
    let mut w = all_doors_world(-1, 1);
    w.tick = 0x7fff_0000;
    let start = std::time::Instant::now();
    assert!(w.sync_doors(5, 0).is_some());
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    // A timestamp in the future just waits.
    let mut w = all_doors_world(-1, 10);
    w.tick = 100;
    assert!(w.sync_doors(5, 500).is_none());
}

#[test]
fn death_greens_and_greens_taken_elsewhere() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 100 * 16, 100 * 16, 0);
    w.spawn_death_green(512, 512, 3);
    w.spawn_death_green(600, 600, 4);
    assert_eq!(w.greens().len(), 2);
    assert!(w.remove_green_at(600, 600));
    assert!(!w.remove_green_at(600, 600));
    // The local ship picks up the death green and reports where it was.
    place(&mut w, 512 * 16 + 8, 512 * 16 + 8, 0);
    let ev = w.step(&[]);
    assert!(ev.contains(&Event::GreenTaken {
        ship: 0,
        x: 512,
        y: 512,
        prize: 3
    }));
    assert!(w.greens().is_empty());
    // Unpicked death greens expire after DeathPrizeTime.
    w.spawn_death_green(300, 300, 5);
    hold(
        ShipInput::default(),
        w.settings.death_prize_time + 1,
        &mut w,
    );
    assert!(w.greens().is_empty());
}
