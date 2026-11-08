//! Which client settings the game uses: a checklist of every setting in the S2C
//! `0x0F` settings packet.
//!
//! Each name is one `Section:Key` from ASSS's `clientset.def` (per-ship keys as
//! `Ship:Key`, one entry for all eight ships). The test below re-derives the full list
//! from oss-proto's name tables and from `clientset-asss-public.txt` (ASSS's own
//! packet, in oss-core's test vectors), so a setting can't be missing here, and checks
//! that every `Read`/`Client` claim names an accessor that is really called. It also
//! keeps `SETTINGS.md` (the human-readable list) up to date.

/// How a setting is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// Read into [`crate::ArenaSettings`] / [`crate::ShipSettings`] (the simulation and
    /// the game take it from there), by this oss-proto accessor.
    Read(&'static str),
    /// Read directly by client code: (file in this repository, text that uses it).
    Client(&'static str, &'static str),
    /// Only matters to the server; the client has nothing to do with it.
    Server(&'static str),
    /// Not used yet, and why it matters (or doesn't).
    NotYet(&'static str),
    /// Padding or packed fields that aren't settings.
    Unused,
}

/// Every setting in the packet and how it's used.
pub const COVERAGE: &[(&str, Use)] = &[
    ("Bomb:BBombDamagePercent", Use::Read("bbomb_damage_percent")),
    ("Bomb:BombAliveTime", Use::Read("bomb_alive_time")),
    ("Bomb:BombDamageLevel", Use::Read("bomb_damage_level")),
    ("Bomb:BombExplodeDelay", Use::Read("bomb_explode_delay")),
    ("Bomb:BombExplodePixels", Use::Read("bomb_explode_pixels")),
    ("Bomb:BombSafety", Use::Read("bomb_safety")),
    ("Bomb:EBombDamagePercent", Use::Read("ebomb_damage_percent")),
    ("Bomb:EBombShutdownTime", Use::Read("ebomb_shutdown_time")),
    (
        "Bomb:JitterTime",
        Use::NotYet("screen shake when a bomb hits you (cosmetic)"),
    ),
    ("Bomb:ProximityDistance", Use::Read("proximity_distance")),
    ("Brick:BrickTime", Use::Read("brick_time")),
    ("Bullet:BulletAliveTime", Use::Read("bullet_alive_time")),
    ("Bullet:BulletDamageLevel", Use::Read("bullet_damage_level")),
    (
        "Bullet:BulletDamageUpgrade",
        Use::Read("bullet_damage_upgrade"),
    ),
    ("Bullet:ExactDamage", Use::Read("exact_damage")),
    ("Burst:BurstDamageLevel", Use::Read("burst_damage_level")),
    (
        "Chat:MessageReliable",
        Use::NotYet("chat is always sent reliably"),
    ),
    ("Door:DoorDelay", Use::Read("door_delay")),
    ("Door:DoorMode", Use::Read("door_mode")),
    (
        "Flag:CarryFlags",
        Use::Client("crates/oss-client/src/game/sandbox.rs", "carry_flags()"),
    ),
    (
        "Flag:EnterGameFlaggingDelay",
        Use::NotYet("new players wait before seeing/taking flags"),
    ),
    (
        "Flag:FlagBlankDelay",
        Use::NotYet("hide flags after no server data"),
    ),
    (
        "Flag:FlagDropDelay",
        Use::Client("crates/oss-client/src/game/sandbox.rs", "flag_drop_delay()"),
    ),
    (
        "Flag:FlagDropResetReward",
        Use::NotYet("a kill reward that resets the flag drop timer"),
    ),
    (
        "Flag:FlaggerBombFireDelay",
        Use::Read("flagger_bomb_fire_delay"),
    ),
    ("Flag:FlaggerBombUpgrade", Use::Read("flagger_bomb_upgrade")),
    (
        "Flag:FlaggerDamagePercent",
        Use::Read("flagger_damage_percent"),
    ),
    (
        "Flag:FlaggerFireCostPercent",
        Use::Read("flagger_fire_cost_percent"),
    ),
    ("Flag:FlaggerGunUpgrade", Use::Read("flagger_gun_upgrade")),
    (
        "Flag:FlaggerKillMultiplier",
        Use::Read("flagger_kill_multiplier"),
    ),
    ("Flag:FlaggerOnRadar", Use::Read("flagger_on_radar")),
    (
        "Flag:FlaggerSpeedAdjustment",
        Use::Read("flagger_speed_adjustment"),
    ),
    (
        "Flag:FlaggerThrustAdjustment",
        Use::Read("flagger_thrust_adjustment"),
    ),
    (
        "Flag:NoDataFlagDropDelay",
        Use::NotYet("drop flags after no server data"),
    ),
    (
        "Kill:BountyIncreaseForKill",
        Use::Read("bounty_increase_for_kill"),
    ),
    ("Kill:EnterDelay", Use::Read("enter_delay")),
    (
        "Kill:MaxBonus",
        Use::Server("kill points are scored by the server"),
    ),
    (
        "Kill:MaxPenalty",
        Use::Server("kill points are scored by the server"),
    ),
    (
        "Kill:RewardBase",
        Use::Server("kill points are scored by the server"),
    ),
    (
        "Latency:ClientSlowPacketSampleSize",
        Use::NotYet("client lag reports (C2S 0x00) aren't sent yet"),
    ),
    (
        "Latency:ClientSlowPacketTime",
        Use::NotYet("client lag reports (C2S 0x00) aren't sent yet"),
    ),
    (
        "Latency:S2CNoDataKickoutDelay",
        Use::NotYet("the transport's own timeout is used"),
    ),
    (
        "Latency:SendRoutePercent",
        Use::NotYet("client lag reports aren't sent yet"),
    ),
    (
        "Message:AllowAudioMessages",
        Use::NotYet("voice messages aren't supported"),
    ),
    ("Mine:MineAliveTime", Use::Read("mine_alive_time")),
    ("Mine:TeamMaxMines", Use::Read("team_max_mines")),
    (
        "Misc:ActivateAppShutdownTime",
        Use::NotYet("engine shutdown after switching back to the game"),
    ),
    ("Misc:AllowSavedShips", Use::NotYet("saved-ship option")),
    (
        "Misc:AntiWarpSettleDelay",
        Use::Read("anti_warp_settle_delay"),
    ),
    ("Misc:BounceFactor", Use::Read("bounce_factor")),
    ("Misc:DecoyAliveTime", Use::Read("decoy_alive_time")),
    ("Misc:DisableBallKilling", Use::NotYet("ball-killing rule")),
    (
        "Misc:DisableBallThroughWalls",
        Use::Read("disable_ball_through_walls"),
    ),
    (
        "Misc:DisableScreenshot",
        Use::Client(
            "crates/oss-client/src/game/sandbox.rs",
            "disable_screenshot()",
        ),
    ),
    (
        "Misc:ExtraPositionData",
        Use::NotYet("sending extra position data"),
    ),
    ("Misc:FrequencyShift", Use::NotYet("meaning not confirmed")),
    (
        "Misc:MaxTimerDrift",
        Use::NotYet("timer drift check (anti-cheat)"),
    ),
    ("Misc:NearDeathLevel", Use::NotYet("low-energy warning")),
    ("Misc:SafetyLimit", Use::NotYet("meaning not confirmed")),
    (
        "Misc:SendPositionDelay",
        Use::Client("crates/oss-session/src/lib.rs", "send_position_delay()"),
    ),
    (
        "Misc:SlowFrameCheck",
        Use::NotYet("frame-rate check (anti-cheat)"),
    ),
    ("Misc:TickerDelay", Use::NotYet("ticker text scrolling")),
    ("Misc:VictoryMusic", Use::NotYet("victory music")),
    ("Misc:WarpPointDelay", Use::Read("warp_point_delay")),
    ("Misc:WarpRadiusLimit", Use::Read("warp_radius_limit")),
    ("Prize:DeathPrizeTime", Use::Read("death_prize_time")),
    (
        "Prize:EngineShutdownTime",
        Use::Read("engine_shutdown_time"),
    ),
    ("Prize:MinimumVirtual", Use::Read("minimum_virtual")),
    ("Prize:MultiPrizeCount", Use::Read("multi_prize_count")),
    ("Prize:PrizeDelay", Use::Read("prize_delay")),
    ("Prize:PrizeFactor", Use::Read("prize_factor")),
    ("Prize:PrizeHideCount", Use::Read("prize_hide_count")),
    ("Prize:PrizeMaxExist", Use::Read("prize_max_exist")),
    ("Prize:PrizeMinExist", Use::Read("prize_min_exist")),
    (
        "Prize:PrizeNegativeFactor",
        Use::Read("prize_negative_factor"),
    ),
    (
        "Prize:TakePrizeReliable",
        Use::Client("crates/oss-session/src/lib.rs", "take_prize_reliable()"),
    ),
    ("Prize:UpgradeVirtual", Use::Read("upgrade_virtual")),
    ("PrizeWeight:AllWeapons", Use::Read("prize_weights")),
    ("PrizeWeight:AntiWarp", Use::Read("prize_weights")),
    ("PrizeWeight:Bomb", Use::Read("prize_weights")),
    ("PrizeWeight:BouncingBullets", Use::Read("prize_weights")),
    ("PrizeWeight:Brick", Use::Read("prize_weights")),
    ("PrizeWeight:Burst", Use::Read("prize_weights")),
    ("PrizeWeight:Cloak", Use::Read("prize_weights")),
    ("PrizeWeight:Decoy", Use::Read("prize_weights")),
    ("PrizeWeight:Energy", Use::Read("prize_weights")),
    ("PrizeWeight:Glue", Use::Read("prize_weights")),
    ("PrizeWeight:Gun", Use::Read("prize_weights")),
    ("PrizeWeight:MultiFire", Use::Read("prize_weights")),
    ("PrizeWeight:MultiPrize", Use::Read("prize_weights")),
    ("PrizeWeight:Portal", Use::Read("prize_weights")),
    ("PrizeWeight:Proximity", Use::Read("prize_weights")),
    ("PrizeWeight:QuickCharge", Use::Read("prize_weights")),
    ("PrizeWeight:Recharge", Use::Read("prize_weights")),
    ("PrizeWeight:Repel", Use::Read("prize_weights")),
    ("PrizeWeight:Rocket", Use::Read("prize_weights")),
    ("PrizeWeight:Rotation", Use::Read("prize_weights")),
    ("PrizeWeight:Shields", Use::Read("prize_weights")),
    ("PrizeWeight:Shrapnel", Use::Read("prize_weights")),
    ("PrizeWeight:Stealth", Use::Read("prize_weights")),
    ("PrizeWeight:Thor", Use::Read("prize_weights")),
    ("PrizeWeight:Thruster", Use::Read("prize_weights")),
    ("PrizeWeight:TopSpeed", Use::Read("prize_weights")),
    ("PrizeWeight:Warp", Use::Read("prize_weights")),
    ("PrizeWeight:XRadar", Use::Read("prize_weights")),
    (
        "Radar:MapZoomFactor",
        Use::NotYet("radar zoom is fixed at 10"),
    ),
    ("Radar:RadarMode", Use::NotYet("team radar modes")),
    (
        "Radar:RadarNeutralSize",
        Use::NotYet("neutral zone on the radar"),
    ),
    ("Repel:RepelDistance", Use::Read("repel_distance")),
    ("Repel:RepelSpeed", Use::Read("repel_speed")),
    ("Repel:RepelTime", Use::Read("repel_time")),
    ("Rocket:RocketSpeed", Use::Read("rocket_speed")),
    ("Rocket:RocketThrust", Use::Read("rocket_thrust")),
    ("Ship:AfterburnerEnergy", Use::Read("afterburner_energy")),
    ("Ship:AntiWarpEnergy", Use::Read("anti_warp_energy")),
    ("Ship:AntiWarpStatus", Use::Read("anti_warp_status")),
    (
        "Ship:AttachBounty",
        Use::Server("the server checks the bounty needed to attach"),
    ),
    ("Ship:BombBounceCount", Use::Read("bomb_bounce_count")),
    ("Ship:BombFireDelay", Use::Read("bomb_fire_delay")),
    ("Ship:BombFireEnergy", Use::Read("bomb_fire_energy")),
    (
        "Ship:BombFireEnergyUpgrade",
        Use::Read("bomb_fire_energy_upgrade"),
    ),
    ("Ship:BombSpeed", Use::Read("bomb_speed")),
    ("Ship:BombThrust", Use::Read("bomb_thrust")),
    ("Ship:BrickMax", Use::Read("brick_max")),
    ("Ship:BulletFireDelay", Use::Read("bullet_fire_delay")),
    ("Ship:BulletFireEnergy", Use::Read("bullet_fire_energy")),
    ("Ship:BulletSpeed", Use::Read("bullet_speed")),
    ("Ship:BurstMax", Use::Read("burst_max")),
    ("Ship:BurstShrapnel", Use::Read("burst_shrapnel")),
    ("Ship:BurstSpeed", Use::Read("burst_speed")),
    ("Ship:CloakEnergy", Use::Read("cloak_energy")),
    ("Ship:CloakStatus", Use::Read("cloak_status")),
    ("Ship:DamageFactor", Use::NotYet("per-ship damage factor")),
    ("Ship:DecoyMax", Use::Read("decoy_max")),
    (
        "Ship:DisableFastShooting",
        Use::Read("disable_fast_shooting"),
    ),
    ("Ship:DoubleBarrel", Use::Read("double_barrel")),
    ("Ship:EmpBomb", Use::Read("emp_bomb")),
    ("Ship:Gravity", Use::Read("gravity")),
    ("Ship:GravityTopSpeed", Use::Read("gravity_top_speed")),
    ("Ship:InitialBombs", Use::Read("initial_bombs")),
    ("Ship:InitialBounty", Use::Read("initial_bounty")),
    ("Ship:InitialBrick", Use::Read("initial_brick")),
    ("Ship:InitialBurst", Use::Read("initial_burst")),
    ("Ship:InitialDecoy", Use::Read("initial_decoy")),
    ("Ship:InitialEnergy", Use::Read("initial_energy")),
    ("Ship:InitialGuns", Use::Read("initial_guns")),
    ("Ship:InitialPortal", Use::Read("initial_portal")),
    ("Ship:InitialRecharge", Use::Read("initial_recharge")),
    ("Ship:InitialRepel", Use::Read("initial_repel")),
    ("Ship:InitialRocket", Use::Read("initial_rocket")),
    ("Ship:InitialRotation", Use::Read("initial_rotation")),
    ("Ship:InitialSpeed", Use::Read("initial_speed")),
    ("Ship:InitialThor", Use::Read("initial_thor")),
    ("Ship:InitialThrust", Use::Read("initial_thrust")),
    ("Ship:LandmineFireDelay", Use::Read("landmine_fire_delay")),
    ("Ship:LandmineFireEnergy", Use::Read("landmine_fire_energy")),
    (
        "Ship:LandmineFireEnergyUpgrade",
        Use::Read("landmine_fire_energy_upgrade"),
    ),
    ("Ship:MaxBombs", Use::Read("max_bombs")),
    ("Ship:MaxGuns", Use::Read("max_guns")),
    ("Ship:MaxMines", Use::Read("max_mines")),
    ("Ship:MaximumEnergy", Use::Read("maximum_energy")),
    ("Ship:MaximumRecharge", Use::Read("maximum_recharge")),
    ("Ship:MaximumRotation", Use::Read("maximum_rotation")),
    ("Ship:MaximumSpeed", Use::Read("maximum_speed")),
    ("Ship:MaximumThrust", Use::Read("maximum_thrust")),
    ("Ship:MultiFireAngle", Use::Read("multi_fire_angle")),
    ("Ship:MultiFireDelay", Use::Read("multi_fire_delay")),
    ("Ship:MultiFireEnergy", Use::Read("multi_fire_energy")),
    ("Ship:PortalMax", Use::Read("portal_max")),
    (
        "Ship:PrizeShareLimit",
        Use::NotYet("prize sharing with turrets"),
    ),
    ("Ship:Radius", Use::Read("radius")),
    ("Ship:RepelMax", Use::Read("repel_max")),
    ("Ship:RocketMax", Use::Read("rocket_max")),
    ("Ship:RocketTime", Use::Read("rocket_time")),
    (
        "Ship:SeeBombLevel",
        Use::NotYet("bombs on radar from this level"),
    ),
    ("Ship:SeeMines", Use::NotYet("mines on radar")),
    ("Ship:ShieldsTime", Use::Read("shields_time")),
    ("Ship:ShrapnelMax", Use::Read("shrapnel_max")),
    ("Ship:ShrapnelRate", Use::Read("shrapnel_rate")),
    ("Ship:SoccerBallFriction", Use::Read("soccer_ball_friction")),
    (
        "Ship:SoccerBallProximity",
        Use::Read("soccer_ball_proximity"),
    ),
    ("Ship:SoccerBallSpeed", Use::Read("soccer_ball_speed")),
    ("Ship:SoccerThrowTime", Use::Read("soccer_throw_time")),
    ("Ship:StealthEnergy", Use::Read("stealth_energy")),
    ("Ship:StealthStatus", Use::Read("stealth_status")),
    ("Ship:SuperTime", Use::Read("super_time")),
    ("Ship:ThorMax", Use::Read("thor_max")),
    (
        "Ship:TurretLimit",
        Use::Server("the server checks the turret count"),
    ),
    (
        "Ship:TurretSpeedPenalty",
        Use::NotYet("hosts slowed by turrets"),
    ),
    (
        "Ship:TurretThrustPenalty",
        Use::NotYet("hosts slowed by turrets"),
    ),
    ("Ship:Unused1", Use::Unused),
    ("Ship:UpgradeEnergy", Use::Read("upgrade_energy")),
    ("Ship:UpgradeRecharge", Use::Read("upgrade_recharge")),
    ("Ship:UpgradeRotation", Use::Read("upgrade_rotation")),
    ("Ship:UpgradeSpeed", Use::Read("upgrade_speed")),
    ("Ship:UpgradeThrust", Use::Read("upgrade_thrust")),
    ("Ship:XRadarEnergy", Use::Read("xradar_energy")),
    ("Ship:XRadarStatus", Use::Read("xradar_status")),
    ("Ship:___MiscBitfield___", Use::Unused),
    (
        "Shrapnel:InactiveShrapDamage",
        Use::Read("inactive_shrap_damage"),
    ),
    ("Shrapnel:Random", Use::Read("shrapnel_random")),
    (
        "Shrapnel:ShrapnelDamagePercent",
        Use::Read("shrapnel_damage_percent"),
    ),
    ("Shrapnel:ShrapnelSpeed", Use::Read("shrapnel_speed")),
    ("Soccer:AllowBombs", Use::Read("allow_bombs")),
    ("Soccer:AllowGuns", Use::Read("allow_guns")),
    ("Soccer:BallBlankDelay", Use::Read("ball_blank_delay")),
    ("Soccer:BallBounce", Use::Read("ball_bounce")),
    ("Soccer:BallLocation", Use::Read("ball_location")),
    ("Soccer:Mode", Use::Read("soccer_mode")),
    ("Soccer:PassDelay", Use::Read("pass_delay")),
    ("Soccer:UseFlagger", Use::Read("use_flagger")),
    (
        "Spawn:Team0-Radius",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team0-X",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team0-Y",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team1-Radius",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team1-X",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team1-Y",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team2-Radius",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team2-X",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team2-Y",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team3-Radius",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team3-X",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spawn:Team3-Y",
        Use::Client("crates/oss-client/src/game/sandbox.rs", ".spawns"),
    ),
    (
        "Spectator:HideFlags",
        Use::Client("crates/oss-client/src/game/sandbox.rs", "hide_flags()"),
    ),
    ("Spectator:NoXRadar", Use::NotYet("spectators' x-radar")),
    (
        "Team:MaxFrequency",
        Use::Server("the server enforces team limits"),
    ),
    (
        "Team:MaxPerPrivateTeam",
        Use::Server("the server enforces team limits"),
    ),
    (
        "Team:MaxPerTeam",
        Use::Server("the server enforces team limits"),
    ),
    ("Toggle:AntiWarpPixels", Use::Read("anti_warp_pixels")),
    ("Unused:Unused1", Use::Unused),
    ("Unused:Unused2", Use::Unused),
    ("Unused:Unused3", Use::Unused),
    ("Unused:Unused4", Use::Unused),
    ("Unused:Unused5", Use::Unused),
    ("Unused:Unused6", Use::Unused),
    ("Unused:Unused7", Use::Unused),
    ("Wormhole:GravityBombs", Use::Read("gravity_bombs")),
    ("Wormhole:SwitchTime", Use::Read("wormhole_switch_time")),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    fn repo() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
    }

    /// Every setting name oss-proto knows, per-ship names folded to `Ship:`.
    fn proto_names() -> BTreeSet<String> {
        use oss_proto::settings::*;
        let mut n: BTreeSet<String> = LONG_KEYS
            .iter()
            .chain(SHORT_KEYS.iter())
            .chain(BYTE_KEYS.iter())
            .chain(PRIZE_WEIGHT_KEYS.iter())
            .map(|s| s.to_string())
            .collect();
        n.extend(ARENA_BITS.iter().map(|b| b.0.to_string()));
        let ship = SHIP_LONG_KEYS
            .iter()
            .chain(SHIP_SHORT_KEYS.iter())
            .chain(SHIP_BYTE_KEYS.iter())
            .copied()
            .chain(SHIP_WEAPON_BITS.iter().map(|b| b.0))
            .chain(SHIP_MISC_BITS.iter().map(|b| b.0));
        n.extend(ship.filter(|k| !k.is_empty()).map(|k| format!("Ship:{k}")));
        // oss-proto names these two bits differently from ASSS's clientset.def
        // (reported to CORE); use ASSS's names.
        for (proto, asss) in [
            ("Soccer:DisableWallPass", "Misc:DisableBallThroughWalls"),
            ("Soccer:DisableBallKilling", "Misc:DisableBallKilling"),
        ] {
            if n.remove(proto) {
                n.insert(asss.to_string());
            }
        }
        n
    }

    /// Names in ASSS's own settings packet (oss-core's vectors), if the sibling
    /// checkout is there.
    fn vector_names() -> Option<BTreeSet<String>> {
        let path = repo()
            .parent()?
            .join("oss-core/crates/oss-proto/tests/vectors/clientset-asss-public.txt");
        let text = std::fs::read_to_string(path).ok()?;
        let ships = oss_proto::settings::SHIP_NAMES;
        Some(
            text.lines()
                .filter_map(|l| l.strip_prefix("field "))
                .filter_map(|l| l.split_whitespace().next())
                .map(|name| match name.split_once(':') {
                    Some((sec, key)) if ships.contains(&sec) => format!("Ship:{key}"),
                    _ => name.to_string(),
                })
                .collect(),
        )
    }

    #[test]
    fn every_setting_is_listed_once_and_claims_hold() {
        let listed: Vec<&str> = COVERAGE.iter().map(|c| c.0).collect();
        let set: BTreeSet<&str> = listed.iter().copied().collect();
        assert_eq!(set.len(), listed.len(), "a setting is listed twice");
        let missing: Vec<String> = proto_names()
            .into_iter()
            .filter(|n| !set.contains(n.as_str()) && !n.starts_with("Ship:Unused"))
            .collect();
        assert!(missing.is_empty(), "not in COVERAGE: {missing:?}");
        if let Some(v) = vector_names() {
            let missing: Vec<&String> = v.iter().filter(|n| !set.contains(n.as_str())).collect();
            assert!(
                missing.is_empty(),
                "in ASSS's packet but not in COVERAGE: {missing:?}"
            );
            let extra: Vec<&&str> = set.iter().filter(|n| !v.contains(**n)).collect();
            assert!(
                extra.is_empty(),
                "in COVERAGE but not in ASSS's packet: {extra:?}"
            );
        } else {
            eprintln!("oss-core vectors not found; checked against oss-proto's tables only");
        }
        let settings = include_str!("settings.rs");
        for (name, u) in COVERAGE {
            match u {
                Use::Read(acc) => assert!(
                    settings.contains(&format!(".{acc}()"))
                        || settings.contains(&format!(".{acc}")),
                    "{name}: settings.rs doesn't call {acc}"
                ),
                Use::Client(file, text) => {
                    let src = std::fs::read_to_string(repo().join(file))
                        .unwrap_or_else(|e| panic!("{name}: {file}: {e}"));
                    assert!(src.contains(text), "{name}: {file} doesn't use {text}");
                }
                _ => {}
            }
        }
    }

    /// `SETTINGS.md` matches the table (run with `OSS_UPDATE_SETTINGS_DOC=1` to
    /// rewrite it).
    #[test]
    fn settings_doc_is_current() {
        let mut doc = String::from(
            "# Client settings coverage\n\n\
             Generated from `crates/oss-sim/src/coverage.rs` by its tests \
             (`OSS_UPDATE_SETTINGS_DOC=1 cargo test -p oss-sim settings_doc`). Every setting \
             in the S2C `0x0F` packet, per-ship keys folded into `Ship:`.\n\n",
        );
        let count = |f: fn(&Use) -> bool| COVERAGE.iter().filter(|c| f(&c.1)).count();
        doc += &format!(
            "| Read | Client code | Server only | Not yet | Not settings |\n|---|---|---|---|---|\n| {} | {} | {} | {} | {} |\n\n",
            count(|u| matches!(u, Use::Read(_))),
            count(|u| matches!(u, Use::Client(..))),
            count(|u| matches!(u, Use::Server(_))),
            count(|u| matches!(u, Use::NotYet(_))),
            count(|u| matches!(u, Use::Unused)),
        );
        doc += "## Not used yet\n\n| Setting | Why it matters |\n|---|---|\n";
        for (n, u) in COVERAGE {
            if let Use::NotYet(why) = u {
                doc += &format!("| `{n}` | {why} |\n");
            }
        }
        doc += "\n## Server only\n\n| Setting | Note |\n|---|---|\n";
        for (n, u) in COVERAGE {
            if let Use::Server(why) = u {
                doc += &format!("| `{n}` | {why} |\n");
            }
        }
        doc += "\n## Used\n\n| Setting | How |\n|---|---|\n";
        for (n, u) in COVERAGE {
            match u {
                Use::Read(acc) => doc += &format!("| `{n}` | `{acc}` |\n"),
                Use::Client(file, text) => doc += &format!("| `{n}` | `{text}` in `{file}` |\n"),
                _ => {}
            }
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("SETTINGS.md");
        if std::env::var_os("OSS_UPDATE_SETTINGS_DOC").is_some() {
            std::fs::write(&path, &doc).unwrap();
        }
        let current = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert_eq!(
            current, doc,
            "SETTINGS.md is stale: run with OSS_UPDATE_SETTINGS_DOC=1"
        );
    }
}
