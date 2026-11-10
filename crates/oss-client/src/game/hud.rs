//! The in-game HUD in Continuum's layout: energy gauge and digits (top), item icons
//! (left), weapon/toggle icons and timed effects (right), radar with the map square
//! above it (bottom right), messages (bottom left).

use oss_sim::{Ship, TileMap, cap, ship::active, tile, toggle};

use super::art::{Art, anim_frame};
use super::lvz::ScreenRefs;
use super::sandbox::Sandbox;
use super::view::{Canvas, color};
use crate::gfx::{Region, Renderer, TextureId};

/// Continuum's default `Radar:MapZoomFactor`.
/// Radar zoom when the arena doesn't say (`Radar:MapZoomFactor`).
const DEFAULT_MAP_ZOOM: u32 = 10;
const RADAR_MARGIN: f32 = 6.0;

/// The whole-map radar image, rebuilt when the view width (and so its zoom) changes.
pub struct RadarCache {
    texture: TextureId,
    full_dim: u32,
    built_for: u32,
    /// Map revision it was built from (doors and bricks change the picture).
    revision: u64,
}

pub fn draw(s: &mut Sandbox, c: &Canvas, r: &mut Renderer) {
    if !s.spectating {
        draw_energy(s, c, r);
        draw_items(s, c, r);
    }
    draw_radar(s, c, r);
    draw_messages(s, c, r);
}

/// Top of the item icon columns.
fn icons_top(c: &Canvas) -> f32 {
    (c.view_h * 0.57 + 1.0 - 100.0).floor()
}

/// Side of the small radar.
pub(super) fn radar_dim(c: &Canvas) -> f32 {
    ((((c.view_w as u32 / 6) / 4) * 8) / 2) as f32
}

/// The map square a pixel position falls in, as Continuum names it: the map is 20
/// by 20 squares, columns A onwards and rows 1 onwards (nullspace
/// `Radar::RenderTime`, which divides the 1024-tile map by `1024 / 20`).
pub(super) fn map_coordinate(x: f32, y: f32) -> String {
    let cell = |v: f32| (v / 16.0 / (1024.0 / 20.0)).floor().clamp(0.0, 19.0) as u32;
    let col = char::from(b'A' + cell(x) as u8);
    format!("{col}{}", cell(y) + 1)
}

/// The line Continuum draws just above the radar. Continuum's is
/// `"{hour}:{min}{am|pm}  {square}"` (nullspace `Radar::RenderTime`); we draw the
/// square alone, because std has no local time and a wall clock in the corner did
/// not look worth a date crate. Adding the clock is a one-line change here once
/// one is available.
pub(super) fn radar_text(x: f32, y: f32) -> String {
    map_coordinate(x, y)
}

/// Anchor points for LVZ screen objects, indexed by `oss_proto::objects::reference`
/// (the LVZ format's one-letter codes in brackets).
///
/// Sources: the LVZ format notes give each code's meaning; nullspace (MIT) implements
/// only Normal, Centre, Bottom-right and Below-energy, and those four follow it
/// exactly. The rest are our reading of the format notes placed on our HUD, which
/// mirrors Continuum's layout. They are **unverified** against Continuum and need a
/// veteran play-test (a zone tutorial LVZ that points arrows at each HUD element is a
/// good check):
///
/// | # | Code | Meaning                         | Status     |
/// |---|------|---------------------------------|------------|
/// | 0 | (none) | screen top-left               | nullspace  |
/// | 1 | C    | screen centre                   | nullspace  |
/// | 2 | B    | screen bottom-right             | nullspace  |
/// | 3 | S    | stat box bottom-right           | unverified |
/// | 4 | G    | specials (left items) top-right | unverified |
/// | 5 | F    | specials bottom-right           | unverified |
/// | 6 | E    | below the energy bar            | nullspace (spectating: unverified) |
/// | 7 | T    | chat top-left                   | unverified |
/// | 8 | R    | radar top-left                  | unverified (radar size from nullspace) |
/// | 9 | O    | radar text top-left             | unverified |
/// | 10 | W   | weapons (right icons) top-left  | unverified |
/// | 11 | V   | weapons bottom-left             | unverified |
pub fn screen_refs(
    c: &Canvas,
    statbox_corner: (f32, f32),
    chat_top: f32,
    spectating: bool,
) -> ScreenRefs {
    let (w, h) = (c.view_w, c.view_h);
    let top = icons_top(c);
    let dim = radar_dim(c);
    let (rx, ry) = (w - dim - RADAR_MARGIN, h - dim - RADAR_MARGIN);
    // Icon (26) plus the count tab beside it.
    let specials_right = 37.0;
    // nullspace: the health bar's height, or below the spectator text lines when
    // spectating (we show no energy bar then; our spectator line is one row).
    let below_energy = if spectating { 12.0 } else { 28.0 };
    [
        (0.0, 0.0),
        ((w / 2.0).floor(), (h / 2.0).floor()),
        (w, h),
        statbox_corner,
        (specials_right, top),
        (specials_right, top + 7.0 * 25.0),
        ((w / 2.0).floor(), below_energy),
        (0.0, chat_top),
        (rx, ry),
        // The radar's text line (coordinates/clock) sits just above it.
        (rx, ry - 12.0),
        (w - 26.0, top),
        (w - 26.0, top + 6.0 * 25.0),
    ]
}

fn draw_energy(s: &Sandbox, c: &Canvas, r: &mut Renderer) {
    let me = s.me();
    let a = &s.art;
    let settings = &s.world.settings.ships[me.kind as usize];
    let white = [1.0; 4];
    let center_x = (c.view_w / 2.0).floor();
    let center_y = 14.0;
    let g = a.gradient;
    let max_possible = settings.maximum_energy.max(1) as f32;
    let ship_max = me.stats.max_energy.max(1) as f32;
    let energy = me.energy.max(0) as f32 / 1000.0;
    let frac = (energy / ship_max).clamp(0.0, 1.0);

    if g.w > 0 {
        // Background track, current-max track, then the pulsing energy bar.
        let top_w = ship_max / max_possible * 240.0;
        c.ui(
            r,
            g.frame(1, 1, 5),
            center_x - 240.0,
            center_y - 4.0,
            Some((480.0, 2.0)),
            white,
        );
        c.ui(
            r,
            g.frame(1, 1, 0),
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
        let bar = anim_frame(g, 1, 1, row * 14, 14, 1.4, s.clock);
        let w = (frac * top_w).round();
        c.ui(r, bar, center_x - w, center_y, Some((w * 2.0, 6.0)), white);
    }
    if a.hlthbar.w > 0 {
        c.ui(
            r,
            a.hlthbar.frame(116, 28, 0),
            center_x - 58.0,
            0.0,
            None,
            white,
        );
    }

    // Energy number, right-aligned at the top right.
    if a.engyfont.w > 0 {
        let mut value = energy as u32;
        let mut i = 1.0;
        loop {
            c.ui(
                r,
                a.engyfont.frame(16, 24, value % 10),
                c.view_w - i * 16.0,
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

/// Icon index in `Icons.bm2` for the gun indicator, or `None` without guns.
fn gun_icon(me: &Ship) -> Option<u32> {
    if me.guns == 0 {
        return None;
    }
    let mut start = 0;
    if me.has(cap::MULTIFIRE) {
        start = if me.multifire { 3 } else { 6 };
    }
    if me.has(cap::BOUNCING_BULLETS) {
        start += 9;
    }
    Some(start + me.guns as u32 - 1)
}

fn bomb_icon(me: &Ship) -> Option<u32> {
    if me.bombs == 0 {
        return None;
    }
    let start = 18
        + match (me.has(cap::PROXIMITY), me.shrapnel > 0) {
            (false, false) => 0,
            (true, false) => 3,
            (false, true) => 6,
            (true, true) => 9,
        };
    Some(start + me.bombs as u32 - 1)
}

fn draw_items(s: &Sandbox, c: &Canvas, r: &mut Renderer) {
    let a: &Art = &s.art;
    let me = s.me();
    let white = [1.0; 4];
    if a.icons.w == 0 || !me.is_alive() {
        return;
    }
    let icon = |i: u32| a.icons.frame(26, 24, i);
    // Thin "empty slot" markers cut from the icon sheet.
    let empty_left = a.icons.sub(152, 120, 4, 24);
    let empty_right = a.icons.sub(78, 120, 4, 24);
    let y_top = icons_top(c);

    // Left column: counted items.
    let mut y = y_top;
    let items = &me.items;
    for (count, index) in [
        (items.bursts, 30),
        (items.repels, 31),
        (items.decoys, 40),
        (items.thors, 41),
        (items.bricks, 42),
        (items.rockets, 43),
        (items.portals, 46),
    ] {
        if count > 0 {
            c.ui(r, icon(index), 0.0, y, None, white);
            if count > 1 && a.icondoor.w > 0 {
                c.ui(
                    r,
                    a.icondoor.frame(11, 13, count.min(10) as u32),
                    23.0,
                    y + 5.0,
                    None,
                    white,
                );
            }
        } else {
            c.ui(r, empty_left, 0.0, y, None, white);
        }
        y += 25.0;
    }

    // Right column: guns, bombs, then the four toggles.
    let x = c.view_w - 26.0;
    let mut y = y_top;
    let mut slot = |index: Option<u32>, y: &mut f32| {
        match index {
            Some(i) => c.ui(r, icon(i), x, *y, None, white),
            None => c.ui(r, empty_right, x + 22.0, *y, None, white),
        }
        *y += 25.0;
    };
    slot(gun_icon(me), &mut y);
    slot(bomb_icon(me), &mut y);
    for (capability, bit, on_icon) in [
        (cap::STEALTH, toggle::STEALTH, 32),
        (cap::CLOAK, toggle::CLOAK, 34),
        (cap::XRADAR, toggle::XRADAR, 36),
        (cap::ANTIWARP, toggle::ANTIWARP, 38),
    ] {
        let index = me
            .has(capability)
            .then(|| if me.is_on(bit) { on_icon } else { on_icon + 1 });
        slot(index, &mut y);
    }

    // Timed effects with time left, under the energy digits.
    let tick = s.world.tick;
    let settings = &s.world.settings.ships[me.kind as usize];
    let secs = |end: u32| end.wrapping_sub(tick) as f32 / 100.0;
    let mut timed = |sheet: Region, y: f32, label: String| {
        let n = sheet.frame_count(16, 16);
        if n > 0 {
            c.ui(
                r,
                anim_frame(sheet, 16, 16, 0, n, 1.0, s.clock),
                c.view_w - 16.0,
                y - 8.0,
                None,
                white,
            );
        }
        c.text_right(r, c.view_w - 18.0, y - 4.0, color::YELLOW, &label);
    };
    if active(tick, me.shield_end_tick) {
        let total = settings.shields_time.max(1) as f32 / 100.0;
        timed(
            a.shield,
            85.0,
            format!("{}%", (secs(me.shield_end_tick) / total * 100.0) as u32),
        );
    }
    if active(tick, me.super_end_tick) {
        timed(
            a.superpower,
            101.0,
            format!("{:.1}", secs(me.super_end_tick)),
        );
    }
    if let Some((_, end)) = me.portal {
        timed(a.warppnt, 133.0, format!("{:.1}", secs(end)));
    }
    if active(tick, me.rocket_end_tick) {
        c.text_right(
            r,
            c.view_w - 18.0,
            145.0,
            color::YELLOW,
            &format!("Rocket {:.1}", secs(me.rocket_end_tick)),
        );
    }
    if active(tick, me.shutdown_end_tick) {
        c.text_right(r, c.view_w - 18.0, 157.0, color::RED, "Engines shut down");
    }
}

fn ensure_radar(s: &mut Sandbox, r: &mut Renderer, view_w: u32) {
    let revision = s.map_revision;
    // Radar:MapZoomFactor: the whole map is (screen width * 8 / zoom) pixels across,
    // as in nullspace.
    let zoom = s.map_zoom().unwrap_or(DEFAULT_MAP_ZOOM).max(1);
    let full_dim = (view_w * 8 / zoom).clamp(64, 4096);
    if s.radar.as_ref().is_some_and(|rd| {
        rd.built_for == view_w && rd.revision == revision && rd.full_dim == full_dim
    }) {
        return;
    }
    let pixels = radar_image(&s.world.map, full_dim);
    match &mut s.radar {
        Some(rd) if rd.full_dim == full_dim => {
            r.update_texture(rd.texture, &pixels);
            rd.built_for = view_w;
            rd.revision = revision;
        }
        Some(rd) => {
            r.recreate_texture(rd.texture, full_dim, full_dim, &pixels);
            *rd = RadarCache {
                texture: rd.texture,
                full_dim,
                built_for: view_w,
                revision,
            };
        }
        None => {
            let texture = r.create_texture(full_dim, full_dim, &pixels);
            s.radar = Some(RadarCache {
                texture,
                full_dim,
                built_for: view_w,
                revision,
            });
        }
    }
}

fn draw_radar(s: &mut Sandbox, c: &Canvas, r: &mut Renderer) {
    ensure_radar(s, r, c.view_w as u32);
    let Some(radar) = s.radar.as_ref() else {
        return;
    };
    let full = radar.full_dim as f32;
    let me = s.me();
    let to_radar = |px: f32| px * full / 16384.0;
    let (dim, min_x, min_y, x0, y0) = if s.big_radar {
        // Whole map, as large as fits, centred.
        let dim = (c.view_w.min(c.view_h) - 40.0).floor().max(64.0);
        (
            dim,
            0.0,
            0.0,
            ((c.view_w - dim) / 2.0).floor(),
            ((c.view_h - dim) / 2.0).floor(),
        )
    } else {
        // About a sixth of the screen width, centred on you, as in Continuum.
        let dim = radar_dim(c);
        // Centred on the camera (your ship, or the spectator view).
        let p = oss_sim::Vec2::new(c.cam.0 as i32, c.cam.1 as i32);
        let min_x = (to_radar(p.x as f32) - dim / 2.0)
            .floor()
            .clamp(0.0, (full - dim).max(0.0));
        let min_y = (to_radar(p.y as f32) - dim / 2.0)
            .floor()
            .clamp(0.0, (full - dim).max(0.0));
        (
            dim,
            min_x,
            min_y,
            c.view_w - dim - RADAR_MARGIN,
            c.view_h - dim - RADAR_MARGIN,
        )
    };
    if dim < 8.0 {
        return;
    }
    // Radar texels per screen pixel (1 for the small radar).
    let k = if s.big_radar { full / dim } else { 1.0 };
    let region = if s.big_radar {
        Region {
            x: 0,
            y: 0,
            w: radar.full_dim,
            h: radar.full_dim,
        }
    } else {
        Region {
            x: min_x as u32,
            y: min_y as u32,
            w: dim as u32,
            h: dim as u32,
        }
    };
    c.ui_tex(r, radar.texture, region, x0, y0, Some((dim, dim)), [1.0; 4]);
    if !s.big_radar {
        // Where Continuum puts it: right-aligned just above the radar.
        let (cx, cy) = c.cam;
        c.text_right(
            r,
            c.view_w - 5.0,
            y0 - 14.0,
            color::WHITE,
            &radar_text(cx, cy),
        );
    }

    let mut dot = |px: f32, py: f32, size: f32, rgba: [f32; 4]| {
        let (rx, ry) = ((to_radar(px) - min_x) / k, (to_radar(py) - min_y) / k);
        if rx >= 0.0 && ry >= 0.0 && rx < dim - 1.0 && ry < dim - 1.0 {
            c.rect(r, x0 + rx.floor(), y0 + ry.floor(), size, size, rgba);
        }
    };
    for g in s.world.greens() {
        dot(
            g.x as f32 * 16.0 + 8.0,
            g.y as f32 * 16.0 + 8.0,
            2.0,
            [0.2, 0.9, 0.2, 1.0],
        );
    }
    // Ships: you blink yellow, team yellow, enemies blue (stealth hides enemies
    // unless your X-radar is on).
    let blink = (s.clock * 2.5).fract() < 0.5;
    let xradar = s.xradar();
    for ship in &s.world.ships {
        if !ship.is_alive() {
            continue;
        }
        let is_me = ship.id == me.id;
        if (is_me && !blink) || (!is_me && ship.is_on(toggle::STEALTH) && !xradar) {
            continue;
        }
        let (x, y) = ship.pos.to_f32();
        let marks = s.palette.marks();
        let rgba = if s.radar_flagger(ship.id) {
            marks.flagger
        } else if ship.freq == me.freq {
            marks.team
        } else {
            marks.enemy
        };
        dot(x, y, 2.0, rgba);
    }
    // Ship:SeeBombLevel / SeeMines: bombs and mines your ship can see on radar.
    if !s.spectating {
        let ship = &s.world.settings.ships[me.kind as usize % 8];
        for pr in &s.world.projectiles {
            let seen = match pr.kind {
                oss_sim::ProjectileKind::Bomb => {
                    ship.see_bomb_level > 0 && pr.level + 1 >= ship.see_bomb_level
                }
                oss_sim::ProjectileKind::Mine => ship.see_mines,
                _ => false,
            };
            if seen {
                let (x, y) = pr.pos.to_f32();
                dot(x, y, 2.0, s.palette.marks().weapon);
            }
        }
    }
    // Soccer:BallLocation: free balls always show.
    if s.world.settings.soccer.ball_location {
        for b in s.balls.balls.iter().flatten() {
            if b.state != super::balls::BallState::Carried {
                let (x, y) = b.pixels();
                dot(x as f32, y as f32, 2.0, s.palette.marks().ball);
            }
        }
    }

    // Border.
    let b = [0.45, 0.45, 0.5, 1.0];
    c.rect(r, x0 - 1.0, y0 - 1.0, dim + 2.0, 1.0, b);
    c.rect(r, x0 - 1.0, y0 + dim, dim + 2.0, 1.0, b);
    c.rect(r, x0 - 1.0, y0, 1.0, dim, b);
    c.rect(r, x0 + dim, y0, 1.0, dim, b);
}

fn draw_messages(s: &Sandbox, c: &Canvas, r: &mut Renderer) {
    let me = s.me();
    // Flags: what you carry (and when they drop), and what your team holds.
    let carried = s.carried_flags();
    let held = s.flags.owned_by(me.freq);
    let mut flag_line = String::new();
    if carried > 0 {
        flag_line = format!("Flags:{carried}");
        if let Some(left) = s.flag_time_left() {
            flag_line += &format!(" {}:{:02}", left as u32 / 60, left as u32 % 60);
        }
    } else if held > 0 && !s.spectating {
        flag_line = format!("Team flags:{held}");
    }
    if !flag_line.is_empty() {
        c.text_right(r, c.view_w - 4.0, 26.0, color::YELLOW, &flag_line);
    }
    if s.recording() {
        c.text_right(r, c.view_w - 4.0, 50.0, color::RED, "REC");
    }
    if s.is_replay() {
        c.text_right(r, c.view_w - 4.0, 50.0, color::GREEN, "REPLAY");
    }
    if let Some(left) = s.crown_time_left() {
        let line = format!("Crown {}:{:02}", left as u32 / 60, left as u32 % 60);
        c.text_right(r, c.view_w - 4.0, 38.0, color::YELLOW, &line);
    }
    if !me.is_alive() && !s.spectating {
        let msg = "You have been destroyed";
        c.text(
            r,
            ((c.view_w - msg.len() as f32 * 8.0) / 2.0).floor(),
            (c.view_h / 2.0 - 40.0).floor(),
            color::RED,
            msg,
        );
    }
    if s.show_fps {
        let line = format!(
            "{:.0} fps  scale {}x  {:?}  tick {}",
            s.fps(),
            c.scale,
            r.present_mode,
            s.world.tick
        );
        c.text(r, 4.0, 30.0, color::WHITE, &line);
    }
}

/// Builds the whole-map radar image, `dim`×`dim` RGBA. Colours match Continuum's radar.
pub fn radar_image(map: &TileMap, dim: u32) -> Vec<u8> {
    const BG: [u8; 4] = [10, 25, 10, 255];
    let color = |id: u8| -> [u8; 4] {
        match id {
            0 | tile::FLAG | 242.. => BG,
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
                if id == 0 || id == tile::FLAG || id > 241 {
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
    fn weapon_icons() {
        let mut s = Ship::new(0, 0, 0, &oss_sim::STOCK_SHIP);
        assert_eq!(gun_icon(&s), None);
        s.guns = 2;
        assert_eq!(gun_icon(&s), Some(1));
        s.caps |= cap::MULTIFIRE | cap::BOUNCING_BULLETS;
        s.multifire = true;
        assert_eq!(gun_icon(&s), Some(13));
        s.bombs = 3;
        s.shrapnel = 4;
        s.caps |= cap::PROXIMITY;
        assert_eq!(bomb_icon(&s), Some(29));
    }
}
