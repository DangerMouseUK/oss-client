//! Drawing the world: tiles, map objects, greens, bricks, projectiles, ships, effects.

use oss_sim::{Pos, ProjectileKind, fixed::ROTATION_UNITS, tile, toggle};

use oss_proto::objects::layer;

use super::art::{Art, anim_frame, once_frame};
use super::sandbox::{EffectKind, Sandbox};
use crate::gfx::{Region, Renderer, TextureId};

/// Font colours (rows of `tallfont`).
#[allow(dead_code)]
pub mod color {
    pub const WHITE: u32 = 0;
    pub const GREEN: u32 = 1;
    pub const BLUE: u32 = 2;
    pub const DARK_RED: u32 = 3;
    pub const YELLOW: u32 = 4;
    pub const FUCHSIA: u32 = 5;
    pub const RED: u32 = 6;
    pub const PINK: u32 = 7;
}

/// Maps game pixels to physical pixels for one frame.
pub struct Canvas {
    pub atlas: TextureId,
    pub white: Region,
    pub font: Region,
    pub scale: f32,
    /// Camera centre in game pixels.
    pub cam: (f32, f32),
    cam_phys: (f32, f32),
    half: (f32, f32),
    /// View size in game pixels.
    pub view_w: f32,
    pub view_h: f32,
}

impl Canvas {
    pub fn new(
        art: &Art,
        atlas: TextureId,
        screen: (u32, u32),
        scale: u32,
        cam: (f32, f32),
    ) -> Self {
        let scale = scale.max(1) as f32;
        Self {
            atlas,
            white: art.white,
            font: art.tallfont,
            scale,
            cam,
            cam_phys: ((cam.0 * scale).round(), (cam.1 * scale).round()),
            half: (
                (screen.0 as f32 / 2.0).floor(),
                (screen.1 as f32 / 2.0).floor(),
            ),
            view_w: screen.0 as f32 / scale,
            view_h: screen.1 as f32 / scale,
        }
    }

    fn to_screen(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x * self.scale).round() - self.cam_phys.0 + self.half.0,
            (y * self.scale).round() - self.cam_phys.1 + self.half.1,
        )
    }

    /// UI position (game pixels from the top-left of the screen) of world pixel (x, y).
    pub fn world_to_ui(&self, x: f32, y: f32) -> (f32, f32) {
        let (px, py) = self.to_screen(x, y);
        (px / self.scale, py / self.scale)
    }

    /// Draws `region` with its top-left at world pixel (x, y).
    pub fn world(&self, r: &mut Renderer, region: Region, x: f32, y: f32) {
        self.world_tint(r, region, x, y, [1.0; 4]);
    }

    pub fn world_tint(&self, r: &mut Renderer, region: Region, x: f32, y: f32, color: [f32; 4]) {
        if region.w == 0 {
            return;
        }
        let (px, py) = self.to_screen(x, y);
        r.quad(
            self.atlas,
            region,
            px,
            py,
            region.w as f32 * self.scale,
            region.h as f32 * self.scale,
            color,
        );
    }

    /// Draws `region` of `texture` with its top-left at world pixel (x, y).
    pub fn world_tex(&self, r: &mut Renderer, texture: TextureId, region: Region, x: f32, y: f32) {
        if region.w == 0 {
            return;
        }
        let (px, py) = self.to_screen(x, y);
        r.quad(
            texture,
            region,
            px,
            py,
            region.w as f32 * self.scale,
            region.h as f32 * self.scale,
            [1.0; 4],
        );
    }

    /// Draws `region` centred on world pixel (x, y).
    pub fn world_centered(&self, r: &mut Renderer, region: Region, x: f32, y: f32) {
        self.world(
            r,
            region,
            (x - region.w as f32 / 2.0).round(),
            (y - region.h as f32 / 2.0).round(),
        );
    }

    /// Draws `region` at UI position (game pixels from the top-left), optionally
    /// stretched to `size` game pixels and tinted.
    pub fn ui(
        &self,
        r: &mut Renderer,
        region: Region,
        x: f32,
        y: f32,
        size: Option<(f32, f32)>,
        color: [f32; 4],
    ) {
        self.ui_tex(r, self.atlas, region, x, y, size, color);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui_tex(
        &self,
        r: &mut Renderer,
        texture: TextureId,
        region: Region,
        x: f32,
        y: f32,
        size: Option<(f32, f32)>,
        color: [f32; 4],
    ) {
        if region.w == 0 {
            return;
        }
        let s = self.scale;
        let (w, h) = size.unwrap_or((region.w as f32, region.h as f32));
        r.quad(
            texture,
            region,
            (x * s).round(),
            (y * s).round(),
            (w * s).round(),
            (h * s).round(),
            color,
        );
    }

    pub fn rect(&self, r: &mut Renderer, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.ui(r, self.white, x, y, Some((w, h)), color);
    }

    /// Draws text with Continuum's 8×12 font in one of the [`color`] rows.
    pub fn text(&self, r: &mut Renderer, x: f32, y: f32, color: u32, s: &str) {
        if self.font.w == 0 {
            return;
        }
        for (i, ch) in s.chars().enumerate() {
            let c = ch as u32;
            if !(32..128).contains(&c) {
                continue;
            }
            let glyph = self.font.frame(8, 12, (color % 8) * 96 + (c - 32));
            self.ui(r, glyph, x + i as f32 * 8.0, y, None, [1.0; 4]);
        }
    }

    /// Text right-aligned so it ends at `right`.
    pub fn text_right(&self, r: &mut Renderer, right: f32, y: f32, color: u32, s: &str) {
        self.text(r, right - s.chars().count() as f32 * 8.0, y, color, s);
    }
}

/// Tiles drawn in the main tile pass (animated objects and fly-under tiles are drawn
/// separately; ids above 190 have no tileset image).
fn base_layer_tile(id: u8) -> bool {
    matches!(id, 1..=169 | tile::SAFE | tile::FIRST_FLY_OVER..=tile::LAST_FLY_OVER)
}

pub fn lerp_pos(a: Pos, b: Pos, t: f32) -> (f32, f32) {
    let (ax, ay) = a.to_f32();
    let (bx, by) = b.to_f32();
    // Large jumps (respawn, warp) snap instead of sliding across the map.
    if (bx - ax).abs() > 200.0 || (by - ay).abs() > 200.0 {
        return (bx, by);
    }
    (ax + (bx - ax) * t, ay + (by - ay) * t)
}

pub fn lerp_direction(prev: u32, cur: u32, t: f32) -> u8 {
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

/// Draws everything in world space.
pub fn draw_world(s: &Sandbox, c: &Canvas, r: &mut Renderer, alpha: f32) {
    let a = &s.art;
    let t = s.clock;
    let world = &s.world;
    let me = s.me();

    // LVZ below everything (the starfield is drawn separately, underneath).
    for layer in [layer::BELOW_ALL, layer::AFTER_BACKGROUND] {
        s.lvz.draw(layer, c, r, t, &s.screen_refs);
    }

    // --- Tiles -------------------------------------------------------------------------
    let tx0 = ((c.cam.0 - c.view_w / 2.0) / 16.0).floor() as i32 - 1;
    let ty0 = ((c.cam.1 - c.view_h / 2.0) / 16.0).floor() as i32 - 1;
    let tx1 = ((c.cam.0 + c.view_w / 2.0) / 16.0).ceil() as i32 + 1;
    let ty1 = ((c.cam.1 + c.view_h / 2.0) / 16.0).ceil() as i32 + 1;
    let tile_at = |x: i32, y: i32| -> u8 {
        if (-1..=1024).contains(&x) && (-1..=1024).contains(&y) {
            world.map.get(x, y)
        } else {
            0
        }
    };
    let door_frame = ((t / 0.4).fract() * 4.0) as u8 % 4;
    for ty in ty0.max(-1)..=ty1.min(1024) {
        for tx in tx0.max(-1)..=tx1.min(1024) {
            let id = tile_at(tx, ty);
            if base_layer_tile(id) {
                let shown = if (tile::FIRST_DOOR..=tile::LAST_DOOR).contains(&id) {
                    // Closed doors cycle through their group of four tiles.
                    tile::FIRST_DOOR + (id - tile::FIRST_DOOR) / 4 * 4 + door_frame
                } else {
                    id
                };
                c.world(r, a.tile(shown), (tx * 16) as f32, (ty * 16) as f32);
            }
        }
    }
    let visible = |x: i32, y: i32, margin: i32| {
        x >= tx0 - margin && x <= tx1 && y >= ty0 - margin && y <= ty1
    };

    // --- Bricks ------------------------------------------------------------------------
    let half = a.wall.frame_count(16, 16) / 2;
    for (&(x, y), b) in world.bricks() {
        if visible(x, y, 0) {
            let first = if b.freq == me.freq { half } else { 0 };
            c.world(
                r,
                anim_frame(a.wall, 16, 16, first, half, 1.0, t),
                (x * 16) as f32,
                (y * 16) as f32,
            );
        }
    }

    // --- Animated map objects ----------------------------------------------------------
    for obj in &s.objects {
        let (ox, oy) = (obj.x as i32, obj.y as i32);
        if !visible(ox, oy, 6) {
            continue;
        }
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
            tile::WORMHOLE => anim_frame(a.over5, 80, 80, 0, a.over5.frame_count(80, 80), 2.5, t),
            tile::GOAL => {
                let n = a.goal.frame_count(16, 16) / 2;
                anim_frame(a.goal, 16, 16, n, n, 0.5, t)
            }
            _ => continue,
        };
        c.world(r, region, (ox * 16) as f32, (oy * 16) as f32);
    }

    // --- Flags (yours in the sheet's second half) -------------------------------------
    let hide_flags = s.spectating && s.hide_flags();
    let flag_frames = a.flag.frame_count(16, 16) / 2;
    for f in s
        .flags
        .flags
        .iter()
        .filter(|f| f.visible(s.clock) && !hide_flags)
    {
        if !visible(f.x, f.y, 1) || flag_frames == 0 {
            continue;
        }
        let first = if f.owner == me.freq { flag_frames } else { 0 };
        let region = anim_frame(a.flag, 16, 16, first, flag_frames, 1.0, t);
        c.world(r, region, (f.x * 16) as f32, (f.y * 16) as f32);
    }

    s.lvz.draw(layer::AFTER_TILES, c, r, t, &s.screen_refs);

    // --- Greens ------------------------------------------------------------------------
    let prize_frames = a.prizes.frame_count(16, 16);
    for g in world.greens() {
        if visible(g.x as i32, g.y as i32, 0) {
            c.world(
                r,
                anim_frame(a.prizes, 16, 16, 0, prize_frames, 1.0, t),
                g.x as f32 * 16.0,
                g.y as f32 * 16.0,
            );
        }
    }

    // --- Portal (yours) ----------------------------------------------------------------
    if let Some((p, _)) = me.portal {
        let (x, y) = p.to_f32();
        c.world_centered(
            r,
            anim_frame(a.warppnt, 16, 16, 0, a.warppnt.frame_count(16, 16), 1.0, t),
            x,
            y,
        );
    }

    // --- Projectiles -------------------------------------------------------------------
    for pr in &world.projectiles {
        let (x, y) = lerp_pos(pr.prev_pos, pr.pos, alpha);
        let lvl = pr.level.min(3) as u32;
        let region = match pr.kind {
            ProjectileKind::Bullet => anim_frame(
                a.bullets,
                5,
                5,
                lvl * 4 + if pr.bouncing { 20 } else { 0 },
                4,
                0.15,
                t,
            ),
            ProjectileKind::Burst => {
                let n = a.bullets.frame_count(5, 5);
                let first = if pr.armed { n.saturating_sub(4) } else { 16 };
                anim_frame(a.bullets, 5, 5, first, 4, 0.15, t)
            }
            ProjectileKind::Shrapnel => {
                let first = lvl.min(2) * 10 + if pr.bouncing { 30 } else { 0 };
                anim_frame(a.shrapnel, 8, 8, first, 10, 0.6, t)
            }
            ProjectileKind::Bomb => {
                let first = if pr.emp {
                    40
                } else if pr.bounces_left > 0 {
                    80
                } else {
                    0
                } + lvl * 10;
                anim_frame(a.bombs, 16, 16, first, 10, 1.0, t)
            }
            ProjectileKind::Mine => anim_frame(
                a.mines,
                16,
                16,
                lvl * 10 + if pr.emp { 40 } else { 0 },
                10,
                1.0,
                t,
            ),
            ProjectileKind::Thor => anim_frame(a.bombs, 16, 16, 120, 10, 1.0, t),
            ProjectileKind::Decoy => {
                let Some(owner) = world.ships.iter().find(|s| s.id == pr.owner) else {
                    continue;
                };
                // Decoys mirror their owner's turning.
                let rot = (2 * pr.launch_rotation as i64 - owner.rotation as i64)
                    .rem_euclid(ROTATION_UNITS as i64);
                a.ship(owner.kind, (rot / 1000) as u8)
            }
        };
        c.world_centered(r, region, x, y);
    }

    // --- Balls ---------------------------------------------------------------------------
    let ball_frames = a.powerb.frame_count(16, 16) / 3;
    if ball_frames > 0 {
        let ship_at = |pid: i16| {
            world
                .ships
                .iter()
                .find(|sh| sh.id == pid as u16 && sh.is_alive())
                .map(|sh| {
                    let (x, y) = lerp_pos(sh.prev_pos, sh.pos, alpha);
                    let radius = world.settings.ships[sh.kind as usize % 8].radius_pixels();
                    (x, y, sh.direction(), radius)
                })
        };
        for (_, (x, y), phased) in s.balls.visible(world.tick, &s.ball_rules, ship_at) {
            let first = if phased { ball_frames } else { 0 };
            let region = anim_frame(a.powerb, 16, 16, first, ball_frames, 1.0, t);
            c.world_centered(r, region, x, y);
        }
    }

    s.lvz.draw(layer::AFTER_WEAPONS, c, r, t, &s.screen_refs);

    // --- Exhaust (under ships) -----------------------------------------------------------
    for e in s
        .effects
        .iter()
        .filter(|e| matches!(e.kind, EffectKind::Exhaust | EffectKind::RocketExhaust))
    {
        let (sheet, fw, n, dur) = match e.kind {
            EffectKind::RocketExhaust => (a.rocket, 24, a.rocket.frame_count(24, 24), 0.3),
            _ => (a.exhaust, 16, a.exhaust.frame_count(16, 16) / 2, 0.3),
        };
        let dt = t - e.start;
        if n > 0 && dt < dur {
            let i = ((dt / dur * n as f32) as u32).min(n - 1);
            c.world_centered(r, sheet.frame(fw, fw, i), e.x, e.y);
        }
    }

    // --- Ships ---------------------------------------------------------------------------
    let xradar = me.is_on(toggle::XRADAR);
    let mut labels = Vec::new();
    for ship in &world.ships {
        if !ship.is_alive() {
            continue;
        }
        let is_me = ship.id == me.id;
        let cloaked = ship.is_on(toggle::CLOAK);
        if cloaked && !is_me && !xradar {
            continue;
        }
        let (x, y) = lerp_pos(ship.prev_pos, ship.pos, alpha);
        let dir = lerp_direction(ship.prev_rotation, ship.rotation, alpha);
        let alpha_tint = if cloaked { 0.35 } else { 1.0 };
        let region = a.ship(ship.kind, dir);
        if !is_me && let Some(label) = s.player_label(ship.id) {
            // Continuum puts the name and bounty at the ship's lower right, in team
            // colour (yellow for your team, blue for enemies).
            let (ux, uy) = c.world_to_ui(x + 14.0, y + 10.0);
            let col = if ship.freq == me.freq {
                color::YELLOW
            } else {
                color::BLUE
            };
            labels.push((ux, uy, col, label));
        }
        c.world_tint(
            r,
            region,
            (x - 18.0).round(),
            (y - 18.0).round(),
            [1.0, 1.0, 1.0, alpha_tint],
        );
    }

    for (ux, uy, col, label) in labels {
        c.text(r, ux.round(), uy.round(), col, &label);
    }

    // --- Fly-under tiles (drawn over ships) ------------------------------------------------
    for ty in ty0.max(-1)..=ty1.min(1024) {
        for tx in tx0.max(-1)..=tx1.min(1024) {
            let id = tile_at(tx, ty);
            if (tile::FIRST_FLY_UNDER..=tile::LAST_FLY_UNDER).contains(&id) {
                c.world(r, a.tile(id), (tx * 16) as f32, (ty * 16) as f32);
            }
        }
    }

    // --- Effects ---------------------------------------------------------------------------
    for e in &s.effects {
        let sheet = match e.kind {
            EffectKind::BulletExplosion => a.explode0,
            EffectKind::BombExplosion => a.explode2,
            EffectKind::EmpExplosion => a.empburst,
            EffectKind::ShipExplosion => a.explode1,
            EffectKind::Warp => a.warp,
            EffectKind::Repel => a.repel,
            EffectKind::Exhaust | EffectKind::RocketExhaust => continue,
        };
        let (fw, fh, dur) = e.kind.params();
        if let Some(region) = once_frame(sheet, fw, fh, dur, t - e.start) {
            c.world_centered(r, region, e.x, e.y);
        }
    }

    s.lvz.draw(layer::AFTER_SHIPS, c, r, t, &s.screen_refs);
}
