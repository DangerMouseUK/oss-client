# Client settings coverage

Generated from `crates/oss-sim/src/coverage.rs` by its tests (`OSS_UPDATE_SETTINGS_DOC=1 cargo test -p oss-sim settings_doc`). Every setting in the S2C `0x0F` packet, per-ship keys folded into `Ship:`.

| Read | Client code | Server only | Not yet | Not settings |
|---|---|---|---|---|
| 171 | 18 | 8 | 32 | 9 |

## Not used yet

| Setting | Why it matters |
|---|---|
| `Bomb:JitterTime` | screen shake when a bomb hits you (cosmetic) |
| `Chat:MessageReliable` | chat is always sent reliably |
| `Flag:EnterGameFlaggingDelay` | new players wait before seeing/taking flags |
| `Flag:FlagBlankDelay` | hide flags after no server data |
| `Flag:FlagDropResetReward` | a kill reward that resets the flag drop timer |
| `Flag:NoDataFlagDropDelay` | drop flags after no server data |
| `Latency:ClientSlowPacketSampleSize` | client lag reports (C2S 0x00) aren't sent yet |
| `Latency:ClientSlowPacketTime` | client lag reports (C2S 0x00) aren't sent yet |
| `Latency:S2CNoDataKickoutDelay` | the transport's own timeout is used |
| `Latency:SendRoutePercent` | client lag reports aren't sent yet |
| `Message:AllowAudioMessages` | voice messages aren't supported |
| `Misc:ActivateAppShutdownTime` | engine shutdown after switching back to the game |
| `Misc:AllowSavedShips` | saved-ship option |
| `Misc:DisableBallKilling` | ball-killing rule |
| `Misc:ExtraPositionData` | sending extra position data |
| `Misc:FrequencyShift` | meaning not confirmed |
| `Misc:MaxTimerDrift` | timer drift check (anti-cheat) |
| `Misc:NearDeathLevel` | low-energy warning |
| `Misc:SafetyLimit` | meaning not confirmed |
| `Misc:SlowFrameCheck` | frame-rate check (anti-cheat) |
| `Misc:TickerDelay` | ticker text scrolling |
| `Misc:VictoryMusic` | victory music |
| `Radar:MapZoomFactor` | radar zoom is fixed at 10 |
| `Radar:RadarMode` | team radar modes |
| `Radar:RadarNeutralSize` | neutral zone on the radar |
| `Ship:DamageFactor` | per-ship damage factor |
| `Ship:PrizeShareLimit` | prize sharing with turrets |
| `Ship:SeeBombLevel` | bombs on radar from this level |
| `Ship:SeeMines` | mines on radar |
| `Ship:TurretSpeedPenalty` | hosts slowed by turrets |
| `Ship:TurretThrustPenalty` | hosts slowed by turrets |
| `Spectator:NoXRadar` | spectators' x-radar |

## Server only

| Setting | Note |
|---|---|
| `Kill:MaxBonus` | kill points are scored by the server |
| `Kill:MaxPenalty` | kill points are scored by the server |
| `Kill:RewardBase` | kill points are scored by the server |
| `Ship:AttachBounty` | the server checks the bounty needed to attach |
| `Ship:TurretLimit` | the server checks the turret count |
| `Team:MaxFrequency` | the server enforces team limits |
| `Team:MaxPerPrivateTeam` | the server enforces team limits |
| `Team:MaxPerTeam` | the server enforces team limits |

## Used

| Setting | How |
|---|---|
| `Bomb:BBombDamagePercent` | `bbomb_damage_percent` |
| `Bomb:BombAliveTime` | `bomb_alive_time` |
| `Bomb:BombDamageLevel` | `bomb_damage_level` |
| `Bomb:BombExplodeDelay` | `bomb_explode_delay` |
| `Bomb:BombExplodePixels` | `bomb_explode_pixels` |
| `Bomb:BombSafety` | `bomb_safety` |
| `Bomb:EBombDamagePercent` | `ebomb_damage_percent` |
| `Bomb:EBombShutdownTime` | `ebomb_shutdown_time` |
| `Bomb:ProximityDistance` | `proximity_distance` |
| `Brick:BrickTime` | `brick_time` |
| `Bullet:BulletAliveTime` | `bullet_alive_time` |
| `Bullet:BulletDamageLevel` | `bullet_damage_level` |
| `Bullet:BulletDamageUpgrade` | `bullet_damage_upgrade` |
| `Bullet:ExactDamage` | `exact_damage` |
| `Burst:BurstDamageLevel` | `burst_damage_level` |
| `Door:DoorDelay` | `door_delay` |
| `Door:DoorMode` | `door_mode` |
| `Flag:CarryFlags` | `carry_flags()` in `crates/oss-client/src/game/sandbox.rs` |
| `Flag:FlagDropDelay` | `flag_drop_delay()` in `crates/oss-client/src/game/sandbox.rs` |
| `Flag:FlaggerBombFireDelay` | `flagger_bomb_fire_delay` |
| `Flag:FlaggerBombUpgrade` | `flagger_bomb_upgrade` |
| `Flag:FlaggerDamagePercent` | `flagger_damage_percent` |
| `Flag:FlaggerFireCostPercent` | `flagger_fire_cost_percent` |
| `Flag:FlaggerGunUpgrade` | `flagger_gun_upgrade` |
| `Flag:FlaggerKillMultiplier` | `flagger_kill_multiplier` |
| `Flag:FlaggerOnRadar` | `flagger_on_radar` |
| `Flag:FlaggerSpeedAdjustment` | `flagger_speed_adjustment` |
| `Flag:FlaggerThrustAdjustment` | `flagger_thrust_adjustment` |
| `Kill:BountyIncreaseForKill` | `bounty_increase_for_kill` |
| `Kill:EnterDelay` | `enter_delay` |
| `Mine:MineAliveTime` | `mine_alive_time` |
| `Mine:TeamMaxMines` | `team_max_mines` |
| `Misc:AntiWarpSettleDelay` | `anti_warp_settle_delay` |
| `Misc:BounceFactor` | `bounce_factor` |
| `Misc:DecoyAliveTime` | `decoy_alive_time` |
| `Misc:DisableBallThroughWalls` | `disable_ball_through_walls` |
| `Misc:DisableScreenshot` | `disable_screenshot()` in `crates/oss-client/src/game/sandbox.rs` |
| `Misc:SendPositionDelay` | `send_position_delay()` in `crates/oss-session/src/lib.rs` |
| `Misc:WarpPointDelay` | `warp_point_delay` |
| `Misc:WarpRadiusLimit` | `warp_radius_limit` |
| `Prize:DeathPrizeTime` | `death_prize_time` |
| `Prize:EngineShutdownTime` | `engine_shutdown_time` |
| `Prize:MinimumVirtual` | `minimum_virtual` |
| `Prize:MultiPrizeCount` | `multi_prize_count` |
| `Prize:PrizeDelay` | `prize_delay` |
| `Prize:PrizeFactor` | `prize_factor` |
| `Prize:PrizeHideCount` | `prize_hide_count` |
| `Prize:PrizeMaxExist` | `prize_max_exist` |
| `Prize:PrizeMinExist` | `prize_min_exist` |
| `Prize:PrizeNegativeFactor` | `prize_negative_factor` |
| `Prize:TakePrizeReliable` | `take_prize_reliable()` in `crates/oss-session/src/lib.rs` |
| `Prize:UpgradeVirtual` | `upgrade_virtual` |
| `PrizeWeight:AllWeapons` | `prize_weights` |
| `PrizeWeight:AntiWarp` | `prize_weights` |
| `PrizeWeight:Bomb` | `prize_weights` |
| `PrizeWeight:BouncingBullets` | `prize_weights` |
| `PrizeWeight:Brick` | `prize_weights` |
| `PrizeWeight:Burst` | `prize_weights` |
| `PrizeWeight:Cloak` | `prize_weights` |
| `PrizeWeight:Decoy` | `prize_weights` |
| `PrizeWeight:Energy` | `prize_weights` |
| `PrizeWeight:Glue` | `prize_weights` |
| `PrizeWeight:Gun` | `prize_weights` |
| `PrizeWeight:MultiFire` | `prize_weights` |
| `PrizeWeight:MultiPrize` | `prize_weights` |
| `PrizeWeight:Portal` | `prize_weights` |
| `PrizeWeight:Proximity` | `prize_weights` |
| `PrizeWeight:QuickCharge` | `prize_weights` |
| `PrizeWeight:Recharge` | `prize_weights` |
| `PrizeWeight:Repel` | `prize_weights` |
| `PrizeWeight:Rocket` | `prize_weights` |
| `PrizeWeight:Rotation` | `prize_weights` |
| `PrizeWeight:Shields` | `prize_weights` |
| `PrizeWeight:Shrapnel` | `prize_weights` |
| `PrizeWeight:Stealth` | `prize_weights` |
| `PrizeWeight:Thor` | `prize_weights` |
| `PrizeWeight:Thruster` | `prize_weights` |
| `PrizeWeight:TopSpeed` | `prize_weights` |
| `PrizeWeight:Warp` | `prize_weights` |
| `PrizeWeight:XRadar` | `prize_weights` |
| `Repel:RepelDistance` | `repel_distance` |
| `Repel:RepelSpeed` | `repel_speed` |
| `Repel:RepelTime` | `repel_time` |
| `Rocket:RocketSpeed` | `rocket_speed` |
| `Rocket:RocketThrust` | `rocket_thrust` |
| `Ship:AfterburnerEnergy` | `afterburner_energy` |
| `Ship:AntiWarpEnergy` | `anti_warp_energy` |
| `Ship:AntiWarpStatus` | `anti_warp_status` |
| `Ship:BombBounceCount` | `bomb_bounce_count` |
| `Ship:BombFireDelay` | `bomb_fire_delay` |
| `Ship:BombFireEnergy` | `bomb_fire_energy` |
| `Ship:BombFireEnergyUpgrade` | `bomb_fire_energy_upgrade` |
| `Ship:BombSpeed` | `bomb_speed` |
| `Ship:BombThrust` | `bomb_thrust` |
| `Ship:BrickMax` | `brick_max` |
| `Ship:BulletFireDelay` | `bullet_fire_delay` |
| `Ship:BulletFireEnergy` | `bullet_fire_energy` |
| `Ship:BulletSpeed` | `bullet_speed` |
| `Ship:BurstMax` | `burst_max` |
| `Ship:BurstShrapnel` | `burst_shrapnel` |
| `Ship:BurstSpeed` | `burst_speed` |
| `Ship:CloakEnergy` | `cloak_energy` |
| `Ship:CloakStatus` | `cloak_status` |
| `Ship:DecoyMax` | `decoy_max` |
| `Ship:DisableFastShooting` | `disable_fast_shooting` |
| `Ship:DoubleBarrel` | `double_barrel` |
| `Ship:EmpBomb` | `emp_bomb` |
| `Ship:Gravity` | `gravity` |
| `Ship:GravityTopSpeed` | `gravity_top_speed` |
| `Ship:InitialBombs` | `initial_bombs` |
| `Ship:InitialBounty` | `initial_bounty` |
| `Ship:InitialBrick` | `initial_brick` |
| `Ship:InitialBurst` | `initial_burst` |
| `Ship:InitialDecoy` | `initial_decoy` |
| `Ship:InitialEnergy` | `initial_energy` |
| `Ship:InitialGuns` | `initial_guns` |
| `Ship:InitialPortal` | `initial_portal` |
| `Ship:InitialRecharge` | `initial_recharge` |
| `Ship:InitialRepel` | `initial_repel` |
| `Ship:InitialRocket` | `initial_rocket` |
| `Ship:InitialRotation` | `initial_rotation` |
| `Ship:InitialSpeed` | `initial_speed` |
| `Ship:InitialThor` | `initial_thor` |
| `Ship:InitialThrust` | `initial_thrust` |
| `Ship:LandmineFireDelay` | `landmine_fire_delay` |
| `Ship:LandmineFireEnergy` | `landmine_fire_energy` |
| `Ship:LandmineFireEnergyUpgrade` | `landmine_fire_energy_upgrade` |
| `Ship:MaxBombs` | `max_bombs` |
| `Ship:MaxGuns` | `max_guns` |
| `Ship:MaxMines` | `max_mines` |
| `Ship:MaximumEnergy` | `maximum_energy` |
| `Ship:MaximumRecharge` | `maximum_recharge` |
| `Ship:MaximumRotation` | `maximum_rotation` |
| `Ship:MaximumSpeed` | `maximum_speed` |
| `Ship:MaximumThrust` | `maximum_thrust` |
| `Ship:MultiFireAngle` | `multi_fire_angle` |
| `Ship:MultiFireDelay` | `multi_fire_delay` |
| `Ship:MultiFireEnergy` | `multi_fire_energy` |
| `Ship:PortalMax` | `portal_max` |
| `Ship:Radius` | `radius` |
| `Ship:RepelMax` | `repel_max` |
| `Ship:RocketMax` | `rocket_max` |
| `Ship:RocketTime` | `rocket_time` |
| `Ship:ShieldsTime` | `shields_time` |
| `Ship:ShrapnelMax` | `shrapnel_max` |
| `Ship:ShrapnelRate` | `shrapnel_rate` |
| `Ship:SoccerBallFriction` | `soccer_ball_friction` |
| `Ship:SoccerBallProximity` | `soccer_ball_proximity` |
| `Ship:SoccerBallSpeed` | `soccer_ball_speed` |
| `Ship:SoccerThrowTime` | `soccer_throw_time` |
| `Ship:StealthEnergy` | `stealth_energy` |
| `Ship:StealthStatus` | `stealth_status` |
| `Ship:SuperTime` | `super_time` |
| `Ship:ThorMax` | `thor_max` |
| `Ship:UpgradeEnergy` | `upgrade_energy` |
| `Ship:UpgradeRecharge` | `upgrade_recharge` |
| `Ship:UpgradeRotation` | `upgrade_rotation` |
| `Ship:UpgradeSpeed` | `upgrade_speed` |
| `Ship:UpgradeThrust` | `upgrade_thrust` |
| `Ship:XRadarEnergy` | `xradar_energy` |
| `Ship:XRadarStatus` | `xradar_status` |
| `Shrapnel:InactiveShrapDamage` | `inactive_shrap_damage` |
| `Shrapnel:Random` | `shrapnel_random` |
| `Shrapnel:ShrapnelDamagePercent` | `shrapnel_damage_percent` |
| `Shrapnel:ShrapnelSpeed` | `shrapnel_speed` |
| `Soccer:AllowBombs` | `allow_bombs` |
| `Soccer:AllowGuns` | `allow_guns` |
| `Soccer:BallBlankDelay` | `ball_blank_delay` |
| `Soccer:BallBounce` | `ball_bounce` |
| `Soccer:BallLocation` | `ball_location` |
| `Soccer:Mode` | `soccer_mode` |
| `Soccer:PassDelay` | `pass_delay` |
| `Soccer:UseFlagger` | `use_flagger` |
| `Spawn:Team0-Radius` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team0-X` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team0-Y` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team1-Radius` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team1-X` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team1-Y` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team2-Radius` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team2-X` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team2-Y` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team3-Radius` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team3-X` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spawn:Team3-Y` | `.spawns` in `crates/oss-client/src/game/sandbox.rs` |
| `Spectator:HideFlags` | `hide_flags()` in `crates/oss-client/src/game/sandbox.rs` |
| `Toggle:AntiWarpPixels` | `anti_warp_pixels` |
| `Wormhole:GravityBombs` | `gravity_bombs` |
| `Wormhole:SwitchTime` | `wormhole_switch_time` |
