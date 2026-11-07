//! CPU-side texture atlas builder: shelf-packs RGBA images into one texture so the
//! whole scene draws with a single bind group.

use oss_assets::Image;

/// A rectangle inside the atlas, in texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    /// Sub-rectangle relative to this region (clipped to it).
    pub fn sub(&self, x: u32, y: u32, w: u32, h: u32) -> Region {
        let x = x.min(self.w);
        let y = y.min(self.h);
        Region {
            x: self.x + x,
            y: self.y + y,
            w: w.min(self.w - x),
            h: h.min(self.h - y),
        }
    }

    /// Frame `index` of a sheet split into `fw`×`fh` cells, row-major.
    pub fn frame(&self, fw: u32, fh: u32, index: u32) -> Region {
        let cols = (self.w / fw.max(1)).max(1);
        self.sub((index % cols) * fw, (index / cols) * fh, fw, fh)
    }

    /// Number of `fw`×`fh` frames in this region.
    pub fn frame_count(&self, fw: u32, fh: u32) -> u32 {
        (self.w / fw.max(1)) * (self.h / fh.max(1))
    }
}

/// Packs images into rows ("shelves"). One texel of padding separates entries so
/// nearest sampling never bleeds.
pub struct AtlasBuilder {
    size: u32,
    pixels: Vec<u8>,
    cursor_x: u32,
    cursor_y: u32,
    shelf_h: u32,
}

impl AtlasBuilder {
    pub fn new(size: u32) -> Self {
        Self {
            size,
            pixels: vec![0; size as usize * size as usize * 4],
            cursor_x: 0,
            cursor_y: 0,
            shelf_h: 0,
        }
    }

    /// Adds an image, returning its region, or `None` when the atlas is full.
    pub fn add(&mut self, img: &Image) -> Option<Region> {
        let (w, h) = (img.width, img.height);
        if w + 1 > self.size || h + 1 > self.size {
            return None;
        }
        if self.cursor_x + w + 1 > self.size {
            self.cursor_x = 0;
            self.cursor_y += self.shelf_h;
            self.shelf_h = 0;
        }
        if self.cursor_y + h + 1 > self.size {
            return None;
        }
        let (x0, y0) = (self.cursor_x, self.cursor_y);
        for row in 0..h {
            let src = (row * w * 4) as usize;
            let dst = (((y0 + row) * self.size + x0) * 4) as usize;
            self.pixels[dst..dst + (w * 4) as usize]
                .copy_from_slice(&img.rgba[src..src + (w * 4) as usize]);
        }
        self.cursor_x += w + 1;
        self.shelf_h = self.shelf_h.max(h + 1);
        Some(Region { x: x0, y: y0, w, h })
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_without_overlap() {
        let mut a = AtlasBuilder::new(64);
        let img = Image::filled(20, 10, [1, 2, 3, 4]).unwrap();
        let r: Vec<Region> = (0..6).map(|_| a.add(&img).unwrap()).collect();
        for (i, p) in r.iter().enumerate() {
            for q in &r[i + 1..] {
                let overlap =
                    p.x < q.x + q.w && q.x < p.x + p.w && p.y < q.y + q.h && q.y < p.y + p.h;
                assert!(!overlap);
            }
        }
        assert!(a.add(&Image::filled(64, 64, [0; 4]).unwrap()).is_none());
    }

    #[test]
    fn frames() {
        let r = Region {
            x: 10,
            y: 20,
            w: 360,
            h: 1152,
        };
        assert_eq!(r.frame_count(36, 36), 320);
        assert_eq!(
            r.frame(36, 36, 11),
            Region {
                x: 46,
                y: 56,
                w: 36,
                h: 36
            }
        );
    }
}
