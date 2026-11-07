//! Deterministic simulation of SubSpace ships, weapons and map collision,
//! driven by the arena settings the server sends. Shared by the client (prediction)
//! and, later, by the server (plausibility checks), so it has no rendering or I/O.
//!
//! The world advances in fixed 100 Hz ticks using integer arithmetic only (see
//! [`fixed`] for units), so identical inputs give bit-identical results everywhere.

pub mod fixed;
pub mod map;
pub mod settings;
pub mod world;

pub use fixed::{POS_SCALE, Pos, TICKS_PER_SECOND, VEL_SCALE, Vec2, Vel};
pub use map::{MAP_TILES, MapObject, TileMap, is_animated_object, is_solid_id, object_size, tile};
pub use settings::{ArenaSettings, STOCK_SHIP, ShipSettings};
pub use world::{Event, Projectile, ProjectileKind, Ship, ShipInput, ShipState, ShipStats, World};

#[cfg(test)]
mod tests;
