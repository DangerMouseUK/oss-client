//! The subset of arena settings the simulation uses, in Continuum's own units
//! (speeds in pixels/10 s, times in ticks, energy in points). The client fills these
//! from the server's settings packet; [`ArenaSettings::default`] holds the stock values
//! a fresh zone ships with, used by the offline sandbox.

/// Per-ship settings (one block per ship type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShipSettings {
    /// Ship radius in pixels (0 means the default, 14).
    pub radius: u16,
    pub initial_rotation: u32,
    pub initial_thrust: u32,
    pub initial_speed: u32,
    pub initial_recharge: u32,
    pub initial_energy: u32,
    pub maximum_rotation: u32,
    pub maximum_thrust: u32,
    pub maximum_speed: u32,
    pub maximum_recharge: u32,
    pub maximum_energy: u32,
    /// Energy per 10 s while afterburners are on.
    pub afterburner_energy: u32,
    /// Recoil (pixels/10 s, applied backwards) when firing a bomb.
    pub bomb_thrust: u32,
    pub bullet_speed: u32,
    pub bomb_speed: u32,
    pub bullet_fire_energy: u32,
    pub multi_fire_energy: u32,
    pub bomb_fire_energy: u32,
    pub bomb_fire_energy_upgrade: u32,
    pub landmine_fire_energy: u32,
    pub landmine_fire_energy_upgrade: u32,
    pub bullet_fire_delay: u32,
    pub multi_fire_delay: u32,
    pub bomb_fire_delay: u32,
    pub landmine_fire_delay: u32,
    /// Multifire spread in 1/111 degree.
    pub multi_fire_angle: u32,
    pub bomb_bounce_count: u8,
    pub max_mines: u8,
    /// Gun level the ship starts with (0 = no guns, 1..=3).
    pub initial_guns: u8,
    pub max_guns: u8,
    pub initial_bombs: u8,
    pub max_bombs: u8,
    pub double_barrel: bool,
    /// Whether the ship starts with multifire capability (sandbox convenience).
    pub multifire: bool,
    pub bouncing_bullets: bool,
}

impl ShipSettings {
    /// Collision radius in pixels.
    pub const fn radius_pixels(&self) -> i32 {
        if self.radius == 0 {
            14
        } else {
            self.radius as i32
        }
    }
}

/// Arena-wide settings plus the eight ship blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaSettings {
    pub ships: [ShipSettings; 8],
    /// Wall bounce: velocity is multiplied by `16 / bounce_factor` on impact.
    pub bounce_factor: u32,
    pub bullet_alive_time: u32,
    pub bomb_alive_time: u32,
    pub mine_alive_time: u32,
    /// Damage in energy points.
    pub bullet_damage_level: u32,
    pub bullet_damage_upgrade: u32,
    pub bomb_damage_level: u32,
    pub bomb_explode_pixels: u32,
    /// Ticks a dead ship waits before respawning.
    pub enter_delay: u32,
}

/// Stock ship block: every ship in a freshly installed zone uses these values.
pub const STOCK_SHIP: ShipSettings = ShipSettings {
    radius: 0,
    initial_rotation: 200,
    initial_thrust: 15,
    initial_speed: 2010,
    initial_recharge: 400,
    initial_energy: 1000,
    maximum_rotation: 230,
    maximum_thrust: 17,
    maximum_speed: 3250,
    maximum_recharge: 1150,
    maximum_energy: 1700,
    afterburner_energy: 1200,
    bomb_thrust: 400,
    bullet_speed: 2000,
    bomb_speed: 2000,
    bullet_fire_energy: 20,
    multi_fire_energy: 30,
    bomb_fire_energy: 300,
    bomb_fire_energy_upgrade: 50,
    landmine_fire_energy: 270,
    landmine_fire_energy_upgrade: 150,
    bullet_fire_delay: 25,
    multi_fire_delay: 50,
    bomb_fire_delay: 150,
    landmine_fire_delay: 125,
    multi_fire_angle: 500,
    bomb_bounce_count: 0,
    max_mines: 5,
    initial_guns: 0,
    max_guns: 3,
    initial_bombs: 0,
    max_bombs: 2,
    double_barrel: false,
    multifire: false,
    bouncing_bullets: false,
};

impl Default for ArenaSettings {
    fn default() -> Self {
        Self {
            ships: [STOCK_SHIP; 8],
            bounce_factor: 22,
            bullet_alive_time: 550,
            bomb_alive_time: 8000,
            mine_alive_time: 12000,
            bullet_damage_level: 200,
            bullet_damage_upgrade: 100,
            bomb_damage_level: 750,
            bomb_explode_pixels: 80,
            enter_delay: 200,
        }
    }
}

impl ArenaSettings {
    /// Settings for flying around offline: stock physics, but every ship starts armed
    /// (level-1 guns with multifire, level-1 bombs) so there is something to shoot.
    pub fn sandbox() -> Self {
        let mut s = Self::default();
        for ship in &mut s.ships {
            ship.initial_guns = 1;
            ship.initial_bombs = 1;
            ship.multifire = true;
        }
        s
    }
}
