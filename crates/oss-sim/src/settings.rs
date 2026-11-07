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
    /// Door behaviour: -2 random, -1 weighted random, 0..=255 fixed door bits.
    pub door_mode: i32,
    /// Ticks between door changes.
    pub door_delay: u32,
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
            door_mode: -2,
            door_delay: 600,
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

fn clamp_u32(v: i32) -> u32 {
    v.max(0) as u32
}

impl ShipSettings {
    /// Reads one ship block of the server's settings packet.
    pub fn from_proto(s: &oss_proto::settings::ShipSettings) -> Self {
        Self {
            radius: s.radius() as u16,
            initial_rotation: s.initial_rotation() as u32,
            initial_thrust: s.initial_thrust() as u32,
            initial_speed: s.initial_speed() as u32,
            initial_recharge: s.initial_recharge() as u32,
            initial_energy: s.initial_energy() as u32,
            maximum_rotation: s.maximum_rotation() as u32,
            maximum_thrust: s.maximum_thrust() as u32,
            maximum_speed: s.maximum_speed() as u32,
            maximum_recharge: s.maximum_recharge() as u32,
            maximum_energy: s.maximum_energy() as u32,
            afterburner_energy: s.afterburner_energy() as u32,
            bomb_thrust: s.bomb_thrust() as u32,
            bullet_speed: s.bullet_speed() as u32,
            bomb_speed: s.bomb_speed() as u32,
            bullet_fire_energy: s.bullet_fire_energy() as u32,
            multi_fire_energy: s.multi_fire_energy() as u32,
            bomb_fire_energy: s.bomb_fire_energy() as u32,
            bomb_fire_energy_upgrade: s.bomb_fire_energy_upgrade() as u32,
            landmine_fire_energy: s.landmine_fire_energy() as u32,
            landmine_fire_energy_upgrade: s.landmine_fire_energy_upgrade() as u32,
            bullet_fire_delay: s.bullet_fire_delay() as u32,
            multi_fire_delay: s.multi_fire_delay() as u32,
            bomb_fire_delay: s.bomb_fire_delay() as u32,
            landmine_fire_delay: s.landmine_fire_delay() as u32,
            multi_fire_angle: s.multi_fire_angle() as u32,
            bomb_bounce_count: s.bomb_bounce_count(),
            max_mines: s.max_mines(),
            initial_guns: s.initial_guns().min(3),
            max_guns: s.max_guns().min(3),
            initial_bombs: s.initial_bombs().min(3),
            max_bombs: s.max_bombs().min(3),
            double_barrel: s.double_barrel() != 0,
            // Multifire and bouncing bullets are prizes, not settings.
            multifire: false,
            bouncing_bullets: false,
        }
    }
}

impl ArenaSettings {
    /// Builds simulation settings from the server's settings packet (S2C 0x0F).
    /// Damage values arrive multiplied by 1000 and are converted back to points.
    pub fn from_client_settings(cs: &oss_proto::ClientSettings) -> Self {
        Self {
            ships: core::array::from_fn(|i| ShipSettings::from_proto(&cs.ships[i])),
            bounce_factor: (cs.bounce_factor() as i32).max(1) as u32,
            bullet_alive_time: clamp_u32(cs.bullet_alive_time()),
            bomb_alive_time: clamp_u32(cs.bomb_alive_time()),
            mine_alive_time: clamp_u32(cs.mine_alive_time()),
            bullet_damage_level: clamp_u32(cs.bullet_damage_level()) / 1000,
            bullet_damage_upgrade: clamp_u32(cs.bullet_damage_upgrade()) / 1000,
            bomb_damage_level: clamp_u32(cs.bomb_damage_level()) / 1000,
            bomb_explode_pixels: clamp_u32(cs.bomb_explode_pixels() as i32),
            enter_delay: clamp_u32(cs.enter_delay() as i32),
            door_mode: cs.door_mode() as i32,
            door_delay: (cs.door_delay() as i32).max(1) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_settings_packet() {
        let mut cs = oss_proto::ClientSettings::default();
        cs.set_bounce_factor(22);
        cs.set_bullet_damage_level(200_000);
        cs.set_bullet_alive_time(550);
        cs.set_enter_delay(-5);
        cs.ships[2].set_initial_speed(2010);
        cs.ships[2].set_radius(16);
        cs.ships[2].set_initial_guns(2);
        let a = ArenaSettings::from_client_settings(&cs);
        assert_eq!(a.bounce_factor, 22);
        assert_eq!(a.bullet_damage_level, 200);
        assert_eq!(a.bullet_alive_time, 550);
        assert_eq!(a.enter_delay, 0);
        assert_eq!(a.ships[2].initial_speed, 2010);
        assert_eq!(a.ships[2].radius_pixels(), 16);
        assert_eq!(a.ships[2].initial_guns, 2);
        // A zero bounce factor must not divide by zero.
        cs.set_bounce_factor(0);
        assert_eq!(ArenaSettings::from_client_settings(&cs).bounce_factor, 1);
    }
}
