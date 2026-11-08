//! The in-game screen: offline (the sandbox) or online (a zone through
//! [`Online`]).
//!
//! The simulation runs at a fixed 100 Hz; rendering runs as fast as the display allows
//! and interpolates positions between the last two ticks. Online, your ship is
//! simulated here and reported to the server; other ships follow their position
//! packets and their weapons are recreated locally.

use std::time::{Duration, Instant};

use oss_assets::AssetDirs;
use oss_proto::objects::layer;
use oss_session::sim::{arena_world, own_position, remote_state, remote_weapon, weapon_for};
use oss_session::{SPECTATOR, SessionEvent, chat_type};
use oss_sim::{
    ArenaSettings, Event, MapObject, Pos, Prize, ProjectileKind, Ship, ShipInput, ShipState,
    TileMap, World, toggle,
};
use winit::keyboard::KeyCode;

use super::art::{self, Art};
use super::balls::{BallAction, BallPlayer, BallRules, Balls};
use super::chat::{self, ChatContext, ChatPlayer, Line};
use super::flags::{Flags, Toucher};
use super::hud::{self, RadarCache};
use super::input::{Action, InputState};
use super::lvz::{Lvz, ScreenRefs};
use super::online::Online;
use super::statbox::{self, StatBox};
use super::view::{self, Canvas, color};
use crate::audio::Audio;
use crate::gfx::{Renderer, StarParams, TextureId};

const TICK: Duration = Duration::from_millis(10);
/// Most sim ticks run per frame before we give up catching up (e.g. after a stall).
const MAX_TICKS_PER_FRAME: u32 = 25;
/// Messages kept in the log.
const MAX_MESSAGES: usize = 200;
/// Message lines shown on screen.
const LOG_LINES: usize = 8;
/// Longest chat line.
const MAX_CHAT: usize = 200;
/// Spectator camera speed, pixels per second.
const SPEC_SPEED: f32 = 1200.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    BulletExplosion,
    BombExplosion,
    EmpExplosion,
    ShipExplosion,
    Warp,
    Repel,
    Exhaust,
    RocketExhaust,
}

impl EffectKind {
    /// (frame width, frame height, duration seconds)
    pub fn params(self) -> (u32, u32, f32) {
        match self {
            EffectKind::BulletExplosion => (16, 16, 0.4),
            EffectKind::BombExplosion => (80, 80, 1.25),
            EffectKind::EmpExplosion => (80, 80, 0.5),
            EffectKind::ShipExplosion => (48, 48, 0.8),
            EffectKind::Warp => (48, 48, 0.5),
            EffectKind::Repel => (96, 96, 0.5),
            EffectKind::Exhaust => (16, 16, 0.3),
            EffectKind::RocketExhaust => (24, 24, 0.3),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Effect {
    pub kind: EffectKind,
    /// Centre in game pixels.
    pub x: f32,
    pub y: f32,
    pub start: f32,
}

/// A line in the message log.
#[derive(Debug, Clone)]
pub struct Message {
    pub text: String,
    pub color: u32,
}

pub struct Sandbox {
    pub(super) world: World,
    /// Your ship's id in `world` (your player id online).
    me_id: u16,
    pub(super) objects: Vec<MapObject>,
    pub(super) art: Art,
    pub(super) atlas: TextureId,
    /// What the atlas was built from, to rebuild it when an LVZ replaces a sheet.
    asset_dirs: AssetDirs,
    tileset: Option<Vec<u8>>,
    pub(super) radar: Option<RadarCache>,
    /// Bumped whenever doors or bricks change, so the radar image is rebuilt.
    pub(super) map_revision: u64,
    statbox: StatBox,
    /// Bottom-right of the stat box as last drawn (an LVZ anchor).
    statbox_corner: (f32, f32),
    pub(super) lvz: Lvz,
    pub(super) flags: Flags,
    pub(super) balls: Balls,
    /// Who last messaged you privately (for `::` replies).
    last_private: Option<String>,
    /// Tab completion in progress: the line as typed and how many Tabs so far.
    completing: Option<(String, usize)>,
    /// Macro keys: binding and the line they send.
    pub macros: Vec<(super::input::Binding, String)>,
    /// The ship you're attached to as a turret.
    attached_to: Option<u16>,
    /// Players wearing a crown (king of the hill).
    crowned: std::collections::BTreeSet<u16>,
    /// When your crown runs out (world tick).
    crown_deadline: Option<u32>,
    pub(super) ball_rules: BallRules,
    /// When your carried flags will be dropped (game clock), if timed.
    flag_deadline: Option<f32>,
    /// LVZ screen anchors for this frame.
    pub(super) screen_refs: ScreenRefs,
    /// World tick of the last clock correction (online).
    slew_tick: u32,
    brick_count: usize,
    last: Instant,
    acc: Duration,
    pub(super) clock: f32,
    pub(super) effects: Vec<Effect>,
    pub(super) messages: Vec<Message>,
    pub(super) menu: bool,
    pub(super) show_fps: bool,
    pub(super) big_radar: bool,
    fps: FpsCounter,
    pub quit: bool,
    /// Integer pixel scale (game pixel → physical pixels).
    pub scale: u32,
    audio: Option<Audio>,
    net: Option<Online>,
    /// Watching instead of flying (online spectator mode).
    pub(super) spectating: bool,
    spec_cam: (f32, f32),
    /// Spectating: the player the camera follows.
    spec_follow: Option<u16>,
    /// The chat line being typed, if the chat box is open.
    pub(super) typing: Option<String>,
    /// Testing aid: steer toward the nearest enemy and shoot (overrides the keys
    /// whenever an enemy is alive).
    pub aim_bot: bool,
}

/// Sounds the game uses, loaded up front.
const SOUNDS: &[&str] = &[
    "gun1", "gun2", "gun3", "gun4", "bomb1", "bomb2", "bomb3", "bomb4", "ebomb1", "ebomb2",
    "ebomb3", "ebomb4", "mine1", "mine2", "mine3", "mine4", "explode0", "explode1", "explode2",
    "ebombex", "bounce", "warp", "multion", "multioff", "thrust", "rev", "prize", "repel", "burst",
    "thor", "decoy", "rocket1", "stealth", "cloak", "xradar", "antiwarp", "off", "warppnt",
];

/// Distance (pixels) at which positional sounds fade out completely.
const HEARING_RANGE: f32 = 1200.0;

fn time_seed() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
        .unwrap_or(1)
}

impl Sandbox {
    /// Offline: fly `ship` around `level_bytes` with stock settings.
    pub fn new(
        renderer: &mut Renderer,
        dir: &AssetDirs,
        level_bytes: &[u8],
        ship: u8,
        scale: u32,
        audio: Option<Audio>,
    ) -> Result<Self, String> {
        let level = oss_formats::Level::parse(level_bytes).map_err(|e| format!("map: {e}"))?;
        let (map, objects) = TileMap::from_entries(level.tiles.iter().map(|t| (t.x, t.y, t.tile)));
        let mut world = World::new(map, ArenaSettings::sandbox(), time_seed());
        world.add_ship(0, ship.min(7), 0);
        let tileset = level.tileset.as_ref().map(|t| t.bytes.as_slice());
        let mut s = Self::build(
            renderer, dir, tileset, world, objects, 0, scale, audio, None,
        )?;
        s.message(
            "Welcome to the Open SubSpace sandbox. Press Esc for the menu.",
            color::GREEN,
        );
        Ok(s)
    }

    /// Online: the arena `net` has finished entering.
    pub fn new_online(
        renderer: &mut Renderer,
        dir: &AssetDirs,
        mut net: Online,
        scale: u32,
        audio: Option<Audio>,
    ) -> Result<Self, String> {
        let (_, map_bytes) = net.map.clone().ok_or("no map")?;
        let settings = net.session.settings().cloned().ok_or("no settings")?;
        let green_seed = net.seeds.map_or(0, |s| s.0);
        let (mut world, objects) = arena_world(&map_bytes, &settings, green_seed);
        // Online the world counts the server's ticks, so doors and greens line up with
        // everyone else's.
        world.set_tick(net.session.server_ticks(net.now_ms()));
        world.player_count = Some(net.session.players().count() as u32);
        if let Some((_, door_seed, timestamp)) = net.seeds {
            world.sync_doors(door_seed, timestamp);
        }
        let me = net.session.me().cloned().ok_or("not in an arena")?;
        // Playing a replay you're a spectator; the recorder is one of the remote ships.
        let replay = net.is_replay();
        let me_id = if replay { REPLAY_VIEWER } else { me.pid as u16 };
        // Spawn where the arena says (first spawn point for your team, if any).
        let spawns: Vec<_> = settings
            .spawns
            .iter()
            .filter(|s| s.x() != 0 || s.y() != 0)
            .collect();
        if !spawns.is_empty() {
            let sp = spawns[(me.freq.unsigned_abs() as usize) % spawns.len()];
            world.spawn_center = (sp.x() as i32, sp.y() as i32);
            world.spawn_radius = sp.radius() as i32;
        }
        for p in net.session.players() {
            if (replay || p.pid != me.pid) && p.ship != SPECTATOR {
                world.add_remote_ship(p.pid as u16, p.ship, p.freq as u16);
            }
        }
        let own_ship = if replay { SPECTATOR } else { me.ship };
        add_own_ship(&mut world, me_id, own_ship, me.freq as u16);
        let level = oss_formats::Level::parse(&map_bytes).ok();
        let tileset = level
            .as_ref()
            .and_then(|l| l.tileset.as_ref())
            .map(|t| t.bytes.as_slice());
        let spectating = own_ship == SPECTATOR;
        // ASSS-style servers start everyone as a spectator and ignore the ship in the
        // arena request; ask for the one we want. The ship appears when the server
        // confirms with a ship change.
        let wanted = net.session.config().ship;
        if !replay && wanted != SPECTATOR && me.ship != wanted {
            net.session.set_ship(wanted);
        }
        if let Some(freq) = net.freq.filter(|&f| f != me.freq && !replay) {
            net.session.set_freq(freq);
        }
        let arena = net
            .session
            .config()
            .arena
            .clone()
            .unwrap_or_else(|| "(public)".into());
        net.flush();
        let files = std::mem::take(&mut net.files);
        let mut s = Self::build(
            renderer,
            dir,
            tileset,
            world,
            objects,
            me_id,
            scale,
            audio,
            Some(net),
        )?;
        for (name, data) in files {
            s.load_lvz(&name, &data);
        }
        s.spectating = spectating;
        s.spec_cam = (8192.0, 8192.0);
        if s.is_replay() {
            s.message(
                "Playing a replay. Arrows move the camera, Ctrl follows the stat box selection.",
                color::GREEN,
            );
        } else {
            s.message(
                format!("Entered arena {arena}. Enter to chat, Esc for the menu."),
                color::GREEN,
            );
        }
        Ok(s)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        renderer: &mut Renderer,
        dir: &AssetDirs,
        tileset: Option<&[u8]>,
        world: World,
        objects: Vec<MapObject>,
        me_id: u16,
        scale: u32,
        mut audio: Option<Audio>,
        net: Option<Online>,
    ) -> Result<Self, String> {
        if let Some(a) = audio.as_mut() {
            a.preload(SOUNDS);
        }
        let (art, atlas_size, atlas_pixels) = art::load(dir, tileset)?;
        let atlas = renderer.create_texture(atlas_size, atlas_size, &atlas_pixels);
        // Turf flags sit on the map's flag tiles, numbered in file order.
        let flag_tiles = objects
            .iter()
            .filter(|o| o.id == oss_sim::tile::FLAG)
            .map(|o| (o.x as i32, o.y as i32))
            .collect();
        let mut s = Self {
            world,
            me_id,
            objects,
            art,
            atlas,
            asset_dirs: dir.clone(),
            tileset: tileset.map(<[u8]>::to_vec),
            radar: None,
            map_revision: 0,
            statbox: StatBox::default(),
            statbox_corner: (0.0, 0.0),
            lvz: Lvz::default(),
            flags: Flags::new(flag_tiles),
            flag_deadline: None,
            balls: Balls::default(),
            attached_to: None,
            last_private: None,
            completing: None,
            macros: Vec::new(),
            crowned: Default::default(),
            crown_deadline: None,
            ball_rules: net
                .as_ref()
                .and_then(|n| n.session.settings())
                .map(BallRules::from_settings)
                .unwrap_or_default(),
            screen_refs: [(0.0, 0.0); 12],
            slew_tick: 0,
            brick_count: 0,
            last: Instant::now(),
            acc: Duration::ZERO,
            clock: 0.0,
            effects: Vec::new(),
            messages: Vec::new(),
            menu: false,
            show_fps: false,
            big_radar: false,
            fps: FpsCounter::default(),
            quit: false,
            scale: scale.max(1),
            audio,
            net,
            spectating: false,
            spec_cam: (8192.0, 8192.0),
            spec_follow: None,
            typing: None,
            aim_bot: false,
        };
        if s.me().is_alive() {
            s.spawn_warp_effect();
        }
        Ok(s)
    }

    pub(super) fn me_index(&self) -> usize {
        self.world.ship_index(self.me_id).unwrap_or(0)
    }

    pub(super) fn me(&self) -> &Ship {
        &self.world.ships[self.me_index()]
    }

    /// "name(bounty)" for another player's ship, online.
    pub(super) fn player_label(&self, id: u16) -> Option<String> {
        let p = self.net.as_ref()?.session.player(id as i16)?;
        let bounty = self
            .world
            .ship_index(id)
            .map(|i| self.world.ships[i].bounty)
            .unwrap_or(0);
        let crown = if self.crowned.contains(&id) { "^" } else { "" };
        Some(format!("{crown}{}({bounty})", p.name))
    }

    pub(super) fn online(&self) -> bool {
        self.net.is_some()
    }

    pub(super) fn fps(&self) -> f32 {
        self.fps.fps
    }

    /// Camera centre (game pixels) for the current frame.
    fn camera(&self, alpha: f32) -> (f32, f32) {
        if self.spectating {
            if let Some(ship) = self.followed() {
                return view::lerp_pos(ship.prev_pos, ship.pos, alpha);
            }
            return self.spec_cam;
        }
        let me = self.me();
        view::lerp_pos(me.prev_pos, me.pos, alpha)
    }

    /// The ship being followed while spectating, if it's in play.
    fn followed(&self) -> Option<&Ship> {
        let id = self.spec_follow?;
        self.world
            .ships
            .iter()
            .find(|s| s.id == id && s.id != self.me_id)
    }

    /// Starts (or with `None`, stops) following a player while spectating. Only
    /// players in a ship can be followed; the server is asked for their extra data.
    fn follow(&mut self, pid: Option<u16>) {
        let pid = pid.filter(|&p| p != self.me_id && self.world.ship_index(p).is_some());
        if pid == self.spec_follow {
            return;
        }
        if pid.is_none()
            && let Some(ship) = self.followed()
        {
            // Carry on from where the followed ship was.
            self.spec_cam = ship.pos.to_f32();
        }
        self.spec_follow = pid;
        if let Some(net) = self.net.as_mut() {
            net.session.spectate(pid.map_or(-1, |p| p as i16));
            net.flush();
        }
    }

    pub fn message(&mut self, text: impl Into<String>, color: u32) {
        let text = text.into();
        log::debug!("message: {text}");
        self.messages.push(Message { text, color });
        if self.messages.len() > MAX_MESSAGES {
            self.messages.remove(0);
        }
    }

    fn spawn_warp_effect(&mut self) {
        let (x, y) = self.me().pos.to_f32();
        self.effect(EffectKind::Warp, x, y);
        self.sound("warp", 1.0, None);
    }

    fn effect(&mut self, kind: EffectKind, x: f32, y: f32) {
        self.effects.push(Effect {
            kind,
            x,
            y,
            start: self.clock,
        });
    }

    /// Plays a sound; with a position it is attenuated and panned relative to you.
    fn sound(&mut self, name: &str, gain: f32, at: Option<Pos>) {
        let (mut gain, mut pan) = (gain, 0.0);
        if let Some(p) = at {
            let (x, y) = p.to_f32();
            let (mx, my) = self.camera(0.0);
            let (dx, dy) = (x - mx, y - my);
            gain *= (1.0 - (dx * dx + dy * dy).sqrt() / HEARING_RANGE).max(0.0);
            pan = (dx / 600.0).clamp(-0.8, 0.8);
        }
        if let Some(a) = self.audio.as_mut() {
            a.play(name, gain, pan);
        }
    }

    /// Handles UI keys, the network and advances the simulation to the current time.
    pub fn update(&mut self, input: &mut InputState) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last)
            .min(Duration::from_millis(250));
        self.last = now;
        self.clock += dt.as_secs_f32();
        self.fps.frame(now);

        let mut edges = input.take_edges();
        let pressed = input.take_pressed();
        let typed = std::mem::take(&mut input.typed);
        let mods = (input.shift(), input.ctrl());
        self.ui_input(&mut edges, &pressed, &typed, mods);
        self.big_radar = !self.menu && input.action_held(Action::BigRadar);
        if self.spectating && !self.menu && self.typing.is_none() {
            let step = SPEC_SPEED * dt.as_secs_f32() * if input.shift() { 3.0 } else { 1.0 };
            let (dx, dy) = (
                input.action_held(Action::Right) as i32 as f32
                    - input.action_held(Action::Left) as i32 as f32,
                input.action_held(Action::Backward) as i32 as f32
                    - input.action_held(Action::Forward) as i32 as f32,
            );
            if dx != 0.0 || dy != 0.0 {
                // Moving the camera stops following.
                self.follow(None);
                self.spec_cam.0 = (self.spec_cam.0 + dx * step).clamp(0.0, 16384.0);
                self.spec_cam.1 = (self.spec_cam.1 + dy * step).clamp(0.0, 16384.0);
            } else if input.action_held(Action::Gun) || input.action_held(Action::Bomb) {
                // As in Continuum: fire follows the player selected in the stat box.
                let target = self.statbox.selected().map(|p| p as u16);
                if target.is_some() && target != self.spec_follow {
                    self.follow(target);
                }
            }
        }
        if !self.spectating && self.spec_follow.is_some() {
            self.spec_follow = None;
        }
        // --attach: once you're both in ships.
        let host = self.net.as_ref().and_then(|n| {
            let name = n.attach.as_ref()?;
            n.session
                .players()
                .find(|p| p.name.eq_ignore_ascii_case(name) && p.ship != SPECTATOR)
                .map(|p| p.pid)
        });
        if !self.spectating
            && self.me().is_alive()
            && let Some(pid) = host
            && let Some(n) = self.net.as_mut()
        {
            n.attach = None;
            n.session.attach(pid);
            n.flush();
        }
        // --follow: once that player is in a ship.
        let wanted = self.net.as_ref().and_then(|n| {
            let name = n.follow.as_ref()?;
            n.session
                .players()
                .find(|p| p.name.eq_ignore_ascii_case(name) && p.ship != SPECTATOR)
                .map(|p| p.pid as u16)
        });
        if self.spectating
            && let Some(pid) = wanted
        {
            if let Some(n) = self.net.as_mut() {
                n.follow = None;
            }
            self.follow(Some(pid));
        }
        match self.followed().map(|s| s.pos.to_f32()) {
            // The server sends what's near your (camera) position, so keep it on them.
            Some(at) => self.spec_cam = at,
            // They left or went to spectator mode.
            None if self.spec_follow.is_some() => self.spec_follow = None,
            None => {}
        }

        self.pump_network();

        self.acc += dt;
        self.sync_clock();
        let mut ticks = 0;
        while self.acc >= TICK {
            let mut ship_input = if self.menu || self.spectating {
                ShipInput::default()
            } else {
                let mut i = input.ship_input(&edges);
                if self.typing.is_some() {
                    // Tab completes names while typing rather than firing.
                    (i.bomb, i.mine) = (false, false);
                }
                i
            };
            if self.aim_bot
                && let Some(aimed) = self.aim_input()
            {
                ship_input = aimed;
            }
            // Carrying a ball: Ctrl/Tab shoot it instead of weapons (unless allowed).
            if self.balls.carrying().is_some() && (ship_input.gun || ship_input.bomb) {
                let me = self.ball_player();
                let now = self.world.tick;
                let (gun, bomb) = (ship_input.gun, ship_input.bomb);
                if let Some(shot) = self.balls.fire(&me, &self.ball_rules, gun, bomb, now) {
                    self.send_ball_actions(vec![shot]);
                }
                ship_input.gun &= self.ball_rules.allow_guns;
                ship_input.bomb &= self.ball_rules.allow_bombs;
            }
            // Edge-triggered actions apply to one tick only.
            edges.clear();
            let mut inputs = vec![ShipInput::default(); self.world.ships.len()];
            let me = self.me_index();
            if let Some(slot) = inputs.get_mut(me) {
                *slot = ship_input;
            }
            let events = self.world.step(&inputs);
            self.turret_and_crown();
            self.step_balls();
            self.handle_events(&events);
            self.report(&events);
            self.spawn_exhaust();
            let bricks = self.world.bricks().count();
            if bricks != self.brick_count {
                self.brick_count = bricks;
                self.map_revision += 1;
            }
            self.acc -= TICK;
            ticks += 1;
            if ticks >= MAX_TICKS_PER_FRAME {
                self.acc = Duration::ZERO;
                break;
            }
        }
        self.send_position_if_due();
        self.check_flags();
        if self.world.tick / 100 != (self.world.tick.wrapping_sub(ticks)) / 100 {
            let me = self.me();
            log::debug!(
                "tick {} alive {} energy {} ships {} projectiles {}",
                self.world.tick,
                me.is_alive(),
                me.energy_points(),
                self.world.ships.len(),
                self.world.projectiles.len()
            );
        }
        let clock = self.clock;
        self.effects.retain(|e| clock - e.start < e.kind.params().2);
        self.lvz.expire(clock);
        self.update_engine_sound();
    }

    /// Turns toward the nearest enemy, closes in and fires (testing aid).
    fn aim_input(&self) -> Option<ShipInput> {
        let me = self.me();
        if !me.is_alive() {
            return None;
        }
        let (mx, my) = me.pos.to_f32();
        let nearest = |points: &mut dyn Iterator<Item = (f32, f32)>| {
            points
                .map(|(x, y)| (x - mx, y - my))
                .min_by(|a, b| (a.0.hypot(a.1)).total_cmp(&b.0.hypot(b.1)))
        };
        let enemy = nearest(
            &mut self
                .world
                .ships
                .iter()
                .filter(|s| s.is_alive() && s.freq != me.freq)
                .map(|s| s.pos.to_f32()),
        );
        // No one to fight: chase a free ball, and shoot it once carried.
        if enemy.is_none() {
            if self.balls.carrying().is_some() {
                return Some(ShipInput {
                    gun: self.world.tick.is_multiple_of(200),
                    forward: true,
                    ..Default::default()
                });
            }
            let ball = nearest(
                &mut self
                    .balls
                    .balls
                    .iter()
                    .flatten()
                    .filter(|b| b.state == super::balls::BallState::World)
                    .map(|b| {
                        let (x, y) = b.pixels();
                        (x as f32, y as f32)
                    }),
            )?;
            let want =
                (ball.0.atan2(-ball.1).to_degrees().rem_euclid(360.0) / 9.0).round() as i32 % 40;
            let diff = (want - me.direction() as i32).rem_euclid(40);
            return Some(ShipInput {
                right: (1..20).contains(&diff),
                left: diff >= 20,
                forward: diff <= 3 || diff >= 37,
                backward: diff > 10 && diff < 30 && ball.0.hypot(ball.1) < 60.0,
                ..Default::default()
            });
        }
        let target = enemy?;
        let dist = target.0.hypot(target.1);
        // Direction 0 is up, clockwise, 40 steps per turn.
        let want =
            (target.0.atan2(-target.1).to_degrees().rem_euclid(360.0) / 9.0).round() as i32 % 40;
        let diff = (want - me.direction() as i32).rem_euclid(40);
        Some(ShipInput {
            right: (1..20).contains(&diff),
            left: diff >= 20,
            forward: dist > 250.0,
            backward: dist < 120.0,
            gun: diff <= 1 || diff >= 39,
            bomb: (diff <= 1 || diff >= 39) && dist < 500.0 && self.world.tick.is_multiple_of(300),
            ..Default::default()
        })
    }

    /// Menu, chat box and other non-flying keys.
    fn ui_input(
        &mut self,
        edges: &mut Vec<Action>,
        pressed: &[KeyCode],
        typed: &str,
        (shift, ctrl): (bool, bool),
    ) {
        if self.typing.is_some() && pressed.contains(&KeyCode::Tab) {
            self.complete_name();
        }
        if let Some(line) = self.typing.as_mut() {
            for c in typed.chars() {
                if !c.is_control() && line.chars().count() < MAX_CHAT {
                    line.push(c);
                    self.completing = None;
                }
            }
            let mut send = false;
            let mut cancel = false;
            for key in pressed {
                match key {
                    KeyCode::Backspace => {
                        line.pop();
                        self.completing = None;
                    }
                    KeyCode::Enter | KeyCode::NumpadEnter => send = true,
                    KeyCode::Escape => cancel = true,
                    _ => {}
                }
            }
            // While typing, the Escape key closes the chat box rather than the menu,
            // and toggles bound to keys that also type text are ignored.
            edges.retain(|a| !matches!(a, Action::Menu));
            if cancel {
                self.typing = None;
            } else if send {
                let line = self.typing.take().unwrap_or_default();
                self.send_chat_line(line.trim());
            }
            return;
        }
        if edges.contains(&Action::Menu) {
            self.menu = !self.menu;
            edges.clear();
            return;
        }
        if self.menu {
            for key in pressed {
                match key {
                    KeyCode::KeyQ => self.quit = true,
                    KeyCode::KeyS if self.online() => {
                        self.request_ship(SPECTATOR);
                        self.menu = false;
                    }
                    k => {
                        if let Some(n) = digit(*k) {
                            self.request_ship(n - 1);
                            self.menu = false;
                        }
                    }
                }
            }
            edges.clear();
            return;
        }
        if pressed
            .iter()
            .any(|k| matches!(k, KeyCode::Enter | KeyCode::NumpadEnter))
        {
            self.typing = Some(String::new());
            return;
        }
        self.run_macros(pressed, shift, ctrl);
        for e in edges.iter() {
            match e {
                Action::ShowFps => self.show_fps = !self.show_fps,
                Action::StatBox => self.statbox.cycle(),
                Action::Attach => self.toggle_attach(),
                Action::DetachTurrets => {
                    if let Some(net) = self.net.as_mut() {
                        net.session.detach_turrets();
                        net.flush();
                    }
                }
                Action::StatBoxUp => self.statbox.move_selection(false),
                Action::StatBoxDown => self.statbox.move_selection(true),
                Action::ZoomIn => self.scale = (self.scale + 1).min(8),
                Action::ZoomOut => self.scale = self.scale.saturating_sub(1).max(1),
                _ => {}
            }
        }
    }

    /// Sends (or, offline, handles) a typed chat line; see [`chat`] for the prefixes.
    fn send_chat_line(&mut self, line: &str) {
        self.send_chat_as(line, false);
    }

    /// Sends a chat line; `macro_key` sends public text as a public macro, as
    /// Continuum does for macro keys.
    fn send_chat_as(&mut self, line: &str, macro_key: bool) {
        if line.is_empty() {
            return;
        }
        let Some(net) = self.net.as_ref() else {
            self.local_command(line);
            return;
        };
        let players: Vec<ChatPlayer> = net
            .session
            .players()
            .map(|p| ChatPlayer {
                pid: p.pid,
                name: p.name.clone(),
                freq: p.freq,
            })
            .collect();
        let me_name = net.session.me().map(|p| p.name.clone()).unwrap_or_default();
        let ctx = ChatContext {
            players: &players,
            selected: self
                .statbox
                .selected()
                .filter(|&pid| pid != self.me_id as i16),
            last_private: self.last_private.as_deref(),
        };
        let parsed = chat::parse(line, &ctx);
        let Some(net) = self.net.as_mut() else { return };
        match parsed {
            Line::SetFreq(freq) => {
                net.session.set_freq(freq);
                net.flush();
            }
            Line::Problem(why) => self.message(why, color::YELLOW),
            Line::Chat {
                kind,
                target,
                text,
                echo,
            } => {
                let kind = if macro_key && kind == chat_type::PUBLIC {
                    chat_type::PUBLIC_MACRO
                } else {
                    kind
                };
                net.session.send_chat(kind, target, &text);
                net.flush();
                // Servers don't echo your own messages; commands get their reply.
                if let Some(echo) = echo {
                    let colour = match kind {
                        chat_type::FREQ => color::YELLOW,
                        chat_type::PRIVATE | chat_type::REMOTE_PRIVATE => color::GREEN,
                        chat_type::ENEMY_FREQ | chat_type::CHANNEL => color::PINK,
                        _ => color::WHITE,
                    };
                    let shown = if kind == chat_type::CHANNEL {
                        echo
                    } else {
                        format!("{me_name}> {echo}")
                    };
                    self.message(shown, colour);
                }
            }
        }
    }

    /// Tab while typing `:partial`: completes the player name, cycling on repeats.
    fn complete_name(&mut self) {
        let Some(line) = self.typing.clone() else {
            return;
        };
        let Some(net) = self.net.as_ref() else { return };
        let names: Vec<String> = net.session.players().map(|p| p.name.clone()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (base, n) = match self.completing.take() {
            Some((base, n)) => (base, n + 1),
            None => (line, 0),
        };
        if let Some(done) = chat::complete(&base, &refs, n) {
            self.typing = Some(done);
            self.completing = Some((base, n));
        }
    }

    /// Macro keys (from the settings file): send their line when pressed.
    fn run_macros(&mut self, pressed: &[KeyCode], shift: bool, ctrl: bool) {
        let lines: Vec<String> = self
            .macros
            .iter()
            .filter(|(b, _)| pressed.contains(&b.key) && b.mods.matches(shift, ctrl, b.key))
            .map(|(_, text)| text.clone())
            .collect();
        for line in lines {
            self.send_chat_as(&line, true);
        }
    }

    /// Offline `?commands`.
    fn local_command(&mut self, line: &str) {
        let mut words = line.split_whitespace();
        match words.next().map(str::to_ascii_lowercase).as_deref() {
            Some("?help") => {
                self.message(
                    "Commands: ?ship <1-8>, ?prize <id|name>, ?quit",
                    color::GREEN,
                );
            }
            Some("?ship") => match words
                .next()
                .and_then(|w| w.parse::<u8>().ok())
                .filter(|n| (1..=8).contains(n))
            {
                Some(n) => self.request_ship(n - 1),
                None => self.message("Usage: ?ship <1-8>", color::YELLOW),
            },
            Some("?prize") => {
                let arg = words.next().unwrap_or("");
                let prize = arg.parse::<i32>().ok().or_else(|| {
                    Prize::ALL
                        .iter()
                        .find(|p| format!("{p:?}").eq_ignore_ascii_case(arg))
                        .map(|p| *p as i32)
                });
                match prize {
                    Some(id) if (-28..=28).contains(&id) => {
                        let mut ev = Vec::new();
                        let me = self.me_index();
                        self.world.apply_prize(me, id, &mut ev);
                        self.handle_events(&ev);
                    }
                    _ => self.message("Usage: ?prize <1-28 or name, e.g. Thor>", color::YELLOW),
                }
            }
            Some("?quit") => self.quit = true,
            Some(c) if c.starts_with('?') => {
                self.message(format!("Unknown command {c}; try ?help"), color::YELLOW)
            }
            _ => self.message(format!("> {line}"), color::WHITE),
        }
    }

    /// Asks for a ship (online the server decides; offline it happens at once).
    fn request_ship(&mut self, kind: u8) {
        if let Some(net) = self.net.as_mut() {
            net.session.set_ship(kind);
            net.flush();
        } else if kind < SPECTATOR {
            let freq = self.me().freq;
            let i = self.me_index();
            self.world.ships.remove(i);
            self.world.add_ship(self.me_id, kind, freq);
            self.spawn_warp_effect();
        }
    }

    // --- Network -------------------------------------------------------------------------

    fn pump_network(&mut self) {
        let Some(net) = self.net.as_mut() else { return };
        let events = net.pump();
        let now_ticks = net.session.server_ticks(net.now_ms());
        let mut sim_events = Vec::new();
        for ev in events {
            match ev {
                SessionEvent::PlayerEntered(pid) => {
                    let Some(p) = self
                        .net
                        .as_ref()
                        .and_then(|n| n.session.player(pid))
                        .cloned()
                    else {
                        continue;
                    };
                    if pid as u16 != self.me_id {
                        if p.ship != SPECTATOR {
                            self.world
                                .add_remote_ship(pid as u16, p.ship, p.freq as u16);
                        }
                        self.message(format!("{} entered arena", p.name), color::GREEN);
                    }
                }
                SessionEvent::PlayerLeft(pid) => {
                    if pid as u16 != self.me_id {
                        self.world.remove_ship(pid as u16);
                    }
                }
                SessionEvent::ShipChanged { pid, ship, freq } => {
                    if pid as u16 == self.me_id {
                        let i = self.me_index();
                        self.world.ships.remove(i);
                        add_own_ship(&mut self.world, self.me_id, ship, freq as u16);
                        self.spectating = ship == SPECTATOR;
                        if !self.spectating {
                            self.spawn_warp_effect();
                        } else {
                            self.spec_cam = self.me().pos.to_f32();
                        }
                    } else {
                        self.world.remove_ship(pid as u16);
                        if ship != SPECTATOR {
                            self.world.add_remote_ship(pid as u16, ship, freq as u16);
                        }
                    }
                }
                SessionEvent::FreqChanged { pid, freq } => {
                    if let Some(i) = self.world.ship_index(pid as u16) {
                        self.world.ships[i].freq = freq as u16;
                    }
                }
                SessionEvent::PlayerMoved(pid) => {
                    let Some(pos) = self
                        .net
                        .as_ref()
                        .and_then(|n| n.session.player(pid))
                        .and_then(|p| p.position)
                    else {
                        continue;
                    };
                    let Some(i) = self
                        .world
                        .ship_index(pid as u16)
                        .filter(|_| pid as u16 != self.me_id)
                    else {
                        continue;
                    };
                    let age = now_ticks.wrapping_sub(pos.server_ticks);
                    let age = if (age as i32) < 0 { 0 } else { age };
                    if pos.x < 0 || pos.y < 0 {
                        // (-1, -1): dead or not yet spawned.
                        if self.world.ships[i].is_alive() {
                            self.world.ships[i].state = ShipState::Dead {
                                respawn_tick: self.world.tick,
                            };
                        }
                        continue;
                    }
                    self.world.set_remote_state(i, &remote_state(&pos), age);
                    if let Some(w) = pos.weapon.and_then(remote_weapon) {
                        self.world.fire_remote(i, w, &mut sim_events);
                    }
                }
                SessionEvent::Kill {
                    killer,
                    killed,
                    green,
                    ..
                } => {
                    let name = |pid: i16, s: &Self| {
                        s.net
                            .as_ref()
                            .and_then(|n| n.session.player(pid))
                            .map(|p| p.name.clone())
                            .unwrap_or_default()
                    };
                    let (kn, dn) = (name(killer, self), name(killed, self));
                    self.message(format!("{dn} killed by {kn}"), color::GREEN);
                    // A kill can leave a prize where the ship died, if it was moving.
                    if green > 0
                        && let Some(i) = self.world.ship_index(killed as u16)
                    {
                        let ship = &self.world.ships[i];
                        if ship.vel != Default::default() {
                            let p = ship.pos.pixels();
                            let (x, y) = ((p.x / 16).max(0) as u16, (p.y / 16).max(0) as u16);
                            self.world.spawn_death_green(x, y, green as i32);
                        }
                    }
                    if killed as u16 != self.me_id {
                        self.world
                            .kill_ship(killed as u16, killer as u16, &mut sim_events);
                    }
                }
                SessionEvent::Chat {
                    kind, name, text, ..
                } => {
                    if let Some(from) = chat::private_sender(kind, name.as_deref(), &text) {
                        self.last_private = Some(from);
                    }
                    let colour = match kind {
                        chat_type::ARENA | chat_type::PRIVATE | chat_type::REMOTE_PRIVATE => {
                            color::GREEN
                        }
                        chat_type::FREQ => color::YELLOW,
                        chat_type::ENEMY_FREQ | chat_type::CHANNEL => color::PINK,
                        chat_type::WARNING | chat_type::SERVER_ERROR => color::RED,
                        _ => color::WHITE,
                    };
                    match name {
                        Some(n) if kind != chat_type::ARENA => {
                            self.message(format!("{n}> {text}"), colour)
                        }
                        _ => self.message(text, colour),
                    }
                }
                SessionEvent::Settings(cs) => {
                    self.ball_rules = BallRules::from_settings(&cs);
                    self.world.settings = ArenaSettings::from_client_settings(&cs);
                }
                SessionEvent::Seeds {
                    green_seed,
                    door_seed,
                    timestamp,
                } => {
                    self.world.prize_seed = green_seed;
                    self.world.reset_green_timer();
                    sim_events.extend(self.world.sync_doors(door_seed, timestamp));
                }
                SessionEvent::FlagLocation { id, x, y, freq } => {
                    self.flags.location(id, x, y, freq)
                }
                SessionEvent::FlagPickup { id, pid } => {
                    let freq = self.player_freq(pid);
                    let carry = self.carry_flags();
                    let was_dropped = self.flags.flags.get(id as usize).is_some_and(|f| !f.turf);
                    self.flags.pickup(id, freq, carry);
                    if pid as u16 == self.me_id {
                        self.sound("flag", 1.0, None);
                        let delay = self.flag_drop_delay();
                        if was_dropped && carry > 0 && delay > 0 && self.flag_deadline.is_none() {
                            self.flag_deadline = Some(self.clock + delay as f32 / 100.0);
                        }
                    }
                }
                SessionEvent::FlagsDropped { pid } => {
                    if pid as u16 == self.me_id {
                        self.flag_deadline = None;
                    }
                }
                SessionEvent::FlagVictory { freq, points } => {
                    self.flags.victory();
                    self.flag_deadline = None;
                    self.message(
                        format!("Team {freq} won the flag game: {points} points"),
                        color::GREEN,
                    );
                }
                SessionEvent::TurfFlags(owners) => self.flags.turf(&owners),
                SessionEvent::FlagReward(rewards) => {
                    let mine = self.me().freq as i16;
                    if let Some(&(_, points)) = rewards.iter().find(|r| r.0 == mine) {
                        self.message(format!("Flag reward: {points} points"), color::GREEN);
                    }
                }
                SessionEvent::Ball(p) => {
                    log::debug!("ball report {p:?} at tick {}", self.world.tick);
                    let me = self.ball_player();
                    let now = self.world.tick;
                    let Self {
                        balls,
                        ball_rules,
                        world,
                        net,
                        ..
                    } = self;
                    let carrier = |pid: i16| {
                        net.as_ref()
                            .and_then(|n| n.session.player(pid))
                            .map(|p| (p.freq as u16, p.ship))
                    };
                    let solid = |x: i32, y: i32, f: u16| world.solid_for(x, y, f);
                    balls.report(&p, now, &me, ball_rules, carrier, &solid);
                }
                SessionEvent::Attached { pid, host } => {
                    if pid as u16 == self.me_id {
                        self.attached_to = (host >= 0).then_some(host as u16);
                        let msg = match self.attached_to {
                            Some(h) => format!("Attached to {}", self.player_name(h as i16)),
                            None => "Detached".to_string(),
                        };
                        self.message(msg, color::GREEN);
                    }
                }
                SessionEvent::TurretsDetached { pid } => {
                    if self.attached_to == Some(pid as u16) {
                        self.attached_to = None;
                        self.message("Detached", color::GREEN);
                    }
                }
                SessionEvent::Crown { on, ticks, pid } => {
                    let me = self.me_id;
                    let ids: Vec<u16> = if pid < 0 {
                        self.world.ships.iter().map(|s| s.id).collect()
                    } else {
                        vec![pid as u16]
                    };
                    for id in ids {
                        if on {
                            self.crowned.insert(id);
                        } else {
                            self.crowned.remove(&id);
                        }
                        if id == me {
                            self.crown_deadline = on.then(|| self.world.tick.wrapping_add(ticks));
                        }
                    }
                }
                SessionEvent::CrownTimer { ticks, add } => {
                    let base = match (add, self.crown_deadline) {
                        (true, Some(d)) => d,
                        _ => self.world.tick,
                    };
                    self.crown_deadline = Some(base.wrapping_add(ticks));
                }
                SessionEvent::Goal { freq, points } => {
                    self.message(
                        format!("Team {freq} scored a goal: {points} points"),
                        color::GREEN,
                    );
                    self.sound("goal", 1.0, None);
                }
                SessionEvent::Bricks(bricks) => {
                    if bricks.is_empty() {
                        self.world.clear_bricks();
                    }
                    for b in bricks {
                        self.world.place_server_brick(
                            (b.x1 as i32, b.y1 as i32),
                            (b.x2 as i32, b.y2 as i32),
                            b.freq as u16,
                            b.start_time,
                        );
                    }
                }
                SessionEvent::ObjectsToggled(toggles) => {
                    for (id, on) in toggles {
                        self.lvz.toggle(id, on, self.clock);
                    }
                }
                SessionEvent::ObjectsChanged(changes) => {
                    for c in &changes {
                        self.lvz.change(c);
                    }
                }
                SessionEvent::GreenTaken { x, y, .. } => {
                    self.world.remove_green_at(x as u16, y as u16);
                }
                SessionEvent::ShipReset => {
                    let i = self.me_index();
                    if self.world.ships[i].is_alive() {
                        self.world.respawn(i);
                        self.spawn_warp_effect();
                    }
                }
                SessionEvent::WarpTo { x, y } => {
                    let i = self.me_index();
                    let ship = &mut self.world.ships[i];
                    ship.pos = Pos::from_pixels(x as i32 * 16 + 8, y as i32 * 16 + 8);
                    ship.prev_pos = ship.pos;
                    ship.vel = Default::default();
                }
                SessionEvent::PrizeReceived { count, prize } => {
                    let i = self.me_index();
                    for _ in 0..count.clamp(0, 100) {
                        self.world.apply_prize(i, prize as i32, &mut sim_events);
                    }
                }
                SessionEvent::Closed(_) => {
                    let msg = self
                        .net
                        .as_ref()
                        .and_then(|n| n.error.clone())
                        .unwrap_or_default();
                    self.message(format!("{msg}. Press Esc, then Q to quit."), color::RED);
                }
                _ => {}
            }
        }
        self.world.player_count = self
            .net
            .as_ref()
            .map(|n| n.session.players().count() as u32);
        if self.net.as_mut().is_some_and(|n| n.replay_just_ended()) {
            self.message("End of the replay. Esc then Q to quit.", color::YELLOW);
        }
        let files = self
            .net
            .as_mut()
            .map(|n| std::mem::take(&mut n.files))
            .unwrap_or_default();
        for (name, data) in files {
            self.load_lvz(&name, &data);
        }
        self.handle_events(&sim_events);
    }

    /// Rebuilds the atlas with the LVZ replacement sheets. On failure (say the atlas
    /// is full) the current art stays.
    fn rebuild_art(&mut self, r: &mut Renderer) {
        let built = art::load_with(
            &self.asset_dirs,
            self.tileset.as_deref(),
            &self.lvz.art_overrides,
        );
        match built {
            Ok((art, size, pixels)) => {
                self.atlas = r.create_texture(size, size, &pixels);
                self.art = art;
                self.map_revision += 1;
            }
            Err(e) => log::warn!("couldn't apply LVZ graphics: {e}"),
        }
    }

    fn player_name(&self, pid: i16) -> String {
        self.net
            .as_ref()
            .and_then(|n| n.session.player(pid))
            .map_or_else(|| "?".to_string(), |p| p.name.clone())
    }

    /// F7: attach to the stat box's selected player, or detach if attached.
    fn toggle_attach(&mut self) {
        let target = if self.attached_to.is_some() {
            -1
        } else {
            match self.statbox.selected().filter(|&p| p != self.me_id as i16) {
                Some(p) => p,
                None => {
                    self.message(
                        "Select a teammate in the stat box first (Page Up/Down).",
                        color::YELLOW,
                    );
                    return;
                }
            }
        };
        if let Some(net) = self.net.as_mut() {
            net.session.attach(target);
            net.flush();
        }
    }

    /// Seconds left on your crown, if you wear one.
    pub(super) fn crown_time_left(&self) -> Option<f32> {
        self.crown_deadline
            .map(|d| (d.wrapping_sub(self.world.tick) as i32).max(0) as f32 / 100.0)
    }

    /// Rides on the host while attached; reports your crown running out.
    fn turret_and_crown(&mut self) {
        if let Some(host) = self.attached_to {
            match self.world.ship_index(host) {
                Some(h) if self.world.ships[h].is_alive() => {
                    let (pos, vel) = (self.world.ships[h].pos, self.world.ships[h].vel);
                    let i = self.me_index();
                    let me = &mut self.world.ships[i];
                    me.pos = pos;
                    me.vel = vel;
                }
                // Host gone or dead: the server detaches us; stop following meanwhile.
                _ => self.attached_to = None,
            }
        }
        if let Some(d) = self.crown_deadline
            && (self.world.tick.wrapping_sub(d) as i32) >= 0
        {
            self.crown_deadline = None;
            if let Some(net) = self.net.as_mut() {
                net.session.crown_expired();
                net.flush();
            }
        }
    }

    /// Your ship as the ball code needs it.
    fn ball_player(&self) -> BallPlayer {
        let me = self.me();
        let p = me.pos.pixels();
        let v = me.vel.to_wire();
        BallPlayer {
            pid: self.me_id as i16,
            ship: if self.spectating { 8 } else { me.kind },
            freq: me.freq,
            x: p.x,
            y: p.y,
            vx: v.x,
            vy: v.y,
            direction: me.direction(),
            can_play: me.is_alive() && !self.spectating,
        }
    }

    /// Sends what the ball code decided (pickups, shots, goals).
    fn send_ball_actions(&mut self, actions: Vec<BallAction>) {
        let Some(net) = self.net.as_mut() else { return };
        if actions.is_empty() {
            return;
        }
        let now = net.now_ms();
        for a in actions {
            match a {
                BallAction::Pickup { id, time } => net.session.pickup_ball(id, time),
                BallAction::Fire { id, pos, vel } => net.session.shoot_ball(now, id, pos, vel),
                BallAction::Goal { id, x, y } => net.session.goal(id, x, y),
            }
        }
        net.flush();
    }

    /// One tick of the balls: movement, pickups, goals, the carry timer.
    fn step_balls(&mut self) {
        if self.net.is_none() {
            return;
        }
        let me = self.ball_player();
        let now = self.world.tick;
        let Self {
            balls,
            ball_rules,
            world,
            ..
        } = self;
        let solid = |x: i32, y: i32, f: u16| world.solid_for(x, y, f);
        let goal = |x: i32, y: i32| world.map.get(x, y) == oss_sim::tile::GOAL;
        let actions = balls.step(now, &me, ball_rules, &solid, &goal);
        self.send_ball_actions(actions);
    }

    /// A player's team (yours if unknown).
    fn player_freq(&self, pid: i16) -> u16 {
        self.net
            .as_ref()
            .and_then(|n| n.session.player(pid))
            .map_or(self.me().freq, |p| p.freq as u16)
    }

    /// `Flag:HideFlags`: spectators don't see dropped flags.
    pub(super) fn hide_flags(&self) -> bool {
        self.net
            .as_ref()
            .and_then(|n| n.session.settings())
            .is_some_and(|s| s.hide_flags() != 0)
    }

    /// `Flag:CarryFlags` (0 offline).
    fn carry_flags(&self) -> u8 {
        self.net
            .as_ref()
            .and_then(|n| n.session.settings())
            .map_or(0, |s| s.carry_flags())
    }

    /// `Flag:FlagDropDelay` in ticks (0 = never).
    fn flag_drop_delay(&self) -> i32 {
        self.net
            .as_ref()
            .and_then(|n| n.session.settings())
            .map_or(0, |s| s.flag_drop_delay())
    }

    /// Flags you carry, as the server last said.
    pub(super) fn carried_flags(&self) -> u32 {
        self.net
            .as_ref()
            .and_then(|n| n.session.me())
            .map_or(0, |p| p.flags.max(0) as u32)
    }

    /// Seconds until your carried flags drop, if timed.
    pub(super) fn flag_time_left(&self) -> Option<f32> {
        self.flag_deadline.map(|d| (d - self.clock).max(0.0))
    }

    /// Asks for flags your ship touches, and drops carried flags when their time
    /// runs out.
    fn check_flags(&mut self) {
        if self.net.is_none() || self.spectating {
            return;
        }
        let me = self.me();
        if !me.is_alive() {
            return;
        }
        let p = me.pos.pixels();
        let toucher = Toucher {
            x: p.x,
            y: p.y,
            radius: self.world.settings.ships[me.kind as usize % 8].radius_pixels(),
            freq: me.freq,
            carried: self.carried_flags(),
        };
        let carry = self.carry_flags();
        let wanted = self.flags.touched(toucher, carry, self.clock);
        let expired = self.flag_deadline.is_some_and(|d| self.clock >= d);
        let Some(net) = self.net.as_mut() else { return };
        for id in wanted {
            net.session.pickup_flag(id);
        }
        if expired {
            self.flag_deadline = None;
            net.session.drop_flags();
        }
        net.flush();
    }

    /// Adds an LVZ file's objects (logged and skipped if it doesn't parse).
    pub fn load_lvz(&mut self, name: &str, data: &[u8]) {
        match self.lvz.add_file(name, data, self.clock) {
            Ok(n) => log::info!("{name}: {n} LVZ objects"),
            Err(e) => log::warn!("LVZ {e}"),
        }
    }

    /// Online, keeps the world's tick count on the server's clock: small drift is
    /// corrected by stepping one tick more or less (at most every 10 ticks), a large
    /// jump (a stall, or a clock resync) by moving the clock.
    fn sync_clock(&mut self) {
        let Some(net) = self.net.as_ref() else { return };
        let target = net.session.server_ticks(net.now_ms());
        let pending = (self.acc.as_nanos() / TICK.as_nanos()) as i32;
        let drift = target.wrapping_sub(self.world.tick) as i32 - pending;
        if drift.unsigned_abs() > 200 {
            log::debug!("clock: moved {drift} ticks to the server's");
            self.world.tick = target;
            self.acc = Duration::ZERO;
        } else if drift.abs() > 2 && self.world.tick.wrapping_sub(self.slew_tick) >= 10 {
            self.slew_tick = self.world.tick;
            if drift > 0 {
                self.acc += TICK;
            } else {
                self.acc = self.acc.saturating_sub(TICK);
            }
        }
    }

    /// Reports what your ship did this tick (weapons, death) to the server.
    fn report(&mut self, events: &[Event]) {
        let Some(net) = self.net.as_mut() else { return };
        let i = self.world.ship_index(self.me_id).unwrap_or(0);
        let me = &self.world.ships[i];
        let now = net.now_ms();
        for e in events {
            if let Some(w) = weapon_for(e, me) {
                net.session.send_position(now, &own_position(me, Some(w)));
            }
            if let Event::GreenTaken { ship, x, y, prize } = e
                && *ship == self.me_id
            {
                net.session.send_take_green(now, *x, *y, *prize);
            }
            if let Event::ShipKilled { ship, killer, .. } = e
                && *ship == self.me_id
            {
                net.session
                    .send_death(*killer as i16, me.bounty.min(u16::MAX as u32) as u16);
                net.flush();
            }
        }
    }

    fn send_position_if_due(&mut self) {
        if self.spectating {
            let (x, y) = self.spec_cam;
            if let Some(net) = self.net.as_mut() {
                let now = net.now_ms();
                if net.session.position_due(now) {
                    let pos = oss_session::OwnPosition {
                        x: x as i16,
                        y: y as i16,
                        ..Default::default()
                    };
                    net.session.send_position(now, &pos);
                }
            }
            return;
        }
        let i = self.me_index();
        let Some(net) = self.net.as_mut() else { return };
        let now = net.now_ms();
        if net.session.position_due(now) {
            net.session
                .send_position(now, &own_position(&self.world.ships[i], None));
        }
    }

    // --- Sound and effects -------------------------------------------------------------------

    fn update_engine_sound(&mut self) {
        let ship = self.me();
        let (fwd, rev) = match (ship.is_alive() && !self.spectating, ship.thrusting) {
            (true, 1) => (Some("thrust"), None),
            (true, -1) => (None, Some("rev")),
            _ => (None, None),
        };
        let gain = if ship.afterburning { 0.55 } else { 0.4 };
        if let Some(a) = self.audio.as_mut() {
            a.set_loop(0, fwd, gain);
            a.set_loop(1, rev, gain);
        }
    }

    fn handle_events(&mut self, events: &[Event]) {
        let me = self.me_id;
        let ship_pos = |w: &World, id: u16| w.ships.iter().find(|s| s.id == id).map(|s| s.pos);
        for e in events {
            match e {
                Event::FiredGun { ship, level, .. } => {
                    let at = ship_pos(&self.world, *ship);
                    self.sound(&format!("gun{}", (*level).min(3) + 1), 0.8, at);
                }
                Event::FiredBomb { ship, level } => {
                    let at = ship_pos(&self.world, *ship);
                    let emp = self
                        .world
                        .ships
                        .iter()
                        .find(|s| s.id == *ship)
                        .is_some_and(|s| self.world.settings.ships[s.kind as usize].emp_bomb);
                    let name = if emp { "ebomb" } else { "bomb" };
                    self.sound(&format!("{name}{}", (*level).min(3) + 1), 0.9, at);
                }
                Event::LaidMine { ship, level } => {
                    let at = ship_pos(&self.world, *ship);
                    self.sound(&format!("mine{}", (*level).min(3) + 1), 0.9, at);
                }
                Event::BurstFired { ship } => {
                    self.sound("burst", 0.9, ship_pos(&self.world, *ship))
                }
                Event::ThorFired { ship } => self.sound("thor", 0.9, ship_pos(&self.world, *ship)),
                Event::DecoyLaunched { ship } => {
                    self.sound("decoy", 0.9, ship_pos(&self.world, *ship))
                }
                Event::RocketStarted { ship } => {
                    self.sound("rocket1", 0.9, ship_pos(&self.world, *ship))
                }
                Event::Repelled { ship, pos } => {
                    let (x, y) = pos.to_f32();
                    self.effect(EffectKind::Repel, x, y);
                    self.sound("repel", 0.9, ship_pos(&self.world, *ship));
                }
                Event::BrickDropped { ship, x, y, .. } => {
                    self.map_revision += 1;
                    if *ship == me
                        && let Some(net) = self.net.as_mut()
                    {
                        net.session.send_brick(*x as i16, *y as i16);
                    }
                }
                Event::PortalPlaced { ship, .. } if *ship == me => self.sound("warppnt", 0.9, None),
                Event::Warped { ship, from, to } => {
                    let (x, y) = from.to_f32();
                    self.effect(EffectKind::Warp, x, y);
                    if *ship == me {
                        let (x, y) = to.to_f32();
                        self.effect(EffectKind::Warp, x, y);
                        self.sound("warp", 1.0, None);
                    }
                }
                Event::Antiwarped { ship } if *ship == me => {
                    self.message("Warp blocked by enemy antiwarp.", color::YELLOW)
                }
                Event::WarpNeedsEnergy { ship } if *ship == me => {
                    self.message("Not enough energy to warp.", color::YELLOW)
                }
                Event::MultifireToggled { on, ship } if *ship == me => {
                    self.sound(if *on { "multion" } else { "multioff" }, 0.8, None);
                }
                Event::Toggled {
                    ship,
                    toggle: t,
                    on,
                } if *ship == me => {
                    let name = match (*on, *t) {
                        (false, _) => "off",
                        (true, toggle::STEALTH) => "stealth",
                        (true, toggle::CLOAK) => "cloak",
                        (true, toggle::XRADAR) => "xradar",
                        (true, _) => "antiwarp",
                    };
                    self.sound(name, 0.8, None);
                }
                Event::PrizeTaken { ship, prize } if *ship == me => {
                    self.sound("prize", 0.8, None);
                    if let Some(p) = Prize::from_id(prize.unsigned_abs().min(255) as u8) {
                        let c = if *prize < 0 {
                            color::YELLOW
                        } else {
                            color::GREEN
                        };
                        self.message(p.message(*prize < 0), c);
                        if p == Prize::Warp && *prize > 0 {
                            self.spawn_warp_effect();
                        }
                    }
                }
                // Continuum only plays the bounce sound for hard hits (>= 3 tiles/s).
                Event::WallBounce { ship, speed } if *ship == me && *speed >= 480 => {
                    self.sound("bounce", 0.7, None);
                }
                Event::Explosion { pos, kind, emp, .. } => {
                    let (x, y) = pos.to_f32();
                    let (effect, name) = match kind {
                        ProjectileKind::Bullet
                        | ProjectileKind::Shrapnel
                        | ProjectileKind::Burst => (EffectKind::BulletExplosion, "explode0"),
                        _ if *emp => (EffectKind::EmpExplosion, "ebombex"),
                        _ => (EffectKind::BombExplosion, "explode2"),
                    };
                    self.effect(effect, x, y);
                    self.sound(name, 0.9, Some(*pos));
                }
                Event::ShipKilled {
                    pos, ship, killer, ..
                } => {
                    if *ship == me {
                        self.lvz
                            .trigger(oss_proto::objects::mode::DEATH, self.clock);
                    } else if *killer == me {
                        self.lvz.trigger(oss_proto::objects::mode::KILL, self.clock);
                    }
                    let (x, y) = pos.to_f32();
                    self.effect(EffectKind::ShipExplosion, x, y);
                    self.sound("explode1", 1.0, Some(*pos));
                }
                Event::Respawned { ship } if *ship == me => self.spawn_warp_effect(),
                Event::DoorsChanged { .. } => self.map_revision += 1,
                _ => {}
            }
        }
    }

    fn spawn_exhaust(&mut self) {
        if !self.world.tick.is_multiple_of(4) {
            return;
        }
        let mut puffs = Vec::new();
        for ship in &self.world.ships {
            if !ship.is_alive() || ship.thrusting == 0 {
                continue;
            }
            let rocket = oss_sim::ship::active(self.world.tick, ship.rocket_end_tick);
            let h = oss_sim::fixed::DIRECTION_HEADINGS[ship.direction() as usize];
            let back = if ship.thrusting > 0 { -1.0 } else { 1.0 };
            let (x, y) = ship.pos.to_f32();
            let one = oss_sim::fixed::UNIT_ONE as f32;
            let (hx, hy) = (h.x as f32 / one, h.y as f32 / one);
            let kind = if rocket {
                EffectKind::RocketExhaust
            } else {
                EffectKind::Exhaust
            };
            puffs.push((kind, x + hx * 14.0 * back, y + hy * 14.0 * back));
        }
        for (kind, x, y) in puffs {
            self.effect(kind, x, y);
        }
    }

    // --- Drawing ----------------------------------------------------------------------------

    /// Queues the whole frame on the renderer and returns the starfield parameters.
    pub fn draw(&mut self, r: &mut Renderer) -> StarParams {
        let alpha = self.acc.as_secs_f32() / TICK.as_secs_f32();
        let cam = self.camera(alpha);
        let c = Canvas::new(&self.art, self.atlas, r.size(), self.scale, cam);
        self.lvz.upload(r);
        if self.lvz.art_changed {
            self.lvz.art_changed = false;
            self.rebuild_art(r);
        }
        let chat_top = c.view_h - 16.0 - LOG_LINES as f32 * 12.0;
        self.screen_refs = hud::screen_refs(&c, self.statbox_corner, chat_top, self.spectating);
        view::draw_world(self, &c, r, alpha);
        hud::draw(self, &c, r);
        self.lvz
            .draw(layer::AFTER_GAUGES, &c, r, self.clock, &self.screen_refs);
        self.draw_statbox(&c, r);
        self.draw_log(&c, r);
        self.lvz
            .draw(layer::AFTER_CHAT, &c, r, self.clock, &self.screen_refs);
        self.lvz
            .draw(layer::TOP_MOST, &c, r, self.clock, &self.screen_refs);
        if self.menu {
            self.draw_menu(&c, r);
        }
        StarParams {
            camera: [cam.0, cam.1],
            scale: c.scale,
        }
    }

    /// Whose point of view the stat box takes: yours, or the recorder's in a replay.
    fn viewer_pid(&self) -> i16 {
        if self.me_id == REPLAY_VIEWER {
            self.net
                .as_ref()
                .and_then(|n| n.session.pid())
                .unwrap_or(-1)
        } else {
            self.me_id as i16
        }
    }

    /// Whether this is a replay being played back.
    pub(super) fn is_replay(&self) -> bool {
        self.me_id == REPLAY_VIEWER
    }

    /// Recording the visit to a replay file.
    pub(super) fn recording(&self) -> bool {
        self.net.as_ref().is_some_and(|n| n.session.recording())
    }

    /// The players the stat box lists: everyone in the arena online, just you offline.
    fn statbox_entries(&self) -> Vec<statbox::Entry> {
        let Some(net) = self.net.as_ref() else {
            let me = self.me();
            return vec![statbox::Entry {
                pid: self.me_id as i16,
                name: "You".into(),
                freq: me.freq as i16,
                kill_points: me.bounty.min(i32::MAX as u32) as i32,
                ..Default::default()
            }];
        };
        net.session
            .players()
            .map(|p| statbox::Entry {
                pid: p.pid,
                name: p.name.clone(),
                squad: p.squad.clone(),
                spectator: p.ship == SPECTATOR,
                freq: p.freq,
                kill_points: p.kill_points,
                flag_points: p.flag_points,
                wins: p.wins,
                losses: p.losses,
                flags: p.flags,
            })
            .collect()
    }

    fn draw_statbox(&mut self, c: &Canvas, r: &mut Renderer) {
        let entries = self.statbox_entries();
        let Some(l) = self.statbox.layout(self.viewer_pid(), &entries) else {
            return;
        };
        let (x, y, w, h) = l.rect;
        self.statbox_corner = (x + w + 3.0, y + h);
        c.rect(r, x, y, w + 3.0, h, [0.0, 0.0, 0.0, 0.75]);
        c.rect(r, x, l.separator, w + 3.0, 1.0, [0.45, 0.45, 0.55, 1.0]);
        for &(mx, my, m) in &l.markers {
            let colour = match m {
                statbox::Marker::Selected => [0.95, 0.85, 0.2, 1.0],
                statbox::Marker::Spectator => [0.5, 0.5, 0.5, 1.0],
                statbox::Marker::SelectedSpectator => [0.95, 0.55, 0.2, 1.0],
            };
            c.rect(r, mx + 1.0, my + 1.0, 5.0, 5.0, colour);
        }
        for t in &l.texts {
            let width = t.text.chars().count() as f32 * 8.0;
            let x = match t.align {
                statbox::Align::Left => t.x,
                statbox::Align::Right => t.x - width,
                statbox::Align::Center => (t.x - width / 2.0).floor(),
            };
            c.text(r, x, t.y, t.color, &t.text);
        }
    }

    fn draw_log(&self, c: &Canvas, r: &mut Renderer) {
        let mut y = c.view_h - 16.0;
        if let Some(line) = &self.typing {
            // Show the end of long lines.
            let max = ((c.view_w - 40.0) / 8.0).max(10.0) as usize;
            let shown: String = line
                .chars()
                .rev()
                .take(max)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let cursor = if (self.clock * 2.0).fract() < 0.5 {
                "_"
            } else {
                " "
            };
            c.text(r, 4.0, y, color::WHITE, &format!("> {shown}{cursor}"));
            y -= 14.0;
        }
        for m in self.messages.iter().rev().take(LOG_LINES) {
            c.text(r, 4.0, y, m.color, &m.text);
            y -= 12.0;
        }
        if self.spectating
            && let Some(ship) = self.followed()
        {
            let name = self
                .net
                .as_ref()
                .and_then(|n| n.session.player(ship.id as i16))
                .map_or_else(|| "?".to_string(), |p| p.name.clone());
            let mut line = format!("Spectating {name}");
            if ship.is_alive() {
                line += &format!("  Engy:{}", ship.energy_points());
            }
            let x = ((c.view_w - line.len() as f32 * 8.0) / 2.0).floor();
            c.text(r, x, 40.0, color::YELLOW, &line);
        } else if self.spectating {
            let msg = "Spectating: arrows move, Ctrl follows the stat box selection";
            c.text(
                r,
                ((c.view_w - msg.len() as f32 * 8.0) / 2.0).floor(),
                40.0,
                color::YELLOW,
                msg,
            );
        }
    }

    fn draw_menu(&self, c: &Canvas, r: &mut Renderer) {
        let mut lines = vec![
            if self.online() { "Menu" } else { "Sandbox" },
            "",
            "1-8   Change ship",
        ];
        if self.online() {
            lines.push("S     Spectate");
        }
        lines.extend([
            "Q     Quit",
            "Esc   Back to game",
            "Enter Chat ('team, :name:private, ?help)",
            "",
            "Arrows fly, Shift afterburner, Ctrl guns, Tab bombs,",
            "Shift+Tab mines, Shift+Ctrl repel, Delete multifire,",
            "Shift+Delete burst, Insert warp, Shift+Insert portal,",
            "Home stealth, Shift+Home cloak, End x-radar,",
            "Shift+End antiwarp, F3 rocket, F4 brick, F5 decoy,",
            "F6 thor, Alt big radar, Ctrl +/- scale, F12 fps",
            "F2 stat box, PgUp/PgDn select, /text to selected,",
            "F7 attach to selected (again: detach), Ctrl+F7 shake off",
        ]);
        let w = 54.0 * 8.0;
        let h = lines.len() as f32 * 14.0 + 16.0;
        let (x, y) = (
            ((c.view_w - w) / 2.0).floor(),
            ((c.view_h - h) / 2.0).floor(),
        );
        c.rect(r, x - 8.0, y - 8.0, w + 16.0, h, [0.0, 0.0, 0.0, 0.85]);
        for (i, line) in lines.iter().enumerate() {
            c.text(
                r,
                x,
                y + i as f32 * 14.0,
                if i == 0 { color::YELLOW } else { color::WHITE },
                line,
            );
        }
    }
}

/// `me_id` while playing a replay: no real player, so everyone is a remote ship.
const REPLAY_VIEWER: u16 = u16::MAX - 1;

/// Adds your own ship to an online world; as a spectator it is a hidden placeholder.
fn add_own_ship(world: &mut World, id: u16, ship: u8, freq: u16) {
    if ship == SPECTATOR {
        let i = world.add_remote_ship(id, 0, freq);
        world.ships[i].takes_damage = false;
    } else {
        world.add_ship(id, ship, freq);
    }
}

fn digit(key: KeyCode) -> Option<u8> {
    Some(match key {
        KeyCode::Digit1 => 1,
        KeyCode::Digit2 => 2,
        KeyCode::Digit3 => 3,
        KeyCode::Digit4 => 4,
        KeyCode::Digit5 => 5,
        KeyCode::Digit6 => 6,
        KeyCode::Digit7 => 7,
        KeyCode::Digit8 => 8,
        _ => return None,
    })
}

#[derive(Default)]
struct FpsCounter {
    window_start: Option<Instant>,
    frames: u32,
    fps: f32,
}

impl FpsCounter {
    fn frame(&mut self, now: Instant) {
        let start = *self.window_start.get_or_insert(now);
        self.frames += 1;
        let elapsed = now.saturating_duration_since(start).as_secs_f32();
        if elapsed >= 0.5 {
            self.fps = self.frames as f32 / elapsed;
            self.frames = 0;
            self.window_start = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::view::lerp_direction;

    #[test]
    fn direction_interpolates_the_short_way() {
        assert_eq!(lerp_direction(39_500, 500, 0.5), 0);
        assert_eq!(lerp_direction(0, 10_000, 0.5), 5);
    }
}
