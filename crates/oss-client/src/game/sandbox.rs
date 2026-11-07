//! The in-game screen: offline (the sandbox) or online (a zone through
//! [`Online`]).
//!
//! The simulation runs at a fixed 100 Hz; rendering runs as fast as the display allows
//! and interpolates positions between the last two ticks. Online, your ship is
//! simulated here and reported to the server; other ships follow their position
//! packets and their weapons are recreated locally.

use std::time::{Duration, Instant};

use oss_assets::ContinuumDir;
use oss_session::sim::{arena_world, own_position, remote_state, remote_weapon, weapon_for};
use oss_session::{SPECTATOR, SessionEvent, chat_type};
use oss_sim::{
    ArenaSettings, Event, MapObject, Pos, Prize, ProjectileKind, Ship, ShipInput, ShipState,
    TileMap, World, toggle,
};
use winit::keyboard::KeyCode;

use super::art::{self, Art};
use super::hud::{self, RadarCache};
use super::input::{Action, InputState};
use super::online::Online;
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
    pub(super) radar: Option<RadarCache>,
    /// Bumped whenever doors or bricks change, so the radar image is rebuilt.
    pub(super) map_revision: u64,
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
        dir: &ContinuumDir,
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
        dir: &ContinuumDir,
        mut net: Online,
        scale: u32,
        audio: Option<Audio>,
    ) -> Result<Self, String> {
        let (_, map_bytes) = net.map.clone().ok_or("no map")?;
        let settings = net.session.settings().cloned().ok_or("no settings")?;
        let (mut world, objects) = arena_world(&map_bytes, &settings, net.green_seed);
        let me = net.session.me().cloned().ok_or("not in an arena")?;
        let me_id = me.pid as u16;
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
            if p.pid != me.pid && p.ship != SPECTATOR {
                world.add_remote_ship(p.pid as u16, p.ship, p.freq as u16);
            }
        }
        add_own_ship(&mut world, me_id, me.ship, me.freq as u16);
        let level = oss_formats::Level::parse(&map_bytes).ok();
        let tileset = level
            .as_ref()
            .and_then(|l| l.tileset.as_ref())
            .map(|t| t.bytes.as_slice());
        let spectating = me.ship == SPECTATOR;
        // ASSS-style servers start everyone as a spectator and ignore the ship in the
        // arena request; ask for the one we want. The ship appears when the server
        // confirms with a ship change.
        let wanted = net.session.config().ship;
        if wanted != SPECTATOR && me.ship != wanted {
            net.session.set_ship(wanted);
        }
        if let Some(freq) = net.freq.filter(|&f| f != me.freq) {
            net.session.set_freq(freq);
        }
        let arena = net
            .session
            .config()
            .arena
            .clone()
            .unwrap_or_else(|| "(public)".into());
        net.flush();
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
        s.spectating = spectating;
        s.spec_cam = (8192.0, 8192.0);
        s.message(
            format!("Entered arena {arena}. Enter to chat, Esc for the menu."),
            color::GREEN,
        );
        Ok(s)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        renderer: &mut Renderer,
        dir: &ContinuumDir,
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
        let mut s = Self {
            world,
            me_id,
            objects,
            art,
            atlas,
            radar: None,
            map_revision: 0,
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
        Some(format!("{}({bounty})", p.name))
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
            return self.spec_cam;
        }
        let me = self.me();
        view::lerp_pos(me.prev_pos, me.pos, alpha)
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
        self.ui_input(&mut edges, &pressed, &typed);
        self.big_radar = !self.menu && input.action_held(Action::BigRadar);
        if self.spectating && !self.menu && self.typing.is_none() {
            let step = SPEC_SPEED * dt.as_secs_f32() * if input.shift() { 3.0 } else { 1.0 };
            let (dx, dy) = (
                input.action_held(Action::Right) as i32 as f32
                    - input.action_held(Action::Left) as i32 as f32,
                input.action_held(Action::Backward) as i32 as f32
                    - input.action_held(Action::Forward) as i32 as f32,
            );
            self.spec_cam.0 = (self.spec_cam.0 + dx * step).clamp(0.0, 16384.0);
            self.spec_cam.1 = (self.spec_cam.1 + dy * step).clamp(0.0, 16384.0);
        }

        self.pump_network();

        self.acc += dt;
        let mut ticks = 0;
        while self.acc >= TICK {
            let mut ship_input = if self.menu || self.spectating {
                ShipInput::default()
            } else {
                input.ship_input(&edges)
            };
            if self.aim_bot
                && let Some(aimed) = self.aim_input()
            {
                ship_input = aimed;
            }
            // Edge-triggered actions apply to one tick only.
            edges.clear();
            let mut inputs = vec![ShipInput::default(); self.world.ships.len()];
            let me = self.me_index();
            if let Some(slot) = inputs.get_mut(me) {
                *slot = ship_input;
            }
            let events = self.world.step(&inputs);
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
        self.update_engine_sound();
    }

    /// Turns toward the nearest enemy, closes in and fires (testing aid).
    fn aim_input(&self) -> Option<ShipInput> {
        let me = self.me();
        if !me.is_alive() {
            return None;
        }
        let (mx, my) = me.pos.to_f32();
        let target = self
            .world
            .ships
            .iter()
            .filter(|s| s.is_alive() && s.freq != me.freq)
            .map(|s| {
                let (x, y) = s.pos.to_f32();
                (x - mx, y - my)
            })
            .min_by(|a, b| (a.0.hypot(a.1)).total_cmp(&b.0.hypot(b.1)))?;
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
    fn ui_input(&mut self, edges: &mut Vec<Action>, pressed: &[KeyCode], typed: &str) {
        if let Some(line) = self.typing.as_mut() {
            for c in typed.chars() {
                if !c.is_control() && line.chars().count() < MAX_CHAT {
                    line.push(c);
                }
            }
            let mut send = false;
            let mut cancel = false;
            for key in pressed {
                match key {
                    KeyCode::Backspace => {
                        line.pop();
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
        for e in edges.iter() {
            match e {
                Action::ShowFps => self.show_fps = !self.show_fps,
                Action::ZoomIn => self.scale = (self.scale + 1).min(8),
                Action::ZoomOut => self.scale = self.scale.saturating_sub(1).max(1),
                _ => {}
            }
        }
    }

    /// Sends (or, offline, handles) a typed chat line: `'text` team, `:name:text`
    /// private, `?command` commands, anything else public.
    fn send_chat_line(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        let Some(net) = self.net.as_mut() else {
            self.local_command(line);
            return;
        };
        // "=N" changes team, as in Continuum.
        if let Some(freq) = line.strip_prefix('=').and_then(|f| f.trim().parse::<i16>().ok()) {
            net.session.set_freq(freq);
            net.flush();
            return;
        }
        let me_name = net.session.me().map(|p| p.name.clone()).unwrap_or_default();
        let (kind, target, text, shown, colour) = if let Some(t) = line.strip_prefix('\'') {
            (
                chat_type::FREQ,
                0,
                t.to_string(),
                format!("{me_name}> {t}"),
                color::YELLOW,
            )
        } else if let Some(rest) = line.strip_prefix(':')
            && let Some((name, t)) = rest.split_once(':')
        {
            let pid = net
                .session
                .players()
                .find(|p| p.name.eq_ignore_ascii_case(name))
                .map(|p| p.pid);
            match pid {
                Some(pid) => (
                    chat_type::PRIVATE,
                    pid,
                    t.to_string(),
                    format!("{me_name}> {t}"),
                    color::GREEN,
                ),
                None => {
                    self.message(format!("No player named {name} here."), color::YELLOW);
                    return;
                }
            }
        } else {
            (
                chat_type::PUBLIC,
                0,
                line.to_string(),
                format!("{me_name}> {line}"),
                color::WHITE,
            )
        };
        net.session.send_chat(kind, target, &text);
        net.flush();
        // Servers don't echo your own messages; commands get their reply instead.
        if !line.starts_with('?') && !line.starts_with('*') {
            self.message(shown, colour);
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
                SessionEvent::Kill { killer, killed, .. } => {
                    let name = |pid: i16, s: &Self| {
                        s.net
                            .as_ref()
                            .and_then(|n| n.session.player(pid))
                            .map(|p| p.name.clone())
                            .unwrap_or_default()
                    };
                    let (kn, dn) = (name(killer, self), name(killed, self));
                    self.message(format!("{dn} killed by {kn}"), color::GREEN);
                    if killed as u16 != self.me_id {
                        self.world
                            .kill_ship(killed as u16, killer as u16, &mut sim_events);
                    }
                }
                SessionEvent::Chat {
                    kind, name, text, ..
                } => {
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
                    self.world.settings = ArenaSettings::from_client_settings(&cs);
                }
                SessionEvent::Seeds { green_seed, .. } => self.world.prize_seed = green_seed,
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
        self.handle_events(&sim_events);
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
                Event::ShipKilled { pos, .. } => {
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
        view::draw_world(self, &c, r, alpha);
        hud::draw(self, &c, r);
        self.draw_log(&c, r);
        if self.menu {
            self.draw_menu(&c, r);
        }
        StarParams {
            camera: [cam.0, cam.1],
            scale: c.scale,
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
        if self.spectating {
            let msg = "Spectating: arrows move, Esc then 1-8 to play";
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
