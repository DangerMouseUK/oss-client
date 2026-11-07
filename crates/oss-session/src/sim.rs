//! Glue between the session and `oss-sim`: reporting a simulated ship to the server
//! and building a simulation world from what the server sent.

use oss_proto::position::weapon_type;
use oss_proto::{ClientSettings, WeaponInfo};
use oss_sim::{
    ArenaSettings, Event, MapObject, RemoteState, RemoteWeapon, RemoteWeaponKind, Ship, TileMap,
    World, cap,
};

use crate::{OwnPosition, RemotePosition};

/// A remote player's reported position, for [`World::set_remote_state`].
pub fn remote_state(p: &RemotePosition) -> RemoteState {
    RemoteState {
        x: p.x as i32,
        y: p.y as i32,
        x_speed: p.x_speed as i32,
        y_speed: p.y_speed as i32,
        direction: p.rotation % 40,
        toggles: p.status,
        bounty: p.bounty as u32,
        energy: p.energy.map(u32::from),
    }
}

/// The weapon in a weapon packet, for [`World::fire_remote`].
pub fn remote_weapon(w: WeaponInfo) -> Option<RemoteWeapon> {
    let kind = match w.kind() {
        weapon_type::BULLET => RemoteWeaponKind::Bullet,
        weapon_type::BOUNCE_BULLET => RemoteWeaponKind::BouncingBullet,
        weapon_type::BOMB => RemoteWeaponKind::Bomb,
        weapon_type::PROX_BOMB => RemoteWeaponKind::ProximityBomb,
        weapon_type::REPEL => RemoteWeaponKind::Repel,
        weapon_type::DECOY => RemoteWeaponKind::Decoy,
        weapon_type::BURST => RemoteWeaponKind::Burst,
        weapon_type::THOR => RemoteWeaponKind::Thor,
        _ => return None,
    };
    Some(RemoteWeapon {
        kind,
        level: w.level(),
        alternate: w.alternate(),
        shrap: w.shrap(),
        shrap_level: w.shrap_level(),
        shrap_bouncing: w.shrap_bouncing(),
    })
}

/// Your ship as a position packet. `weapon` is set when a weapon was fired.
pub fn own_position(ship: &Ship, weapon: Option<WeaponInfo>) -> OwnPosition {
    let p = ship.pos.pixels();
    let v = ship.vel.to_wire();
    let clamp16 = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    OwnPosition {
        x: clamp16(p.x),
        y: clamp16(p.y),
        x_speed: clamp16(v.x),
        y_speed: clamp16(v.y),
        rotation: ship.direction(),
        // oss-sim's toggle bits use the protocol's status layout.
        status: ship.toggles,
        bounty: ship.bounty.min(u16::MAX as u32) as u16,
        energy: ship.energy_points().clamp(0, i16::MAX as i32) as i16,
        weapon: weapon.unwrap_or_default(),
        dead: !ship.is_alive(),
    }
}

/// The weapon packet field for a weapon your ship fired this tick, if `event` is one.
pub fn weapon_for(event: &Event, ship: &Ship) -> Option<WeaponInfo> {
    let mut w = WeaponInfo::default();
    let shrap = |w: &mut WeaponInfo| {
        if ship.guns > 0 {
            w.set_shrap(ship.shrapnel.min(31));
            w.set_shrap_level(ship.guns - 1);
            w.set_shrap_bouncing(ship.has(cap::BOUNCING_BULLETS));
        }
    };
    match *event {
        Event::FiredGun {
            ship: id,
            level,
            multifire,
        } if id == ship.id => {
            w.set_kind(if ship.has(cap::BOUNCING_BULLETS) {
                weapon_type::BOUNCE_BULLET
            } else {
                weapon_type::BULLET
            });
            w.set_level(level.min(3));
            w.set_alternate(multifire);
        }
        Event::FiredBomb { ship: id, level } | Event::LaidMine { ship: id, level }
            if id == ship.id =>
        {
            w.set_kind(if ship.has(cap::PROXIMITY) {
                weapon_type::PROX_BOMB
            } else {
                weapon_type::BOMB
            });
            w.set_level(level.min(3));
            w.set_alternate(matches!(event, Event::LaidMine { .. }));
            shrap(&mut w);
        }
        Event::ThorFired { ship: id } if id == ship.id => {
            w.set_kind(weapon_type::THOR);
            shrap(&mut w);
        }
        Event::BurstFired { ship: id } if id == ship.id => w.set_kind(weapon_type::BURST),
        Event::DecoyLaunched { ship: id } if id == ship.id => w.set_kind(weapon_type::DECOY),
        Event::Repelled { ship: id, .. } if id == ship.id => w.set_kind(weapon_type::REPEL),
        _ => return None,
    }
    Some(w)
}

/// A simulation world for the arena: its map and settings, seeded with the server's
/// green seed. Doors and bricks follow the server; the world doesn't place bricks.
pub fn arena_world(
    map_lvl: &[u8],
    settings: &ClientSettings,
    green_seed: u32,
) -> (World, Vec<MapObject>) {
    let (map, objects) = match oss_formats::Level::parse(map_lvl) {
        Ok(level) => TileMap::from_entries(level.tiles.iter().map(|t| (t.x, t.y, t.tile))),
        Err(_) => (TileMap::empty(), Vec::new()),
    };
    let mut world = World::new(
        map,
        ArenaSettings::from_client_settings(settings),
        green_seed,
    );
    world.prize_seed = green_seed;
    world.local_bricks = false;
    (world, objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_ship_and_weapons() {
        let mut ship = Ship::new(3, 0, 0, &oss_sim::STOCK_SHIP);
        ship.pos = oss_sim::Pos::from_pixels(8000, 9000);
        ship.vel = oss_sim::Vel::from_wire(-300, 120);
        ship.rotation = 25_400;
        ship.guns = 2;
        ship.caps |= cap::BOUNCING_BULLETS;
        let p = own_position(&ship, None);
        assert_eq!(
            (p.x, p.y, p.x_speed, p.y_speed, p.rotation),
            (8000, 9000, -300, 120, 25)
        );
        let w = weapon_for(
            &Event::FiredGun {
                ship: 3,
                level: 1,
                multifire: true,
            },
            &ship,
        )
        .unwrap();
        assert_eq!(
            (w.kind(), w.level(), w.alternate()),
            (weapon_type::BOUNCE_BULLET, 1, true)
        );
        assert!(
            weapon_for(
                &Event::FiredGun {
                    ship: 4,
                    level: 1,
                    multifire: true
                },
                &ship
            )
            .is_none()
        );
        let w = weapon_for(&Event::LaidMine { ship: 3, level: 0 }, &ship).unwrap();
        assert_eq!((w.kind(), w.alternate()), (weapon_type::BOMB, true));
        let r = remote_weapon(w).unwrap();
        assert_eq!((r.kind, r.alternate), (RemoteWeaponKind::Bomb, true));
        assert!(remote_weapon(WeaponInfo::default()).is_none());
    }
}
