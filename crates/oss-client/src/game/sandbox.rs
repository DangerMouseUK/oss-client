//! Offline sandbox: fly a ship around a real map with stock zone settings.
//!
//! The simulation runs at a fixed 100 Hz; rendering runs as fast as the display allows
//! and interpolates positions between the last two ticks.

use std::time::{Duration, Instant};

use oss_assets::ContinuumDir;
use oss_sim::{
    ArenaSettings, Event, MapObject, Pos, Prize, ProjectileKind, Ship, TileMap, World, toggle,
};
use winit::keyboard::KeyCode;

use super::art::{self, Art};
use super::hud::{self, RadarCache};
use super::input::{Action, InputState};
use super::view::{self, Canvas, color};
use crate::audio::Audio;
use crate::gfx::{Renderer, StarParams, TextureId};

const TICK: Duration = Duration::from_millis(10);
/// Most sim ticks run per frame before we give up catching up (e.g. after a stall).
const MAX_TICKS_PER_FRAME: u32 = 25;
/// Messages kept in the log.
const MAX_MESSAGES: usize = 64;

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
    pub(super) me: usize,
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
}

/// Sounds the sandbox uses, loaded up front.
const SOUNDS: &[&str] = &[
    "gun1", "gun2", "gun3", "gun4", "bomb1", "bomb2", "bomb3", "bomb4", "ebomb1", "ebomb2",
    "ebomb3", "ebomb4", "mine1", "mine2", "mine3", "mine4", "explode0", "explode1", "explode2",
    "ebombex", "bounce", "warp", "multion", "multioff", "thrust", "rev", "prize", "repel", "burst",
    "thor", "decoy", "rocket1", "stealth", "cloak", "xradar", "antiwarp", "off", "warppnt",
];

/// Distance (pixels) at which positional sounds fade out completely.
const HEARING_RANGE: f32 = 1200.0;

impl Sandbox {
    pub fn new(
        renderer: &mut Renderer,
        dir: &ContinuumDir,
        level_bytes: &[u8],
        ship: u8,
        scale: u32,
        mut audio: Option<Audio>,
    ) -> Result<Self, String> {
        if let Some(a) = audio.as_mut() {
            a.preload(SOUNDS);
        }
        let level = oss_formats::Level::parse(level_bytes).map_err(|e| format!("map: {e}"))?;
        let (map, objects) = TileMap::from_entries(level.tiles.iter().map(|t| (t.x, t.y, t.tile)));
        let (art, atlas_size, atlas_pixels) =
            art::load(dir, level.tileset.as_ref().map(|t| t.bytes.as_slice()))?;
        let atlas = renderer.create_texture(atlas_size, atlas_size, &atlas_pixels);

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
            .unwrap_or(1);
        let mut world = World::new(map, ArenaSettings::sandbox(), seed);
        let me = world.add_ship(0, ship.min(7), 0);
        let mut s = Self {
            world,
            me,
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
        };
        s.spawn_warp_effect();
        s.message(
            "Welcome to the Open SubSpace sandbox. Press Esc for the menu.",
            color::GREEN,
        );
        Ok(s)
    }

    pub(super) fn me(&self) -> &Ship {
        &self.world.ships[self.me]
    }

    pub(super) fn fps(&self) -> f32 {
        self.fps.fps
    }

    pub fn message(&mut self, text: impl Into<String>, color: u32) {
        self.messages.push(Message {
            text: text.into(),
            color,
        });
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
            let (mx, my) = self.me().pos.to_f32();
            let (dx, dy) = (x - mx, y - my);
            gain *= (1.0 - (dx * dx + dy * dy).sqrt() / HEARING_RANGE).max(0.0);
            pan = (dx / 600.0).clamp(-0.8, 0.8);
        }
        if let Some(a) = self.audio.as_mut() {
            a.play(name, gain, pan);
        }
    }

    /// Handles UI keys and advances the simulation to the current time.
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
        if edges.contains(&Action::Menu) {
            self.menu = !self.menu;
            edges.clear();
        } else if self.menu {
            for key in pressed {
                match key {
                    KeyCode::KeyQ => self.quit = true,
                    k => {
                        if let Some(n) = digit(k) {
                            self.change_ship(n - 1);
                            self.menu = false;
                        }
                    }
                }
            }
            edges.clear();
        }
        for e in &edges {
            match e {
                Action::ShowFps => self.show_fps = !self.show_fps,
                Action::ZoomIn => self.scale = (self.scale + 1).min(8),
                Action::ZoomOut => self.scale = self.scale.saturating_sub(1).max(1),
                _ => {}
            }
        }
        self.big_radar = !self.menu && input.action_held(Action::BigRadar);

        self.acc += dt;
        let mut ticks = 0;
        while self.acc >= TICK {
            let ship_input = if self.menu {
                Default::default()
            } else {
                input.ship_input(&edges)
            };
            // Edge-triggered actions apply to one tick only.
            edges.clear();
            let events = self.world.step(&[ship_input]);
            self.handle_events(&events);
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
        let clock = self.clock;
        self.effects.retain(|e| clock - e.start < e.kind.params().2);
        self.update_engine_sound();
    }

    fn update_engine_sound(&mut self) {
        let ship = &self.world.ships[self.me];
        let (fwd, rev) = match (ship.is_alive(), ship.thrusting) {
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

    fn change_ship(&mut self, kind: u8) {
        let id = self.me().id;
        let freq = self.me().freq;
        self.world.ships.remove(self.me);
        self.me = self.world.add_ship(id, kind, freq);
        self.spawn_warp_effect();
    }

    fn handle_events(&mut self, events: &[Event]) {
        let me = self.me().id;
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
                Event::BrickDropped { .. } => self.map_revision += 1,
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
        let ship = self.me();
        if !ship.is_alive() || ship.thrusting == 0 || !self.world.tick.is_multiple_of(4) {
            return;
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
        self.effect(kind, x + hx * 14.0 * back, y + hy * 14.0 * back);
    }

    /// Queues the whole frame on the renderer and returns the starfield parameters.
    pub fn draw(&mut self, r: &mut Renderer) -> StarParams {
        let alpha = self.acc.as_secs_f32() / TICK.as_secs_f32();
        let me = self.me();
        let cam = view::lerp_pos(me.prev_pos, me.pos, alpha);
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
        let lines = self.messages.iter().rev().take(6).collect::<Vec<_>>();
        for (i, m) in lines.iter().enumerate() {
            c.text(r, 4.0, c.view_h - 16.0 - i as f32 * 12.0, m.color, &m.text);
        }
    }

    fn draw_menu(&self, c: &Canvas, r: &mut Renderer) {
        let lines = [
            "Sandbox",
            "",
            "1-8   Change ship",
            "Q     Quit",
            "Esc   Back to game",
            "",
            "Arrows fly, Shift afterburner, Ctrl guns, Tab bombs,",
            "Shift+Tab mines, Shift+Ctrl repel, Delete multifire,",
            "Shift+Delete burst, Insert warp, Shift+Insert portal,",
            "Home stealth, Shift+Home cloak, End x-radar,",
            "Shift+End antiwarp, F3 rocket, F4 brick, F5 decoy,",
            "F6 thor, Alt big radar, Ctrl +/- scale, F12 fps",
        ];
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
