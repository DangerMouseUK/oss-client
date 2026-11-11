//! LVZ: zone-supplied images placed on the map or the screen.
//!
//! Each `.lvz` file holds image files and object sections. An object section lists
//! images (file name, frame grid, animation time) and objects (map position or a
//! screen reference point plus offset, image, layer, display time and mode). Objects
//! with mode "show always" (and the zone/arena entry modes) appear when loaded;
//! "kill" and "death" ones on those events; "server controlled" ones only when the
//! server toggles them (S2C `0x35`). A display time hides the object again after that
//! many tenths of a second.

use std::collections::HashMap;

use oss_assets::{Image, decode_image};
use oss_formats::{Clv1Reference, Lvz as LvzFile, ObjectLayout, ObjectSection};
use oss_proto::objects::{layer, mode};
use oss_session::{ObjectChange, object_change};

use super::view::Canvas;
use crate::gfx::{Region, Renderer, TextureId};

/// Largest image side we upload (GPU texture limits are at least this).
const MAX_SIDE: u32 = 4096;
/// Objects beyond this are ignored (Continuum allows a few thousand per arena).
const MAX_OBJECTS: usize = 20_000;

/// Number of layers, `BELOW_ALL` (0) to `TOP_MOST` (7).
pub const LAYERS: u8 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LvzImage {
    pub texture: Option<TextureId>,
    /// Texture size (after any repacking).
    pub width: u32,
    pub height: u32,
    /// Frame size and count.
    pub frame_w: u32,
    pub frame_h: u32,
    pub frames: u32,
    /// Frames per row in the texture.
    pub cols: u32,
    /// One pass through all frames, in 1/100 s (0 = still).
    pub anim: u32,
}

impl LvzImage {
    /// The frame shown `t` seconds after the object appeared.
    fn frame(&self, t: f32) -> Region {
        let frames = self.frames.max(1);
        let i = if self.anim == 0 || frames == 1 {
            0
        } else {
            let cycle = self.anim as f32 / 100.0;
            ((t.max(0.0) % cycle) / cycle * frames as f32) as u32 % frames
        };
        let cols = self.cols.max(1);
        Region {
            x: (i % cols) * self.frame_w,
            y: (i / cols) * self.frame_h,
            w: self.frame_w,
            h: self.frame_h,
        }
    }
}

/// Fits an `x_count`×`y_count` frame sheet into textures of at most [`MAX_SIDE`]
/// by re-flowing the frames into rows (long strips are common in LVZ files).
/// Returns the image and its frame layout, or `None` if a single frame is too big.
fn fit_sheet(img: Image, x_count: u32, y_count: u32, anim: u32) -> Option<(Image, LvzImage)> {
    let (xc, yc) = (x_count.clamp(1, img.width), y_count.clamp(1, img.height));
    let (fw, fh) = (img.width / xc, img.height / yc);
    let frames = xc * yc;
    let layout = |width, height, cols| LvzImage {
        texture: None,
        width,
        height,
        frame_w: fw,
        frame_h: fh,
        frames,
        cols,
        anim,
    };
    if img.width <= MAX_SIDE && img.height <= MAX_SIDE {
        let l = layout(img.width, img.height, xc);
        return Some((img, l));
    }
    if fw > MAX_SIDE || fh > MAX_SIDE {
        return None;
    }
    let cols = (MAX_SIDE / fw).min(frames);
    let rows = frames.div_ceil(cols);
    if rows * fh > MAX_SIDE {
        return None;
    }
    let (w, h) = (cols * fw, rows * fh);
    let mut out = Image::filled(w, h, [0, 0, 0, 0]).ok()?;
    for i in 0..frames {
        let (sx, sy) = ((i % xc) * fw, (i / xc) * fh);
        let (dx, dy) = ((i % cols) * fw, (i / cols) * fh);
        for row in 0..fh {
            let s = (((sy + row) * img.width + sx) * 4) as usize;
            let d = (((dy + row) * w + dx) * 4) as usize;
            let n = (fw * 4) as usize;
            out.rgba[d..d + n].copy_from_slice(&img.rgba[s..s + n]);
        }
    }
    Some((out, layout(w, h, cols)))
}

/// What a screen object's offset is measured from, on one axis.
///
/// The two LVZ layouts anchor to **different and unrelated sets of points**, and the
/// whole reason `CLV1` screen objects were refused until now is that reading one with
/// the other's table puts it hundreds of pixels out. Keeping them in one `u8` is what
/// made that mistake possible, so they are separate variants here and the compiler
/// keeps them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenAnchor {
    /// `CLV2`: an index into the twelve HUD anchors ([`ScreenRefs`]), which depend on
    /// where this client puts its own radar, stat box, icons and chat.
    Hud(u8),
    /// `CLV1`: one of three points that depend on the view size alone, so they are
    /// resolved at draw time and owe nothing to the HUD.
    Clv1(Clv1Reference),
}

impl ScreenAnchor {
    /// Where this anchor sits along an axis `extent` pixels long, given the twelve
    /// HUD anchors for this frame.
    fn axis(self, refs: &ScreenRefs, extent: f32, hud: impl Fn(&(f32, f32)) -> f32) -> f32 {
        match self {
            Self::Hud(i) => refs.get(i as usize).map_or(0.0, hud),
            Self::Clv1(a) => a.axis(extent as i32) as f32,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LvzObject {
    pub id: u16,
    pub map: bool,
    /// Map: pixels. Screen: offset from the reference points.
    pub x: i16,
    pub y: i16,
    pub x_ref: ScreenAnchor,
    pub y_ref: ScreenAnchor,
    /// Index into [`Lvz::images`].
    pub image: usize,
    /// First image of the object's own section (image numbers are relative to it).
    pub image_base: usize,
    pub layer: u8,
    /// Tenths of a second (0 = until hidden).
    pub time: u16,
    pub mode: u8,
    /// When it appeared (game clock, seconds), while shown.
    pub shown_at: Option<f32>,
}

/// Where screen objects are anchored, as (x, y) game pixels per reference point
/// (see `oss_proto::objects::reference`).
pub type ScreenRefs = [(f32, f32); 12];

#[derive(Default)]
pub struct Lvz {
    pub images: Vec<LvzImage>,
    pub objects: Vec<LvzObject>,
    /// Decoded images waiting for upload: (image index, pixels).
    pending: Vec<(usize, Image)>,
    /// Textures of images dropped by [`Lvz::clear`], handed back to the renderer on
    /// the next [`Lvz::upload`] (the only place this has a renderer to hand).
    freed: Vec<TextureId>,
    /// Files that replace standard sheets (`ships`, `ship3`, `bullets`...), keyed by
    /// lowercase name without extension.
    pub art_overrides: HashMap<String, Image>,
    /// Set when `art_overrides` changed and the atlas needs rebuilding.
    pub art_changed: bool,
}

impl Lvz {
    /// Adds the objects and images of one `.lvz` file. Returns how many objects it
    /// added; problems with single images are logged and skipped.
    pub fn add_file(&mut self, name: &str, bytes: &[u8], clock: f32) -> Result<usize, String> {
        let file = LvzFile::parse(bytes).map_err(|e| format!("{name}: {e}"))?;
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        for s in file.sections.iter().filter(|s| !s.is_object_section()) {
            match s.data() {
                Ok(d) => {
                    let file_name = String::from_utf8_lossy(&s.name).to_ascii_lowercase();
                    let stem = file_name.split('.').next().unwrap_or_default().to_string();
                    if super::art::is_overridable(&stem) {
                        match decode_image(&d) {
                            Ok(mut img) => {
                                img.key_black_transparent();
                                log::info!("{name}: {file_name} replaces the standard {stem}");
                                self.art_overrides.insert(stem, img);
                                self.art_changed = true;
                            }
                            Err(e) => log::warn!("{name}: {file_name}: {e}"),
                        }
                    }
                    files.insert(file_name, d);
                }
                Err(e) => log::warn!("{name}: {}: {e}", String::from_utf8_lossy(&s.name)),
            }
        }
        // Not `file.object_sections()`, which goes through `ObjectSection::parse` and
        // refuses `CLV1` screen objects on purpose: that guard is for callers that
        // read positions with the `CLV2` accessors, which this one no longer does.
        // (oss-core intends to make `parse_with_clv1` the default and retire the
        // split, at which point this can go back to `object_sections()`.)
        let sections = file
            .sections
            .iter()
            .filter(|s| s.is_object_section())
            .map(|s| ObjectSection::parse_with_clv1(&s.data()?))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{name}: {e}"))?;
        let before = self.objects.len();
        for section in sections {
            let layout = section.layout();
            let base = self.images.len();
            for def in &section.images {
                let file_name = String::from_utf8_lossy(&def.name).to_ascii_lowercase();
                let anim = def.animation_time.max(0) as u32;
                let (xc, yc) = (def.x_count.max(1) as u32, def.y_count.max(1) as u32);
                let fitted = match files.get(&file_name).map(|d| decode_image(d)) {
                    Some(Ok(mut img)) => {
                        // Pure black is transparent in every LVZ image, as in Continuum.
                        img.key_black_transparent();
                        let (w, h) = (img.width, img.height);
                        let fitted = fit_sheet(img, xc, yc, anim);
                        if fitted.is_none() {
                            log::warn!("{name}: {file_name} ({w}x{h}) has frames too large");
                        }
                        fitted
                    }
                    Some(Err(e)) => {
                        log::warn!("{name}: {file_name}: {e}");
                        None
                    }
                    None => {
                        log::warn!("{name}: image {file_name} isn't in the file");
                        None
                    }
                };
                match fitted {
                    Some((img, layout)) => {
                        self.pending.push((self.images.len(), img));
                        self.images.push(layout);
                    }
                    // Keep the slot so later image numbers still line up.
                    None => self.images.push(LvzImage {
                        texture: None,
                        width: 0,
                        height: 0,
                        frame_w: 0,
                        frame_h: 0,
                        frames: 0,
                        cols: 1,
                        anim,
                    }),
                }
            }
            for o in &section.objects {
                if self.objects.len() >= MAX_OBJECTS {
                    log::warn!("{name}: too many LVZ objects; ignoring the rest");
                    break;
                }
                // The two layouts put the reference and the offset in opposite ends
                // of the word, so which accessors apply is decided here, once, by the
                // section's own version.
                let (x, y, x_ref, y_ref) = match (o.is_map_object(), layout) {
                    (true, _) => (
                        o.map_x(),
                        o.map_y(),
                        ScreenAnchor::Hud(0),
                        ScreenAnchor::Hud(0),
                    ),
                    (false, ObjectLayout::Clv2) => (
                        o.screen_x(),
                        o.screen_y(),
                        ScreenAnchor::Hud(o.x_ref()),
                        ScreenAnchor::Hud(o.y_ref()),
                    ),
                    (false, ObjectLayout::Clv1) => (
                        o.clv1_screen_x(),
                        o.clv1_screen_y(),
                        ScreenAnchor::Clv1(o.clv1_x_ref()),
                        ScreenAnchor::Clv1(o.clv1_y_ref()),
                    ),
                };
                let mode = o.mode();
                if !o.is_map_object() {
                    log::debug!(
                        "{name}: {layout:?} screen object {} at {x_ref:?} {x:+}, {y_ref:?} {y:+}, image {} layer {} mode {}",
                        o.id(),
                        o.image,
                        o.layer,
                        mode
                    );
                }
                let shown = matches!(
                    mode,
                    mode::SHOW_ALWAYS | mode::ENTER_ZONE | mode::ENTER_ARENA
                );
                self.objects.push(LvzObject {
                    id: o.id(),
                    map: o.is_map_object(),
                    x,
                    y,
                    x_ref,
                    y_ref,
                    image: base + o.image as usize,
                    image_base: base,
                    layer: o.layer.min(LAYERS - 1),
                    time: o.display_time(),
                    mode,
                    shown_at: shown.then_some(clock),
                });
            }
        }
        Ok(self.objects.len() - before)
    }

    /// Drops every object and image this arena's LVZ files loaded, and with them the
    /// standard sheets they replaced. The GPU textures go back to the renderer on the
    /// next [`Lvz::upload`]; before this existed they stayed on the GPU for the rest
    /// of the session every time the player changed arena.
    pub fn clear(&mut self) {
        self.freed
            .extend(self.images.iter().filter_map(|i| i.texture));
        self.images.clear();
        self.objects.clear();
        self.pending.clear();
        if !self.art_overrides.is_empty() {
            self.art_overrides.clear();
            self.art_changed = true;
        }
    }

    /// Uploads newly decoded images, and frees the ones [`Lvz::clear`] dropped.
    pub fn upload(&mut self, r: &mut Renderer) {
        for id in self.freed.drain(..) {
            r.free_texture(id);
        }
        for (i, img) in self.pending.drain(..) {
            if let Some(slot) = self.images.get_mut(i) {
                slot.texture = Some(r.create_texture(img.width, img.height, &img.rgba));
            }
        }
    }

    /// Shows or hides every object with this id (S2C `0x35`). Showing restarts its
    /// animation and display time.
    pub fn toggle(&mut self, id: u16, on: bool, clock: f32) {
        for o in self.objects.iter_mut().filter(|o| o.id == id) {
            o.shown_at = on.then_some(clock);
        }
    }

    /// Applies a server change to every object with that id (S2C `0x36`).
    pub fn change(&mut self, c: &ObjectChange) {
        let d = &c.data;
        let id = d.id();
        for o in self.objects.iter_mut().filter(|o| o.id == id) {
            if c.mask & object_change::POSITION != 0 {
                if o.map {
                    (o.x, o.y) = (d.map_x(), d.map_y());
                } else {
                    // A server change (S2C `0x36`) carries position words with no
                    // version of their own, so they are read with the layout the
                    // object's own file used. A `CLV1` object read as `CLV2` would
                    // jump hundreds of pixels on the first change, which is the bug
                    // this whole split exists to prevent. **Not observed either way:**
                    // nothing we hold says which layout a server composes these in,
                    // and until today a `CLV1` screen object could not reach this code
                    // at all. Settled by a zone with a `CLV1` LVZ whose server moves
                    // one of its screen objects, watched in Continuum.
                    match o.x_ref {
                        ScreenAnchor::Clv1(_) => {
                            (o.x, o.y) = (d.clv1_screen_x(), d.clv1_screen_y());
                            (o.x_ref, o.y_ref) = (
                                ScreenAnchor::Clv1(d.clv1_x_ref()),
                                ScreenAnchor::Clv1(d.clv1_y_ref()),
                            );
                        }
                        ScreenAnchor::Hud(_) => {
                            (o.x, o.y) = (d.screen_x(), d.screen_y());
                            (o.x_ref, o.y_ref) =
                                (ScreenAnchor::Hud(d.x_ref()), ScreenAnchor::Hud(d.y_ref()));
                        }
                    }
                }
            }
            if c.mask & object_change::IMAGE != 0 {
                o.image = o.image_base + d.image as usize;
            }
            if c.mask & object_change::LAYER != 0 {
                o.layer = d.layer.min(LAYERS - 1);
            }
            if c.mask & object_change::TIME != 0 {
                o.time = d.display_time();
            }
            if c.mask & object_change::MODE != 0 {
                o.mode = d.mode();
            }
        }
    }

    /// Shows the objects tied to an event (`mode::KILL` when you kill someone,
    /// `mode::DEATH` when you die).
    pub fn trigger(&mut self, event_mode: u8, clock: f32) {
        for o in self.objects.iter_mut().filter(|o| o.mode == event_mode) {
            o.shown_at = Some(clock);
        }
    }

    /// Hides objects whose display time has run out.
    pub fn expire(&mut self, clock: f32) {
        for o in &mut self.objects {
            if let Some(t) = o.shown_at
                && o.time > 0
                && clock - t >= o.time as f32 / 10.0
            {
                o.shown_at = None;
            }
        }
    }

    pub fn shown(&self) -> impl Iterator<Item = &LvzObject> {
        self.objects.iter().filter(|o| o.shown_at.is_some())
    }

    /// Draws the visible objects on `layer`: map objects on the world canvas `c`,
    /// screen objects on the interface canvas `ui`.
    pub fn draw(
        &self,
        layer: u8,
        c: &Canvas,
        ui: &Canvas,
        r: &mut Renderer,
        clock: f32,
        refs: &ScreenRefs,
    ) {
        for o in self.shown().filter(|o| o.layer == layer) {
            let Some(img) = self.images.get(o.image) else {
                continue;
            };
            let Some(texture) = img.texture else {
                continue;
            };
            let region = img.frame(clock - o.shown_at.unwrap_or(clock));
            if o.map {
                let (x, y) = (o.x as f32, o.y as f32);
                // Skip what's off screen.
                let (ux, uy) = c.world_to_ui(x, y);
                if ux > c.view_w || uy > c.view_h || ux + (region.w as f32) < 0.0 {
                    continue;
                }
                if uy + (region.h as f32) < 0.0 {
                    continue;
                }
                c.world_tex(r, texture, region, x, y);
            } else {
                let rx = o.x_ref.axis(refs, ui.view_w, |p| p.0);
                let ry = o.y_ref.axis(refs, ui.view_h, |p| p.1);
                ui.ui_tex(
                    r,
                    texture,
                    region,
                    rx + o.x as f32,
                    ry + o.y as f32,
                    None,
                    [1.0; 4],
                );
            }
        }
    }
}

/// Continuum's layer names, for reference in draw order.
#[allow(dead_code)]
pub const LAYER_NAMES: [(u8, &str); 8] = [
    (layer::BELOW_ALL, "below all"),
    (layer::AFTER_BACKGROUND, "after background"),
    (layer::AFTER_TILES, "after tiles"),
    (layer::AFTER_WEAPONS, "after weapons"),
    (layer::AFTER_SHIPS, "after ships"),
    (layer::AFTER_GAUGES, "after gauges"),
    (layer::AFTER_CHAT, "after chat"),
    (layer::TOP_MOST, "top most"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use oss_formats::{ImageDef, ObjectSection, Section};
    use oss_proto::ObjectData;

    /// A 4×2 24-bit BMP: two 2×2 frames, red then blue.
    fn sheet() -> Vec<u8> {
        let (w, h) = (4u32, 2u32);
        let row = (w * 3).div_ceil(4) * 4;
        let mut v = Vec::new();
        v.extend_from_slice(b"BM");
        v.extend_from_slice(&(54 + row * h).to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&54u32.to_le_bytes());
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&(w as i32).to_le_bytes());
        v.extend_from_slice(&(h as i32).to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&24u16.to_le_bytes());
        v.extend_from_slice(&[0; 24]);
        for _ in 0..h {
            for x in 0..w {
                v.extend_from_slice(if x < 2 { &[0, 0, 255] } else { &[255, 0, 0] });
            }
        }
        v
    }

    fn lvz_bytes() -> Vec<u8> {
        let mut map_obj = ObjectData::map(7, 8000, 8100);
        map_obj.layer = layer::AFTER_TILES;
        let mut timed = ObjectData::screen(9, 1, -10, 2, -20);
        timed.set_display_time(15);
        timed.set_mode(mode::SERVER_CONTROLLED);
        let mut on_kill = ObjectData::screen(11, 0, 0, 0, 0);
        on_kill.set_mode(mode::KILL);
        let objects = ObjectSection {
            version: *b"CLV2",
            objects: vec![map_obj, timed, on_kill],
            images: vec![ImageDef {
                x_count: 2,
                y_count: 1,
                animation_time: 100,
                name: b"Sheet.BMP".to_vec(),
            }],
        };
        LvzFile {
            sections: vec![
                Section::new(b"sheet.bmp", 0, &sheet()),
                Section::objects(&objects),
            ],
        }
        .to_bytes()
    }

    #[test]
    fn loads_objects_and_images() {
        let mut lvz = Lvz::default();
        assert_eq!(lvz.add_file("t.lvz", &lvz_bytes(), 5.0).unwrap(), 3);
        assert_eq!(lvz.images.len(), 1);
        let img = &lvz.images[0];
        assert_eq!(
            (img.width, img.height, img.frame_w, img.frames),
            (4, 2, 2, 2)
        );
        assert_eq!(lvz.pending.len(), 1);
        let shown: Vec<u16> = lvz.shown().map(|o| o.id).collect();
        assert_eq!(shown, [7], "only the always-shown object starts visible");
        let o = &lvz.objects[1];
        assert_eq!(
            (o.map, o.x_ref, o.x, o.y_ref, o.y),
            (false, ScreenAnchor::Hud(1), -10, ScreenAnchor::Hud(2), -20)
        );
        // Animation: two frames over one second.
        assert_eq!(img.frame(0.2).x, 0);
        assert_eq!(img.frame(0.7).x, 2);
        assert_eq!(img.frame(1.2).x, 0);
        assert!(lvz.add_file("bad.lvz", b"nope", 0.0).is_err());
    }

    #[test]
    fn sections_named_like_standard_sheets_replace_them() {
        let bytes = LvzFile {
            sections: vec![
                Section::new(b"Ship3.bmp", 0, &sheet()),
                Section::new(b"bullets.bm2", 0, &sheet()),
                Section::new(b"ship9.bmp", 0, &sheet()),
                Section::new(b"logo.bmp", 0, &sheet()),
            ],
        }
        .to_bytes();
        let mut lvz = Lvz::default();
        lvz.add_file("art.lvz", &bytes, 0.0).unwrap();
        let mut names: Vec<&str> = lvz.art_overrides.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, ["bullets", "ship3"]);
        assert!(lvz.art_changed);
        // Black was keyed out as in object images.
        assert_eq!(lvz.art_overrides["ship3"].pixel(0, 0).unwrap()[3], 255);
    }

    /// Trench Wars' `entermsg.lvz`, which is the file that made `CLV1` worth
    /// decoding: one screen object whose x hangs from the centred box at -780 and
    /// whose y hangs from the view's top-left at +100. Those two words were read out
    /// of the real file, not invented — loading it logs exactly this.
    ///
    /// The expected positions are oss-core's, measured from Continuum at two view
    /// sizes. 1280x800 is the one that matters here: under the `CLV2` reading this
    /// landed 613px out and the banner ran off the screen.
    #[test]
    fn the_trench_wars_banner_lands_where_continuum_puts_it() {
        let clv1_word = |nibble: u16, offset: i16| (nibble << 12) | (offset as u16 & 0xFFF);
        let banner = ObjectData {
            x_word: clv1_word(4, -780), // Clv1Reference::CentredBox
            y_word: clv1_word(0, 100),  // Clv1Reference::TopLeft
            ..Default::default()
        };
        let objects = ObjectSection {
            version: *b"CLV1",
            objects: vec![banner],
            images: vec![ImageDef {
                x_count: 1,
                y_count: 1,
                animation_time: 0,
                name: b"sheet.bmp".to_vec(),
            }],
        };
        let bytes = LvzFile {
            sections: vec![
                Section::new(b"sheet.bmp", 0, &sheet()),
                Section::objects(&objects),
            ],
        }
        .to_bytes();

        let mut lvz = Lvz::default();
        // Before this, `ObjectSection::parse` refused the whole file.
        assert_eq!(lvz.add_file("entermsg.lvz", &bytes, 0.0).unwrap(), 1);
        let o = &lvz.objects[0];
        assert_eq!(o.x_ref, ScreenAnchor::Clv1(Clv1Reference::CentredBox));
        assert_eq!(o.y_ref, ScreenAnchor::Clv1(Clv1Reference::TopLeft));
        assert_eq!((o.x, o.y), (-780, 100));

        // The HUD anchors must not be consulted at all for a CLV1 object, so this
        // deliberately passes a table of absurd values: if one leaked in, the
        // positions below would be wrong.
        let poison: ScreenRefs = [(9999.0, 9999.0); 12];
        let at = |w: f32, h: f32| {
            (
                o.x_ref.axis(&poison, w, |p| p.0) + o.x as f32,
                o.y_ref.axis(&poison, h, |p| p.1) + o.y as f32,
            )
        };
        assert_eq!(at(1888.0, 1016.0), (644.0, 100.0));
        assert_eq!(at(1920.0, 1080.0), (660.0, 100.0));
        assert_eq!(at(1280.0, 800.0), (340.0, 100.0), "used to clip off-screen");

        // And the same answer as oss-core's own helper, which is where the
        // measurement lives; ours only splits it into anchor plus offset.
        for (w, h) in [(1888, 1016), (1920, 1080), (1280, 800)] {
            let (x, y) = objects.objects[0].clv1_screen_position(w, h);
            assert_eq!(at(w as f32, h as f32), (x as f32, y as f32));
        }
    }

    #[test]
    fn changing_arena_drops_the_old_one_and_queues_its_textures() {
        let art = LvzFile {
            sections: vec![Section::new(b"ship3.bmp", 0, &sheet())],
        }
        .to_bytes();
        let mut lvz = Lvz::default();
        lvz.add_file("objects.lvz", &lvz_bytes(), 0.0).unwrap();
        lvz.add_file("art.lvz", &art, 0.0).unwrap();
        assert!(!lvz.images.is_empty());
        assert!(!lvz.art_overrides.is_empty());
        // Pretend the images were uploaded, as `upload` would have done on the first
        // frame after the file arrived.
        lvz.pending.clear();
        for (i, img) in lvz.images.iter_mut().enumerate() {
            img.texture = Some(crate::gfx::TextureId::for_test(i as u32));
        }
        let uploaded = lvz.images.len();

        lvz.clear();

        assert!(lvz.images.is_empty(), "the old arena's images are gone");
        assert!(lvz.objects.is_empty(), "the old arena's objects are gone");
        assert!(lvz.art_overrides.is_empty(), "standard sheets are back");
        assert!(lvz.art_changed, "the atlas has to be rebuilt without them");
        assert_eq!(
            lvz.freed.len(),
            uploaded,
            "every uploaded texture is queued for the renderer"
        );
    }

    #[test]
    fn long_strips_are_reflowed_to_fit() {
        // 10 frames of 1000x10 in one 10000-pixel row.
        let mut img = Image::filled(10_000, 10, [0, 0, 0, 255]).unwrap();
        for i in 0..10u32 {
            img.rgba[(i * 1000 * 4) as usize] = i as u8 + 1; // mark each frame's corner
        }
        let (out, l) = fit_sheet(img, 10, 1, 100).unwrap();
        assert_eq!((l.cols, l.width, l.height), (4, 4000, 30));
        assert_eq!((out.width, out.height), (4000, 30));
        for i in 0..10u32 {
            let r = l.clone();
            let region = Region {
                x: (i % r.cols) * r.frame_w,
                y: (i / r.cols) * r.frame_h,
                w: r.frame_w,
                h: r.frame_h,
            };
            assert_eq!(out.pixel(region.x, region.y).unwrap()[0], i as u8 + 1);
        }
        // One frame wider than any texture can't be shown.
        let huge = Image::filled(5000, 4, [0; 4]).unwrap();
        assert!(fit_sheet(huge, 1, 1, 0).is_none());
    }

    #[test]
    fn toggles_triggers_timers_and_changes() {
        let mut lvz = Lvz::default();
        lvz.add_file("t.lvz", &lvz_bytes(), 0.0).unwrap();
        lvz.toggle(9, true, 10.0);
        lvz.toggle(7, false, 10.0);
        let shown: Vec<u16> = lvz.shown().map(|o| o.id).collect();
        assert_eq!(shown, [9]);
        lvz.expire(11.4);
        assert_eq!(lvz.shown().count(), 1);
        lvz.expire(11.5); // 1.5 s display time
        assert_eq!(lvz.shown().count(), 0);
        lvz.trigger(mode::KILL, 20.0);
        assert_eq!(lvz.shown().map(|o| o.id).collect::<Vec<_>>(), [11]);

        let mut moved = ObjectData::map(7, 100, 200);
        moved.layer = layer::TOP_MOST;
        lvz.change(&ObjectChange {
            mask: object_change::POSITION | object_change::LAYER,
            data: moved,
        });
        let o = &lvz.objects[0];
        assert_eq!((o.x, o.y, o.layer), (100, 200, layer::TOP_MOST));
    }
}
