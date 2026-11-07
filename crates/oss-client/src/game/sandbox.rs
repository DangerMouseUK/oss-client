//! Offline sandbox: fly a ship around a real map with stock settings.
//!
//! The simulation runs at a fixed 100 Hz; rendering runs as fast as the display allows
//! and interpolates positions between the last two ticks.

use std::time::{Duration, Instant};

use oss_assets::ContinuumDir;
use oss_sim::{
    ArenaSettings, Event, MapObject, Pos, ProjectileKind, Ship, TileMap, World,
    fixed::ROTATION_UNITS, tile,
};
use winit::keyboard::KeyCode;

use super::art::{self, Art, anim_frame, once_frame};
use super::input::InputState;
use crate::audio::Audio;
use crate::gfx::{Region, Renderer, StarParams, TextureId};

const TICK: Duration = Duration::from_millis(10);
/// Most sim ticks run per frame before we give up catching up (e.g. after a stall).
const MAX_TICKS_PER_FRAME: u32 = 25;
/// Continuum's default `Radar:MapZoomFactor`.
const MAP_ZOOM: u32 = 10;
const RADAR_MARGIN: f32 = 6.0;

#[derive(Debug, Clone, Copy)]
enum EffectKind {
    BulletExplosion,
    BombExplosion,
    ShipExplosion,
    Warp,
    Exhaust,
}

#[derive(Debug, Clone, Copy)]
struct Effect {
    kind: EffectKind,
    /// Centre in game pixels.
    x: f32,
    y: f32,
    start: f32,
}

impl EffectKind {
    /// (frame width, frame height, duration seconds)
    fn params(self) -> (u32, u32, f32) {
        match self {
            EffectKind::BulletExplosion => (16, 16, 0.4),
            EffectKind::BombExplosion => (80, 80, 1.25),
            EffectKind::ShipExplosion => (48, 48, 0.8),
            EffectKind::Warp => (48, 48, 0.5),
            EffectKind::Exhaust => (16, 16, 0.3),
        }
    }
}

struct Radar {
    texture: TextureId,
    /// Side of the whole-map radar image in pixels.
    full_dim: u32,
    /// UI width it was built for.
    built_for: u32,
}

pub struct Sandbox {
    world: World,
    me: usize,
    objects: Vec<MapObject>,
    art: Art,
    atlas: TextureId,
    radar: Option<Radar>,
    last: Instant,
    acc: Duration,
    clock: f32,
    effects: Vec<Effect>,
    menu: bool,
    show_fps: bool,
    fps: FpsCounter,
    pub quit: bool,
    /// Integer pixel scale (game pixel → physical pixels).
    pub scale: u32,
    pending_multifire: bool,
    audio: Option<Audio>,
}

/// Sounds the sandbox uses, loaded up front.
const SOUNDS: &[&str] = &[
    "gun1", "gun2", "gun3", "gun4", "bomb1", "bomb2", "bomb3", "bomb4", "mine1", "mine2", "mine3",
    "mine4", "explode0", "explode1", "explode2", "bounce", "warp", "multion", "multioff", "thrust",
    "rev",
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
            last: Instant::now(),
            acc: Duration::ZERO,
            clock: 0.0,
            effects: Vec::new(),
            menu: false,
            show_fps: false,
            fps: FpsCounter::default(),
            quit: false,
            scale: scale.max(1),
            pending_multifire: false,
            audio,
        };
        s.spawn_warp_effect();
        Ok(s)
    }

    fn me(&self) -> &Ship {
        &self.world.ships[self.me]
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

    /// Handles menu keys and advances the simulation to the current time.
    pub fn update(&mut self, input: &mut InputState) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last)
            .min(Duration::from_millis(250));
        self.last = now;
        self.clock += dt.as_secs_f32();
        self.fps.frame(now);

        for key in input.take_pressed() {
            if self.menu {
                match key {
                    KeyCode::Escape => self.menu = false,
                    KeyCode::KeyQ => self.quit = true,
                    KeyCode::Digit1
                    | KeyCode::Digit2
                    | KeyCode::Digit3
                    | KeyCode::Digit4
                    | KeyCode::Digit5
                    | KeyCode::Digit6
                    | KeyCode::Digit7
                    | KeyCode::Digit8 => {
                        let kind = digit(key).unwrap_or(1) - 1;
                        self.change_ship(kind);
                        self.menu = false;
                    }
                    _ => {}
                }
                continue;
            }
            match key {
                KeyCode::Escape => self.menu = true,
                KeyCode::Delete => self.pending_multifire = true,
                KeyCode::F12 => self.show_fps = !self.show_fps,
                KeyCode::Equal | KeyCode::NumpadAdd if ctrl(input) => {
                    self.scale = (self.scale + 1).min(8)
                }
                KeyCode::Minus | KeyCode::NumpadSubtract if ctrl(input) => {
                    self.scale = self.scale.saturating_sub(1).max(1)
                }
                _ => {}
            }
        }

        self.acc += dt;
        let mut ticks = 0;
        while self.acc >= TICK {
            let ship_input = if self.menu {
                Default::default()
            } else {
                input.ship_input(std::mem::take(&mut self.pending_multifire))
            };
            let events = self.world.step(&[ship_input]);
            self.handle_events(&events);
            self.spawn_exhaust();
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
        let ship_pos = |w: &World, id: u16| w.ships.iter().find(|s| s.id == id).map(|s| s.pos);
        for e in events {
            match e {
                Event::FiredGun { ship, level, .. } => {
                    let at = ship_pos(&self.world, *ship);
                    self.sound(&format!("gun{}", (*level).min(3) + 1), 0.8, at);
                }
                Event::FiredBomb { ship, level } => {
                    let at = ship_pos(&self.world, *ship);
                    self.sound(&format!("bomb{}", (*level).min(3) + 1), 0.9, at);
                }
                Event::LaidMine { ship, level } => {
                    let at = ship_pos(&self.world, *ship);
                    self.sound(&format!("mine{}", (*level).min(3) + 1), 0.9, at);
                }
                Event::MultifireToggled { on, .. } => {
                    self.sound(if *on { "multion" } else { "multioff" }, 0.8, None);
                }
                // Continuum only plays the bounce sound for hard hits (>= 3 tiles/s).
                Event::WallBounce { ship, speed } if *ship == self.me().id && *speed >= 480 => {
                    self.sound("bounce", 0.7, None);
                }
                Event::Explosion { pos, kind, .. } => {
                    let (x, y) = pos.to_f32();
                    let (kind, name) = match kind {
                        ProjectileKind::Bullet { .. } => (EffectKind::BulletExplosion, "explode0"),
                        _ => (EffectKind::BombExplosion, "explode2"),
                    };
                    self.effect(kind, x, y);
                    self.sound(name, 0.9, Some(*pos));
                }
                Event::ShipKilled { pos, .. } => {
                    let (x, y) = pos.to_f32();
                    self.effect(EffectKind::ShipExplosion, x, y);
                    self.sound("explode1", 1.0, Some(*pos));
                }
                Event::Respawned { ship } if *ship == self.me().id => self.spawn_warp_effect(),
                _ => {}
            }
        }
    }

    fn spawn_exhaust(&mut self) {
        let ship = self.me();
        if !ship.is_alive() || ship.thrusting == 0 || !self.world.tick.is_multiple_of(4) {
            return;
        }
        let h = oss_sim::fixed::DIRECTION_HEADINGS[ship.direction() as usize];
        let back = if ship.thrusting > 0 { -1.0 } else { 1.0 };
        let (x, y) = ship.pos.to_f32();
        let one = oss_sim::fixed::UNIT_ONE as f32;
        let (hx, hy) = (h.x as f32 / one, h.y as f32 / one);
        self.effects.push(Effect {
            kind: EffectKind::Exhaust,
            x: x + hx * 14.0 * back,
            y: y + hy * 14.0 * back,
            start: self.clock,
        });
    }

    /// Queues the whole frame on the renderer and returns the starfield parameters.
    pub fn draw(&mut self, r: &mut Renderer) -> StarParams {
        let (sw, sh) = r.size();
        let scale = self.scale as f32;
        let view_w = sw as f32 / scale;
        let view_h = sh as f32 / scale;
        let alpha = self.acc.as_secs_f32() / TICK.as_secs_f32();

        let me = self.me().clone();
        let cam = lerp_pos(me.prev_pos, me.pos, alpha);
        let cam_phys = ((cam.0 * scale).round(), (cam.1 * scale).round());
        let half = ((sw as f32 / 2.0).floor(), (sh as f32 / 2.0).floor());
        let to_screen = |x: f32, y: f32| {
            (
                (x * scale).round() - cam_phys.0 + half.0,
                (y * scale).round() - cam_phys.1 + half.1,
            )
        };
        let atlas = self.atlas;
        let white = [1.0; 4];

        // Draws a region with its top-left at world pixel (x, y).
        let sprite = |r: &mut Renderer, region: Region, x: f32, y: f32| {
            if region.w == 0 {
                return;
            }
            let (px, py) = to_screen(x, y);
            r.quad(
                atlas,
                region,
                px,
                py,
                region.w as f32 * scale,
                region.h as f32 * scale,
                white,
            );
        };

        // --- Tiles ---------------------------------------------------------------------
        let tx0 = ((cam.0 - view_w / 2.0) / 16.0).floor() as i32 - 1;
        let ty0 = ((cam.1 - view_h / 2.0) / 16.0).floor() as i32 - 1;
        let tx1 = ((cam.0 + view_w / 2.0) / 16.0).ceil() as i32 + 1;
        let ty1 = ((cam.1 + view_h / 2.0) / 16.0).ceil() as i32 + 1;
        let tile_at = |x: i32, y: i32| -> u8 {
            if (-1..=1024).contains(&x) && (-1..=1024).contains(&y) {
                self.world.map.get(x, y)
            } else {
                0
            }
        };
        for ty in ty0.max(-1)..=ty1.min(1024) {
            for tx in tx0.max(-1)..=tx1.min(1024) {
                let id = tile_at(tx, ty);
                if base_layer_tile(id) {
                    let shown = if (tile::FIRST_DOOR..=tile::LAST_DOOR).contains(&id) {
                        // Closed doors cycle through their group of four tiles.
                        let group = (id - tile::FIRST_DOOR) / 4;
                        let frame = ((self.clock / 0.4).fract() * 4.0) as u8 % 4;
                        tile::FIRST_DOOR + group * 4 + frame
                    } else {
                        id
                    };
                    sprite(r, self.art.tile(shown), (tx * 16) as f32, (ty * 16) as f32);
                }
            }
        }

        // --- Animated map objects ------------------------------------------------------
        let t = self.clock;
        for obj in &self.objects {
            let (ox, oy) = (obj.x as i32, obj.y as i32);
            if ox < tx0 - 6 || ox > tx1 || oy < ty0 - 6 || oy > ty1 {
                continue;
            }
            let a = &self.art;
            let region = match obj.id {
                tile::ASTEROID_SMALL_1 => {
                    anim_frame(a.over1, 16, 16, 0, a.over1.frame_count(16, 16), 1.5, t)
                }
                tile::ASTEROID_LARGE => {
                    anim_frame(a.over2, 32, 32, 0, a.over2.frame_count(32, 32), 1.5, t)
                }
                tile::ASTEROID_SMALL_2 => {
                    anim_frame(a.over3, 16, 16, 0, a.over3.frame_count(16, 16), 1.5, t)
                }
                tile::SPACE_STATION => {
                    anim_frame(a.over4, 96, 96, 0, a.over4.frame_count(96, 96), 1.0, t)
                }
                tile::WORMHOLE => {
                    anim_frame(a.over5, 80, 80, 0, a.over5.frame_count(80, 80), 2.5, t)
                }
                tile::GOAL => {
                    let n = a.goal.frame_count(16, 16) / 2;
                    anim_frame(a.goal, 16, 16, n, n, 0.5, t)
                }
                _ => continue,
            };
            sprite(r, region, (ox * 16) as f32, (oy * 16) as f32);
        }

        // --- Projectiles ---------------------------------------------------------------
        for pr in &self.world.projectiles {
            let (x, y) = lerp_pos(pr.prev_pos, pr.pos, alpha);
            let a = &self.art;
            let lvl = pr.level.min(3) as u32;
            let (region, size) = match pr.kind {
                ProjectileKind::Bullet { bouncing } => {
                    let first = lvl * 4 + if bouncing { 20 } else { 0 };
                    (anim_frame(a.bullets, 5, 5, first, 4, 0.15, t), 5.0)
                }
                ProjectileKind::Bomb { bounces_left } => {
                    let first = lvl * 10 + if bounces_left > 0 { 80 } else { 0 };
                    (anim_frame(a.bombs, 16, 16, first, 10, 1.0, t), 16.0)
                }
                ProjectileKind::Mine => (anim_frame(a.mines, 16, 16, lvl * 10, 10, 1.0, t), 16.0),
            };
            sprite(
                r,
                region,
                (x - size / 2.0).round(),
                (y - size / 2.0).round(),
            );
        }

        // --- Exhaust (under ships) -----------------------------------------------------
        for e in self
            .effects
            .iter()
            .filter(|e| matches!(e.kind, EffectKind::Exhaust))
        {
            let n = self.art.exhaust.frame_count(16, 16) / 2;
            let dt = t - e.start;
            if n > 0 && dt < 0.3 {
                let i = ((dt / 0.3 * n as f32) as u32).min(n - 1);
                sprite(
                    r,
                    self.art.exhaust.frame(16, 16, i),
                    (e.x - 8.0).round(),
                    (e.y - 8.0).round(),
                );
            }
        }

        // --- Ships ---------------------------------------------------------------------
        for ship in &self.world.ships {
            if !ship.is_alive() {
                continue;
            }
            let (x, y) = lerp_pos(ship.prev_pos, ship.pos, alpha);
            let dir = lerp_direction(ship.prev_rotation, ship.rotation, alpha);
            sprite(
                r,
                self.art.ship(ship.kind, dir),
                (x - 18.0).round(),
                (y - 18.0).round(),
            );
        }

        // --- Fly-under tiles (drawn over ships) ----------------------------------------
        for ty in ty0.max(-1)..=ty1.min(1024) {
            for tx in tx0.max(-1)..=tx1.min(1024) {
                let id = tile_at(tx, ty);
                if (tile::FIRST_FLY_UNDER..=tile::LAST_FLY_UNDER).contains(&id) {
                    sprite(r, self.art.tile(id), (tx * 16) as f32, (ty * 16) as f32);
                }
            }
        }

        // --- Effects -------------------------------------------------------------------
        for e in &self.effects {
            let sheet = match e.kind {
                EffectKind::BulletExplosion => self.art.explode0,
                EffectKind::BombExplosion => self.art.explode2,
                EffectKind::ShipExplosion => self.art.explode1,
                EffectKind::Warp => self.art.warp,
                EffectKind::Exhaust => continue,
            };
            let (fw, fh, dur) = e.kind.params();
            if let Some(region) = once_frame(sheet, fw, fh, dur, t - e.start) {
                sprite(
                    r,
                    region,
                    (e.x - fw as f32 / 2.0).round(),
                    (e.y - fh as f32 / 2.0).round(),
                );
            }
        }

        // --- HUD -----------------------------------------------------------------------
        self.draw_energy(r, view_w);
        self.draw_radar(r, view_w, view_h);
        self.draw_text_overlays(r, view_w, view_h);

        StarParams {
            camera: [cam.0, cam.1],
            scale,
        }
    }

    /// Queues a region at UI position (game pixels from the top-left), optionally
    /// stretched to `w`×`h` game pixels and tinted.
    fn ui(
        &self,
        r: &mut Renderer,
        region: Region,
        x: f32,
        y: f32,
        size: Option<(f32, f32)>,
        color: [f32; 4],
    ) {
        if region.w == 0 {
            return;
        }
        let s = self.scale as f32;
        let (w, h) = size.unwrap_or((region.w as f32, region.h as f32));
        r.quad(
            self.atlas,
            region,
            (x * s).round(),
            (y * s).round(),
            (w * s).round(),
            (h * s).round(),
            color,
        );
    }

    fn rect(&self, r: &mut Renderer, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.ui(r, self.art.white, x, y, Some((w, h)), color);
    }

    fn draw_energy(&self, r: &mut Renderer, view_w: f32) {
        let me = self.me();
        let settings = &self.world.settings.ships[me.kind as usize];
        let white = [1.0; 4];
        let center_x = (view_w / 2.0).floor();
        let center_y = 14.0;
        let g = self.art.gradient;
        let max_possible = settings.maximum_energy.max(1) as f32;
        let ship_max = me.stats.max_energy.max(1) as f32;
        let energy = me.energy.max(0) as f32 / 1000.0;
        let frac = (energy / ship_max).clamp(0.0, 1.0);

        if g.w > 0 {
            // Background track, current-max track, then the pulsing energy bar.
            let full = g.frame(1, 1, 5);
            let top = g.frame(1, 1, 0);
            let top_w = ship_max / max_possible * 240.0;
            self.ui(
                r,
                full,
                center_x - 240.0,
                center_y - 4.0,
                Some((480.0, 2.0)),
                white,
            );
            self.ui(
                r,
                top,
                center_x - top_w,
                center_y - 4.0,
                Some((top_w * 2.0, 2.0)),
                white,
            );
            let row = if frac > 0.5 {
                0
            } else if frac > 0.25 {
                1
            } else {
                2
            };
            let bar = anim_frame(g, 1, 1, row * 14, 14, 1.4, self.clock);
            let w = (frac * top_w).round();
            self.ui(r, bar, center_x - w, center_y, Some((w * 2.0, 6.0)), white);
        }
        if self.art.hlthbar.w > 0 {
            let frame = self.art.hlthbar.frame(116, 28, 0);
            self.ui(r, frame, center_x - 58.0, 0.0, None, white);
        }

        // Energy number, right-aligned at the top right.
        if self.art.engyfont.w > 0 {
            let mut value = energy as u32;
            let mut i = 1.0;
            loop {
                let d = value % 10;
                self.ui(
                    r,
                    self.art.engyfont.frame(16, 24, d),
                    view_w - i * 16.0,
                    0.0,
                    None,
                    white,
                );
                value /= 10;
                i += 1.0;
                if value == 0 {
                    break;
                }
            }
        }
    }

    fn ensure_radar(&mut self, r: &mut Renderer, view_w: u32) {
        if self.radar.as_ref().is_some_and(|rd| rd.built_for == view_w) {
            return;
        }
        let full_dim = (view_w * 8 / MAP_ZOOM).clamp(64, 4096);
        let pixels = radar_image(&self.world.map, full_dim);
        match &mut self.radar {
            Some(rd) => {
                r.recreate_texture(rd.texture, full_dim, full_dim, &pixels);
                rd.full_dim = full_dim;
                rd.built_for = view_w;
            }
            None => {
                let texture = r.create_texture(full_dim, full_dim, &pixels);
                self.radar = Some(Radar {
                    texture,
                    full_dim,
                    built_for: view_w,
                });
            }
        }
    }

    fn draw_radar(&mut self, r: &mut Renderer, view_w: f32, view_h: f32) {
        self.ensure_radar(r, view_w as u32);
        let Some(radar) = self.radar.as_ref() else {
            return;
        };
        let full = radar.full_dim as f32;
        // Radar box: about a sixth of the screen width, as in Continuum.
        let dim = ((((view_w as u32 / 6) / 4) * 8) / 2) as f32;
        if dim < 8.0 {
            return;
        }
        let me = self.me();
        let p = me.pos.pixels();
        let to_radar = |px: i32| px as f32 * full / 16384.0;
        let min_x = (to_radar(p.x) - dim / 2.0)
            .floor()
            .clamp(0.0, (full - dim).max(0.0));
        let min_y = (to_radar(p.y) - dim / 2.0)
            .floor()
            .clamp(0.0, (full - dim).max(0.0));
        let x0 = view_w - dim - RADAR_MARGIN;
        let y0 = view_h - dim - RADAR_MARGIN;
        let s = self.scale as f32;
        let region = Region {
            x: min_x as u32,
            y: min_y as u32,
            w: dim as u32,
            h: dim as u32,
        };
        r.quad(
            radar.texture,
            region,
            (x0 * s).round(),
            (y0 * s).round(),
            dim * s,
            dim * s,
            [1.0; 4],
        );

        // Ships: you blink yellow, team yellow, enemies blue.
        let blink = (self.clock * 2.5).fract() < 0.5;
        for ship in &self.world.ships {
            if !ship.is_alive() {
                continue;
            }
            let sp = ship.pos.pixels();
            let (rx, ry) = (to_radar(sp.x) - min_x, to_radar(sp.y) - min_y);
            if rx < 0.0 || ry < 0.0 || rx >= dim - 1.0 || ry >= dim - 1.0 {
                continue;
            }
            let is_me = ship.id == me.id;
            if is_me && !blink {
                continue;
            }
            let color = if ship.freq == me.freq {
                [1.0, 1.0, 0.0, 1.0]
            } else {
                [0.3, 0.5, 1.0, 1.0]
            };
            self.rect(r, x0 + rx.floor(), y0 + ry.floor(), 2.0, 2.0, color);
        }

        // Border.
        let c = [0.45, 0.45, 0.5, 1.0];
        self.rect(r, x0 - 1.0, y0 - 1.0, dim + 2.0, 1.0, c);
        self.rect(r, x0 - 1.0, y0 + dim, dim + 2.0, 1.0, c);
        self.rect(r, x0 - 1.0, y0, 1.0, dim, c);
        self.rect(r, x0 + dim, y0, 1.0, dim, c);
    }

    /// Draws text with Continuum's tall font. `color` selects the font row (0 = grey,
    /// 1 = green, 2 = blue, 3 = red, 4 = yellow, 5 = fuchsia...).
    fn text(&self, r: &mut Renderer, x: f32, y: f32, color: u32, s: &str) {
        let font = self.art.tallfont;
        if font.w == 0 {
            return;
        }
        for (i, ch) in s.chars().enumerate() {
            let c = ch as u32;
            if !(32..128).contains(&c) {
                continue;
            }
            let glyph = font.frame(8, 12, color * 96 + (c - 32));
            self.ui(r, glyph, x + i as f32 * 8.0, y, None, [1.0; 4]);
        }
    }

    fn draw_text_overlays(&self, r: &mut Renderer, view_w: f32, view_h: f32) {
        let me = self.me();
        if !me.is_alive() {
            let msg = "You have been destroyed";
            self.text(
                r,
                ((view_w - msg.len() as f32 * 8.0) / 2.0).floor(),
                (view_h / 2.0 - 40.0).floor(),
                3,
                msg,
            );
        }
        let mf = if me.multifire { "Multifire on" } else { "" };
        self.text(r, 4.0, view_h - 16.0, 1, mf);
        if self.show_fps {
            let line = format!(
                "{:.0} fps  scale {}x  {:?}  tick {}",
                self.fps.fps, self.scale, r.present_mode, self.world.tick
            );
            self.text(r, 4.0, 4.0, 0, &line);
        }
        if self.menu {
            let lines = [
                "Sandbox",
                "",
                "1-8   Change ship",
                "Q     Quit",
                "Esc   Back to game",
                "",
                "Arrows fly, Shift afterburner, Ctrl guns,",
                "Tab bombs, Shift+Tab mines, Delete multifire,",
                "Ctrl +/- pixel scale, F12 fps",
            ];
            let w = 46.0 * 8.0;
            let h = lines.len() as f32 * 14.0 + 16.0;
            let (x, y) = (((view_w - w) / 2.0).floor(), ((view_h - h) / 2.0).floor());
            self.rect(r, x - 8.0, y - 8.0, w + 16.0, h, [0.0, 0.0, 0.0, 0.85]);
            for (i, line) in lines.iter().enumerate() {
                self.text(r, x, y + i as f32 * 14.0, if i == 0 { 4 } else { 0 }, line);
            }
        }
    }
}

fn ctrl(input: &InputState) -> bool {
    input.held(KeyCode::ControlLeft) || input.held(KeyCode::ControlRight)
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

/// Tiles drawn in the main tile pass (animated objects and fly-under tiles are drawn
/// separately; ids above 190 have no tileset image).
fn base_layer_tile(id: u8) -> bool {
    matches!(id, 1..=169 | tile::SAFE | tile::FIRST_FLY_OVER..=tile::LAST_FLY_OVER)
}

fn lerp_pos(a: Pos, b: Pos, t: f32) -> (f32, f32) {
    let (ax, ay) = a.to_f32();
    let (bx, by) = b.to_f32();
    // Large jumps (respawn, warp) snap instead of sliding across the map.
    if (bx - ax).abs() > 200.0 || (by - ay).abs() > 200.0 {
        return (bx, by);
    }
    (ax + (bx - ax) * t, ay + (by - ay) * t)
}

fn lerp_direction(prev: u32, cur: u32, t: f32) -> u8 {
    let n = ROTATION_UNITS as i64;
    let mut d = cur as i64 - prev as i64;
    if d > n / 2 {
        d -= n;
    } else if d < -n / 2 {
        d += n;
    }
    let r = (prev as i64 + (d as f32 * t) as i64).rem_euclid(n);
    (r / 1000) as u8
}

/// Builds the whole-map radar image, `dim`×`dim` RGBA. Colours match Continuum's radar.
pub fn radar_image(map: &TileMap, dim: u32) -> Vec<u8> {
    const BG: [u8; 4] = [10, 25, 10, 255];
    let color = |id: u8| -> [u8; 4] {
        match id {
            0 | 242.. => BG,
            tile::SAFE => [24, 82, 24, 255],
            tile::GOAL => [255, 57, 8, 255],
            tile::FIRST_DOOR..=tile::LAST_DOOR => [173, 173, 173, 255],
            _ => [90, 90, 90, 255],
        }
    };
    let n = dim as usize;
    let mut out = vec![0u8; n * n * 4];
    for px in out.as_chunks_mut::<4>().0 {
        *px = BG;
    }
    if dim < 1024 {
        // Downsampling: draw every non-empty tile so thin walls are never lost.
        for y in 0..1024u32 {
            let dy = (y * dim / 1024) as usize;
            for x in 0..1024u32 {
                let id = map.get(x as i32, y as i32);
                if id == 0 || id > 241 {
                    continue;
                }
                let i = (dy * n + (x * dim / 1024) as usize) * 4;
                out[i..i + 4].copy_from_slice(&color(id));
            }
        }
    } else {
        for gy in 0..dim {
            let y = gy * 1024 / dim;
            for gx in 0..dim {
                let id = map.get((gx * 1024 / dim) as i32, y as i32);
                let i = (gy as usize * n + gx as usize) * 4;
                out[i..i + 4].copy_from_slice(&color(id));
            }
        }
    }
    out
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
    use super::*;

    #[test]
    fn radar_keeps_thin_walls_when_downsampling() {
        let mut map = TileMap::empty();
        map.set(500, 500, 1);
        let img = radar_image(&map, 512);
        let i = (250 * 512 + 250) * 4;
        assert_eq!(&img[i..i + 4], &[90, 90, 90, 255]);
    }

    #[test]
    fn direction_interpolates_the_short_way() {
        assert_eq!(lerp_direction(39_500, 500, 0.5), 0);
        assert_eq!(lerp_direction(0, 10_000, 0.5), 5);
    }
}
