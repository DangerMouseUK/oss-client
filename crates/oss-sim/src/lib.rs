//! Deterministic simulation of SubSpace ships, weapons and map collision,
//! driven by the arena settings the server sends. Shared by the client (prediction)
//! and, later, by the server (plausibility checks), so it has no rendering or I/O.
//!
//! The world advances in fixed 100 Hz ticks using integer arithmetic only (see
//! [`fixed`] for units), so identical inputs give bit-identical results everywhere.

pub mod coverage;
pub mod fixed;
pub mod map;
pub mod prize;
pub mod remote;
pub mod settings;
pub mod ship;
pub mod weapons;
pub mod world;

pub use fixed::{POS_SCALE, Pos, TICKS_PER_SECOND, VEL_SCALE, Vec2, Vel, VieRng};
pub use map::{MAP_TILES, MapObject, TileMap, is_animated_object, is_solid_id, object_size, tile};
pub use prize::{Green, Prize, generate_prize};
pub use remote::{RemoteState, RemoteWeapon, RemoteWeaponKind};
pub use settings::{
    ArenaSettings, FlaggerSettings, PRIZE_COUNT, STOCK_SHIP, ShipSettings, SoccerSettings,
};
pub use ship::{Items, Ship, ShipInput, ShipState, ShipStats, cap, toggle};
pub use weapons::{Projectile, ProjectileKind, ProxTrigger};
pub use world::{Brick, Event, World};

#[cfg(test)]
mod tests;
