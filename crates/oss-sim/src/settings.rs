//! The arena settings the simulation uses, in Continuum's own units (speeds in
//! pixels/10 s, times in ticks, energy in points, percentages in 0.1%). The client fills
//! these from the server's settings packet; [`ArenaSettings::default`] holds the stock
//! values a freshly installed zone ships with, used by the offline sandbox.

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
    pub upgrade_rotation: u32,
    pub upgrade_thrust: u32,
    pub upgrade_speed: u32,
    pub upgrade_recharge: u32,
    pub upgrade_energy: u32,
    /// Energy per 10 s while afterburners are on.
    pub afterburner_energy: u32,
    /// Recoil (pixels/10 s, applied backwards) when firing a bomb.
    pub bomb_thrust: u32,
    pub bullet_speed: u32,
    pub bomb_speed: u32,
    pub burst_speed: u32,
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
    /// Toggle upkeep, energy per 10 s.
    pub cloak_energy: u32,
    pub stealth_energy: u32,
    pub anti_warp_energy: u32,
    pub xradar_energy: u32,
    /// Wormhole pull (0 = none) and the top speed allowed while being pulled.
    pub gravity: i32,
    pub gravity_top_speed: u32,
    pub super_time: u32,
    pub shields_time: u32,
    pub rocket_time: u32,
    pub initial_bounty: u32,
    pub bomb_bounce_count: u8,
    pub burst_shrapnel: u8,
    pub max_mines: u8,
    pub repel_max: u8,
    pub burst_max: u8,
    pub decoy_max: u8,
    pub thor_max: u8,
    pub brick_max: u8,
    pub rocket_max: u8,
    pub portal_max: u8,
    pub initial_repel: u8,
    pub initial_burst: u8,
    pub initial_brick: u8,
    pub initial_rocket: u8,
    pub initial_thor: u8,
    pub initial_decoy: u8,
    pub initial_portal: u8,
    pub shrapnel_max: u8,
    pub shrapnel_rate: u8,
    /// 0 = never, 1 = can be prized, 2 = starts with it.
    pub cloak_status: u8,
    pub stealth_status: u8,
    pub xradar_status: u8,
    pub anti_warp_status: u8,
    /// Gun/bomb level (0 = none, 1..=3).
    pub initial_guns: u8,
    pub max_guns: u8,
    pub initial_bombs: u8,
    pub max_bombs: u8,
    pub double_barrel: bool,
    pub emp_bomb: bool,
    /// Weapons can't be fired while afterburners are on.
    pub disable_fast_shooting: bool,
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

/// Number of prize types (and prize weights).
pub const PRIZE_COUNT: usize = 28;

/// Arena-wide settings plus the eight ship blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaSettings {
    pub ships: [ShipSettings; 8],
    /// Wall bounce: velocity is multiplied by `16 / bounce_factor` on impact.
    pub bounce_factor: u32,
    pub bullet_alive_time: u32,
    pub bomb_alive_time: u32,
    pub mine_alive_time: u32,
    pub decoy_alive_time: u32,
    /// Damage in energy points.
    pub bullet_damage_level: u32,
    pub bullet_damage_upgrade: u32,
    pub bomb_damage_level: u32,
    pub burst_damage_level: u32,
    pub inactive_shrap_damage: u32,
    /// Shrapnel damage relative to bullets, EMP and bouncing-bomb damage (0.1%).
    pub shrapnel_damage_percent: u32,
    pub ebomb_damage_percent: u32,
    pub bbomb_damage_percent: u32,
    /// Bullet damage is exact rather than randomised.
    pub exact_damage: bool,
    /// Longest recharge stop from an EMP hit.
    pub ebomb_shutdown_time: u32,
    pub bomb_explode_pixels: u32,
    pub bomb_explode_delay: u32,
    /// Proximity trigger radius in tiles (plus the bomb level).
    pub proximity_distance: u32,
    pub bomb_safety: bool,
    pub shrapnel_speed: u32,
    pub shrapnel_random: bool,
    pub team_max_mines: u32,
    pub repel_speed: u32,
    pub repel_time: u32,
    pub repel_distance: u32,
    pub rocket_thrust: u32,
    pub rocket_speed: u32,
    pub brick_time: u32,
    pub warp_point_delay: u32,
    pub anti_warp_pixels: u32,
    /// Ticks of fake antiwarp after warping or using a portal.
    pub anti_warp_settle_delay: u32,
    pub engine_shutdown_time: u32,
    /// Ticks a dead ship waits before respawning.
    pub enter_delay: u32,
    pub bounty_increase_for_kill: u32,
    /// Door behaviour: -2 random, -1 weighted random, 0..=255 fixed door bits.
    pub door_mode: i32,
    /// Ticks between door changes.
    pub door_delay: u32,
    /// Prize weights in prize-id order (1 = QuickCharge .. 28 = Portal).
    pub prize_weights: [u8; PRIZE_COUNT],
    /// Greens allowed per player, in 0.1% (1000 = one per player).
    pub prize_factor: u32,
    pub prize_delay: u32,
    pub prize_hide_count: u32,
    /// Green spawn square: `minimum_virtual + upgrade_virtual * players` tiles wide.
    pub minimum_virtual: u32,
    pub upgrade_virtual: u32,
    pub prize_max_exist: u32,
    pub prize_min_exist: u32,
    /// One prize in this many is negative (0 = never).
    pub prize_negative_factor: u32,
    pub multi_prize_count: u32,
    /// Random warp/spawn distance limit from the centre (tiles).
    pub warp_radius_limit: u32,
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
    upgrade_rotation: 40,
    upgrade_thrust: 2,
    upgrade_speed: 250,
    upgrade_recharge: 166,
    upgrade_energy: 100,
    afterburner_energy: 1200,
    bomb_thrust: 400,
    bullet_speed: 2000,
    bomb_speed: 2000,
    burst_speed: 3000,
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
    cloak_energy: 100,
    stealth_energy: 333,
    anti_warp_energy: 100,
    xradar_energy: 250,
    gravity: 1500,
    gravity_top_speed: 100,
    super_time: 6000,
    shields_time: 4000,
    rocket_time: 1000,
    initial_bounty: 500,
    bomb_bounce_count: 0,
    burst_shrapnel: 24,
    max_mines: 5,
    repel_max: 3,
    burst_max: 3,
    decoy_max: 3,
    thor_max: 3,
    brick_max: 3,
    rocket_max: 3,
    portal_max: 3,
    initial_repel: 0,
    initial_burst: 0,
    initial_brick: 0,
    initial_rocket: 0,
    initial_thor: 0,
    initial_decoy: 0,
    initial_portal: 0,
    shrapnel_max: 8,
    shrapnel_rate: 2,
    cloak_status: 0,
    stealth_status: 1,
    xradar_status: 1,
    anti_warp_status: 1,
    initial_guns: 0,
    max_guns: 3,
    initial_bombs: 0,
    max_bombs: 2,
    double_barrel: false,
    emp_bomb: false,
    disable_fast_shooting: false,
};

/// Stock prize weights (QuickCharge, Energy, Rotation, Stealth, Cloak, XRadar, Warp,
/// Gun, Bomb, BouncingBullets, Thruster, TopSpeed, Recharge, Glue, MultiFire,
/// Proximity, AllWeapons, Shields, Shrapnel, AntiWarp, Repel, Burst, Decoy, Thor,
/// MultiPrize, Brick, Rocket, Portal).
pub const STOCK_PRIZE_WEIGHTS: [u8; PRIZE_COUNT] = [
    40, 40, 40, 40, 25, 40, 7, 25, 25, 25, 40, 40, 10, 0, 30, 25, 10, 10, 30, 25, 70, 70, 40, 30,
    130, 200, 50, 60,
];

impl Default for ArenaSettings {
    fn default() -> Self {
        Self {
            ships: [STOCK_SHIP; 8],
            bounce_factor: 22,
            bullet_alive_time: 550,
            bomb_alive_time: 8000,
            mine_alive_time: 12000,
            decoy_alive_time: 1500,
            bullet_damage_level: 200,
            bullet_damage_upgrade: 100,
            bomb_damage_level: 750,
            burst_damage_level: 700,
            inactive_shrap_damage: 3,
            shrapnel_damage_percent: 1000,
            ebomb_damage_percent: 1000,
            bbomb_damage_percent: 1000,
            exact_damage: false,
            ebomb_shutdown_time: 400,
            bomb_explode_pixels: 80,
            bomb_explode_delay: 150,
            proximity_distance: 3,
            bomb_safety: true,
            shrapnel_speed: 3000,
            shrapnel_random: true,
            team_max_mines: 12,
            repel_speed: 5000,
            repel_time: 225,
            repel_distance: 512,
            rocket_thrust: 25,
            rocket_speed: 5500,
            brick_time: 12000,
            warp_point_delay: 6000,
            anti_warp_pixels: 1000,
            anti_warp_settle_delay: 0,
            engine_shutdown_time: 700,
            enter_delay: 200,
            bounty_increase_for_kill: 6,
            door_mode: -2,
            door_delay: 600,
            prize_weights: STOCK_PRIZE_WEIGHTS,
            prize_factor: 10000,
            prize_delay: 700,
            prize_hide_count: 10,
            minimum_virtual: 10,
            upgrade_virtual: 6,
            prize_max_exist: 8000,
            prize_min_exist: 4000,
            prize_negative_factor: 300,
            multi_prize_count: 10,
            warp_radius_limit: 20,
        }
    }
}

impl ArenaSettings {
    /// Settings for flying around offline: the stock zone. Ships start armed through
    /// their initial bounty, exactly as in a real zone.
    pub fn sandbox() -> Self {
        Self::default()
    }

    /// Sum of all prize weights.
    pub fn prize_weight_total(&self) -> u32 {
        self.prize_weights.iter().map(|&w| w as u32).sum()
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
            upgrade_rotation: s.upgrade_rotation() as u32,
            upgrade_thrust: s.upgrade_thrust() as u32,
            upgrade_speed: s.upgrade_speed() as u32,
            upgrade_recharge: s.upgrade_recharge() as u32,
            upgrade_energy: s.upgrade_energy() as u32,
            afterburner_energy: s.afterburner_energy() as u32,
            bomb_thrust: s.bomb_thrust() as u32,
            bullet_speed: s.bullet_speed() as u32,
            bomb_speed: s.bomb_speed() as u32,
            burst_speed: s.burst_speed() as u32,
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
            cloak_energy: s.cloak_energy() as u32,
            stealth_energy: s.stealth_energy() as u32,
            anti_warp_energy: s.anti_warp_energy() as u32,
            xradar_energy: s.xradar_energy() as u32,
            gravity: s.gravity() as i32,
            gravity_top_speed: (s.gravity_top_speed() as i32).max(0) as u32,
            super_time: clamp_u32(s.super_time()),
            shields_time: clamp_u32(s.shields_time()),
            rocket_time: s.rocket_time() as u32,
            initial_bounty: s.initial_bounty() as u32,
            bomb_bounce_count: s.bomb_bounce_count(),
            burst_shrapnel: s.burst_shrapnel(),
            max_mines: s.max_mines(),
            repel_max: s.repel_max(),
            burst_max: s.burst_max(),
            decoy_max: s.decoy_max(),
            thor_max: s.thor_max(),
            brick_max: s.brick_max(),
            rocket_max: s.rocket_max(),
            portal_max: s.portal_max(),
            initial_repel: s.initial_repel(),
            initial_burst: s.initial_burst(),
            initial_brick: s.initial_brick(),
            initial_rocket: s.initial_rocket(),
            initial_thor: s.initial_thor(),
            initial_decoy: s.initial_decoy(),
            initial_portal: s.initial_portal(),
            shrapnel_max: s.shrapnel_max(),
            shrapnel_rate: s.shrapnel_rate(),
            cloak_status: s.cloak_status(),
            stealth_status: s.stealth_status(),
            xradar_status: s.xradar_status(),
            anti_warp_status: s.anti_warp_status(),
            initial_guns: s.initial_guns().min(3),
            max_guns: s.max_guns().min(3),
            initial_bombs: s.initial_bombs().min(3),
            max_bombs: s.max_bombs().min(3),
            double_barrel: s.double_barrel() != 0,
            emp_bomb: s.emp_bomb() != 0,
            disable_fast_shooting: s.disable_fast_shooting() != 0,
        }
    }
}

impl ArenaSettings {
    /// Builds simulation settings from the server's settings packet (S2C 0x0F).
    /// Damage values arrive multiplied by 1000 and are converted back to points;
    /// negative values are clamped so a hostile packet can't break the simulation.
    pub fn from_client_settings(cs: &oss_proto::ClientSettings) -> Self {
        let short = |v: i16| (v as i32).max(0) as u32;
        Self {
            ships: core::array::from_fn(|i| ShipSettings::from_proto(&cs.ships[i])),
            bounce_factor: (cs.bounce_factor() as i32).max(1) as u32,
            bullet_alive_time: clamp_u32(cs.bullet_alive_time()),
            bomb_alive_time: clamp_u32(cs.bomb_alive_time()),
            mine_alive_time: clamp_u32(cs.mine_alive_time()),
            decoy_alive_time: clamp_u32(cs.decoy_alive_time()),
            bullet_damage_level: clamp_u32(cs.bullet_damage_level()) / 1000,
            bullet_damage_upgrade: clamp_u32(cs.bullet_damage_upgrade()) / 1000,
            bomb_damage_level: clamp_u32(cs.bomb_damage_level()) / 1000,
            burst_damage_level: clamp_u32(cs.burst_damage_level()) / 1000,
            inactive_shrap_damage: clamp_u32(cs.inactive_shrap_damage()) / 1000,
            shrapnel_damage_percent: short(cs.shrapnel_damage_percent()),
            ebomb_damage_percent: short(cs.ebomb_damage_percent()),
            bbomb_damage_percent: short(cs.bbomb_damage_percent()),
            exact_damage: cs.exact_damage() != 0,
            ebomb_shutdown_time: short(cs.ebomb_shutdown_time()),
            bomb_explode_pixels: short(cs.bomb_explode_pixels()),
            bomb_explode_delay: short(cs.bomb_explode_delay()),
            proximity_distance: short(cs.proximity_distance()),
            bomb_safety: cs.bomb_safety() != 0,
            shrapnel_speed: clamp_u32(cs.shrapnel_speed()),
            shrapnel_random: cs.shrapnel_random() != 0,
            team_max_mines: cs.team_max_mines() as u32,
            repel_speed: clamp_u32(cs.repel_speed()),
            repel_time: short(cs.repel_time()),
            repel_distance: short(cs.repel_distance()),
            rocket_thrust: clamp_u32(cs.rocket_thrust()),
            rocket_speed: clamp_u32(cs.rocket_speed()),
            brick_time: short(cs.brick_time()),
            warp_point_delay: short(cs.warp_point_delay()),
            anti_warp_pixels: short(cs.anti_warp_pixels()),
            anti_warp_settle_delay: cs.anti_warp_settle_delay() as u32,
            engine_shutdown_time: short(cs.engine_shutdown_time()),
            enter_delay: short(cs.enter_delay()),
            bounty_increase_for_kill: short(cs.bounty_increase_for_kill()),
            door_mode: cs.door_mode() as i32,
            door_delay: (cs.door_delay() as i32).max(1) as u32,
            prize_weights: cs.prize_weights,
            prize_factor: short(cs.prize_factor()),
            prize_delay: short(cs.prize_delay()),
            prize_hide_count: short(cs.prize_hide_count() as i16),
            minimum_virtual: short(cs.minimum_virtual()),
            upgrade_virtual: short(cs.upgrade_virtual()),
            prize_max_exist: short(cs.prize_max_exist()),
            prize_min_exist: short(cs.prize_min_exist()),
            prize_negative_factor: short(cs.prize_negative_factor()),
            multi_prize_count: short(cs.multi_prize_count()),
            warp_radius_limit: short(cs.warp_radius_limit()),
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
        cs.prize_weights[7] = 25;
        cs.ships[2].set_initial_speed(2010);
        cs.ships[2].set_radius(16);
        cs.ships[2].set_initial_guns(2);
        cs.ships[2].set_shrapnel_max(8);
        let a = ArenaSettings::from_client_settings(&cs);
        assert_eq!(a.bounce_factor, 22);
        assert_eq!(a.bullet_damage_level, 200);
        assert_eq!(a.bullet_alive_time, 550);
        assert_eq!(a.enter_delay, 0);
        assert_eq!(a.prize_weights[7], 25);
        assert_eq!(a.ships[2].initial_speed, 2010);
        assert_eq!(a.ships[2].radius_pixels(), 16);
        assert_eq!(a.ships[2].initial_guns, 2);
        assert_eq!(a.ships[2].shrapnel_max, 8);
        // A zero bounce factor must not divide by zero.
        cs.set_bounce_factor(0);
        assert_eq!(ArenaSettings::from_client_settings(&cs).bounce_factor, 1);
    }
}
