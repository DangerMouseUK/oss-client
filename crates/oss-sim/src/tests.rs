use crate::fixed::{ENERGY_SCALE, ROTATION_UNITS};
use crate::*;

fn open_world() -> World {
    World::new(TileMap::empty(), ArenaSettings::sandbox(), 1)
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

#[test]
fn thrust_accelerates_up_to_max_speed() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 10); // facing right
    hold(
        ShipInput {
            forward: true,
            ..Default::default()
        },
        1,
        &mut w,
    );
    // One tick of thrust 15 = 15 px/10s.
    assert_eq!(w.ships[0].vel, Vel::from_wire(15, 0));
    hold(
        ShipInput {
            forward: true,
            ..Default::default()
        },
        1000,
        &mut w,
    );
    assert_eq!(w.ships[0].vel, Vel::from_wire(2010, 0));
}

#[test]
fn afterburner_raises_speed_and_costs_energy() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 20); // facing down
    let ab = ShipInput {
        forward: true,
        afterburner: true,
        ..Default::default()
    };
    hold(ab, 300, &mut w);
    let s = &w.ships[0];
    assert!(
        s.vel.y > 2010 * VEL_SCALE,
        "afterburner should exceed normal speed"
    );
    assert!(s.vel.y <= 3250 * VEL_SCALE);
    assert!(s.energy < 1000 * ENERGY_SCALE);
    // Releasing afterburner snaps back to normal top speed.
    hold(
        ShipInput {
            forward: true,
            ..Default::default()
        },
        1,
        &mut w,
    );
    assert_eq!(w.ships[0].vel.y, 2010 * VEL_SCALE);
}

#[test]
fn rotation_speed_matches_settings() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 0);
    // InitialRotation 200 = half a turn per second.
    hold(
        ShipInput {
            right: true,
            ..Default::default()
        },
        100,
        &mut w,
    );
    assert_eq!(w.ships[0].rotation, ROTATION_UNITS / 2);
    hold(
        ShipInput {
            left: true,
            ..Default::default()
        },
        150,
        &mut w,
    );
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
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    for y in 0..MAP_TILES {
        cells[y * MAP_TILES + 520] = 1; // vertical wall at tile x = 520
    }
    let mut w = World::new(TileMap::from_cells(&cells), ArenaSettings::sandbox(), 1);
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
    // Never inside the wall.
    let px = w.ships[0].pos.pixels().x;
    assert!(px + 14 < 520 * 16);
}

#[test]
fn ship_never_tunnels_through_walls() {
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    // A box of walls around (512,512).
    for i in 500..=524 {
        for (x, y) in [(i, 500), (i, 524), (500, i), (524, i)] {
            cells[y * MAP_TILES + x] = 1;
        }
    }
    let mut w = World::new(TileMap::from_cells(&cells), ArenaSettings::sandbox(), 7);
    w.spawn_radius = 5;
    w.add_ship(0, 0, 0);
    let input = ShipInput {
        forward: true,
        afterburner: true,
        right: true,
        ..Default::default()
    };
    for t in 0..5000 {
        let mut i = input;
        i.right = t % 300 < 40;
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
fn guns_fire_with_delay_and_cost() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 0);
    let ev = hold(
        ShipInput {
            gun: true,
            ..Default::default()
        },
        100,
        &mut w,
    );
    let shots = ev
        .iter()
        .filter(|e| matches!(e, Event::FiredGun { .. }))
        .count();
    // BulletFireDelay 25: fires on tick 1, then every 26 ticks (strictly-after rule).
    assert_eq!(shots, 4);
    assert!(!w.projectiles.is_empty());
    let b = &w.projectiles[0];
    assert!(b.vel.y < 0, "bullet should travel up");
}

#[test]
fn multifire_spawns_three_bullets() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    place(&mut w, 8000, 8000, 0);
    hold(
        ShipInput {
            toggle_multifire: true,
            ..Default::default()
        },
        1,
        &mut w,
    );
    assert!(w.ships[0].multifire);
    hold(
        ShipInput {
            gun: true,
            ..Default::default()
        },
        1,
        &mut w,
    );
    assert_eq!(w.projectiles.len(), 3);
}

#[test]
fn bullet_kills_enemy_and_enemy_respawns() {
    let mut w = open_world();
    w.add_ship(0, 0, 0);
    w.add_ship(1, 0, 1);
    place(&mut w, 8000, 8000, 10); // facing right
    w.ships[1].pos = Pos::from_pixels(8100, 8000);
    w.ships[1].energy = 100 * ENERGY_SCALE;
    let ev = hold(
        ShipInput {
            gun: true,
            ..Default::default()
        },
        80,
        &mut w,
    );
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
    let ev = hold(ShipInput::default(), 250, &mut w);
    assert!(ev.iter().any(|e| matches!(e, Event::Respawned { ship: 1 })));
    assert!(w.ships[1].is_alive());
}

#[test]
fn bomb_hits_wall_and_explodes() {
    let mut cells = vec![0u8; MAP_TILES * MAP_TILES];
    cells[480 * MAP_TILES + 512] = 1;
    let mut w = World::new(TileMap::from_cells(&cells), ArenaSettings::sandbox(), 1);
    w.add_ship(0, 0, 0);
    place(&mut w, 512 * 16 + 8, 512 * 16 + 8, 0);
    let ev = hold(
        ShipInput {
            bomb: true,
            ..Default::default()
        },
        1,
        &mut w,
    );
    assert!(ev.iter().any(|e| matches!(e, Event::FiredBomb { .. })));
    // Recoil pushes the ship backwards (down).
    assert!(w.ships[0].vel.y > 0);
    let ev = hold(ShipInput::default(), 400, &mut w);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Explosion {
            kind: ProjectileKind::Bomb { .. },
            ..
        }
    )));
    assert!(w.projectiles.is_empty());
}

#[test]
fn deterministic_replay() {
    let run = || {
        let mut w = World::new(TileMap::empty(), ArenaSettings::sandbox(), 42);
        w.add_ship(0, 3, 0);
        w.add_ship(1, 5, 1);
        for t in 0u32..3000 {
            let a = ShipInput {
                forward: t % 7 < 4,
                right: t % 50 < 10,
                gun: t % 3 == 0,
                bomb: t % 200 == 0,
                afterburner: t % 400 < 100,
                ..Default::default()
            };
            let b = ShipInput {
                backward: t % 5 < 2,
                left: t % 30 < 8,
                mine: t % 500 == 1,
                ..Default::default()
            };
            w.step(&[a, b]);
        }
        (w.ships.clone(), w.projectiles.clone())
    };
    assert_eq!(run(), run());
}
