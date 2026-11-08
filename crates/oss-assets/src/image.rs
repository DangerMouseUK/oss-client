//! Image decoding: Windows BMP (every depth/compression SubSpace content uses) and PNG.
//!
//! Continuum 0.40 ships most `.bm2` files as PNG (8- and 4-bit palettised) and a few as
//! 8-bit BMP (raw and RLE8). Map tilesets embedded in `.lvl` files and user content can
//! be any BMP variant, so the BMP decoder handles 1/4/8/16/24/32 bpp, RLE4, RLE8 and
//! BITFIELDS, bottom-up and top-down. All input is untrusted: malformed data yields an
//! error, never a panic.

use thiserror::Error;

/// Largest width/height we accept (SubSpace art is far smaller; this bounds allocations).
pub const MAX_DIMENSION: u32 = 16384;
/// Largest pixel count we accept (64 MiB of RGBA).
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;

/// A decoded image in straight (non-premultiplied) RGBA8, rows top to bottom.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, RGBA, row-major from the top-left.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageError {
    #[error("unrecognised image format")]
    UnknownFormat,
    #[error("truncated image data")]
    Truncated,
    #[error("unsupported image: {0}")]
    Unsupported(&'static str),
    #[error("invalid image: {0}")]
    Invalid(&'static str),
    #[error("image too large ({0}x{1})")]
    TooLarge(u32, u32),
    #[error("png: {0}")]
    Png(String),
    #[error("jpeg: {0}")]
    Jpeg(String),
    #[error("gif: {0}")]
    Gif(String),
}

impl Image {
    /// An image filled with one colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self, ImageError> {
        check_dimensions(width, height)?;
        let n = width as usize * height as usize;
        let mut data = Vec::with_capacity(n * 4);
        for _ in 0..n {
            data.extend_from_slice(&rgba);
        }
        Ok(Self {
            width,
            height,
            rgba: data,
        })
    }

    /// RGBA of the pixel at (x, y), or `None` when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let p = self.rgba.get(i..i + 4)?;
        Some([p[0], p[1], p[2], p[3]])
    }

    /// SubSpace treats pure black as transparent in sprites and most tiles. This sets
    /// alpha to 0 for every pixel whose RGB is exactly (0, 0, 0).
    pub fn key_black_transparent(&mut self) {
        for p in self.rgba.as_chunks_mut::<4>().0 {
            if p[0] == 0 && p[1] == 0 && p[2] == 0 {
                p[3] = 0;
            }
        }
    }

    /// Copies a `w`x`h` sub-rectangle starting at (x, y). Out-of-range areas are clipped.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Image {
        let x0 = x.min(self.width);
        let y0 = y.min(self.height);
        let w = w.min(self.width - x0);
        let h = h.min(self.height - y0);
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for row in y0..y0 + h {
            let start = (row as usize * self.width as usize + x0 as usize) * 4;
            rgba.extend_from_slice(&self.rgba[start..start + w as usize * 4]);
        }
        Image {
            width: w,
            height: h,
            rgba,
        }
    }
}

fn check_dimensions(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Invalid("zero dimension"));
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION || width as u64 * height as u64 > MAX_PIXELS
    {
        return Err(ImageError::TooLarge(width, height));
    }
    Ok(())
}

/// Decodes a BMP, PNG, JPEG or GIF by sniffing its signature (`.bm2` files may be
/// BMP or PNG; LVZ files use all four).
pub fn decode_image(data: &[u8]) -> Result<Image, ImageError> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(data)
    } else if data.starts_with(b"BM") {
        decode_bmp(data)
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        decode_jpeg(data)
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        decode_gif(data)
    } else {
        Err(ImageError::UnknownFormat)
    }
}

// ---------------------------------------------------------------------------------------
// JPEG

/// Decodes a baseline or progressive JPEG to RGBA8 (opaque).
pub fn decode_jpeg(data: &[u8]) -> Result<Image, ImageError> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(MAX_DIMENSION as usize)
        .set_max_height(MAX_DIMENSION as usize);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(data), options);
    decoder
        .decode_headers()
        .map_err(|e| ImageError::Jpeg(e.to_string()))?;
    let (w, h) = decoder
        .dimensions()
        .ok_or(ImageError::Invalid("jpeg without dimensions"))?;
    let (width, height) = (w as u32, h as u32);
    check_dimensions(width, height)?;
    let rgba = decoder
        .decode()
        .map_err(|e| ImageError::Jpeg(e.to_string()))?;
    if rgba.len() != (width * height * 4) as usize {
        return Err(ImageError::Invalid("jpeg size mismatch"));
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

// ---------------------------------------------------------------------------------------
// GIF

/// Decodes the first frame of a GIF to RGBA8, placed on its logical screen.
pub fn decode_gif(data: &[u8]) -> Result<Image, ImageError> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    if let Some(limit) = std::num::NonZeroU64::new(MAX_PIXELS * 4) {
        options.set_memory_limit(gif::MemoryLimit::Bytes(limit));
    }
    let mut decoder = options
        .read_info(std::io::Cursor::new(data))
        .map_err(|e| ImageError::Gif(e.to_string()))?;
    let (width, height) = (decoder.width() as u32, decoder.height() as u32);
    check_dimensions(width, height)?;
    let frame = decoder
        .read_next_frame()
        .map_err(|e| ImageError::Gif(e.to_string()))?
        .ok_or(ImageError::Invalid("gif without frames"))?;
    let mut img = Image::filled(width, height, [0, 0, 0, 0])?;
    let (fw, fh) = (frame.width as u32, frame.height as u32);
    if frame.buffer.len() < (fw * fh * 4) as usize {
        return Err(ImageError::Truncated);
    }
    for y in 0..fh {
        let ty = frame.top as u32 + y;
        if ty >= height {
            break;
        }
        for x in 0..fw {
            let tx = frame.left as u32 + x;
            if tx >= width {
                break;
            }
            let s = ((y * fw + x) * 4) as usize;
            let d = ((ty * width + tx) * 4) as usize;
            img.rgba[d..d + 4].copy_from_slice(&frame.buffer[s..s + 4]);
        }
    }
    Ok(img)
}

// ---------------------------------------------------------------------------------------
// PNG

/// Decodes a PNG of any colour type/bit depth to RGBA8.
pub fn decode_png(data: &[u8]) -> Result<Image, ImageError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let limits = png::Limits {
        bytes: (MAX_PIXELS * 4) as usize,
    };
    decoder.set_limits(limits);
    let mut reader = decoder
        .read_info()
        .map_err(|e| ImageError::Png(e.to_string()))?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    check_dimensions(width, height)?;
    let size = reader
        .output_buffer_size()
        .ok_or(ImageError::TooLarge(width, height))?;
    let mut buf = vec![0u8; size];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| ImageError::Png(e.to_string()))?;
    let n = width as usize * height as usize;
    let line = frame.line_size;
    let mut rgba = Vec::with_capacity(n * 4);
    for y in 0..height as usize {
        let row = buf
            .get(y * line..y * line + line)
            .ok_or(ImageError::Truncated)?;
        for x in 0..width as usize {
            let px = match frame.color_type {
                png::ColorType::Rgba => {
                    [row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]]
                }
                png::ColorType::Rgb => [row[x * 3], row[x * 3 + 1], row[x * 3 + 2], 255],
                png::ColorType::GrayscaleAlpha => {
                    let g = row[x * 2];
                    [g, g, g, row[x * 2 + 1]]
                }
                png::ColorType::Grayscale => {
                    let g = row[x];
                    [g, g, g, 255]
                }
                png::ColorType::Indexed => {
                    return Err(ImageError::Unsupported("unexpanded indexed png"));
                }
            };
            rgba.extend_from_slice(&px);
        }
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

// ---------------------------------------------------------------------------------------
// BMP

struct Reader<'a> {
    data: &'a [u8],
}

impl Reader<'_> {
    fn u16(&self, at: usize) -> Result<u16, ImageError> {
        let b = self.data.get(at..at + 2).ok_or(ImageError::Truncated)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&self, at: usize) -> Result<u32, ImageError> {
        let b = self.data.get(at..at + 4).ok_or(ImageError::Truncated)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&self, at: usize) -> Result<i32, ImageError> {
        Ok(self.u32(at)? as i32)
    }
}

const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

/// Channel mask helper for 16/32-bit BITFIELDS images.
#[derive(Clone, Copy)]
struct Mask {
    mask: u32,
    shift: u32,
    bits: u32,
}

impl Mask {
    fn new(mask: u32) -> Self {
        if mask == 0 {
            return Self {
                mask: 0,
                shift: 0,
                bits: 0,
            };
        }
        let shift = mask.trailing_zeros();
        let bits = (mask >> shift).trailing_ones();
        Self { mask, shift, bits }
    }
    fn extract(&self, v: u32) -> Option<u8> {
        if self.bits == 0 {
            return None;
        }
        let raw = (v & self.mask) >> self.shift;
        let bits = self.bits.min(16);
        let raw = if self.bits > 16 {
            raw >> (self.bits - 16)
        } else {
            raw
        };
        let max = (1u32 << bits) - 1;
        Some(((raw * 255 + max / 2) / max) as u8)
    }
}

/// Decodes a Windows BMP to RGBA8. Palette images are fully opaque; 32-bit images use
/// their alpha channel only when a non-zero alpha mask is declared.
pub fn decode_bmp(data: &[u8]) -> Result<Image, ImageError> {
    let r = Reader { data };
    if data.get(0..2) != Some(b"BM") {
        return Err(ImageError::UnknownFormat);
    }
    let pixel_offset = r.u32(10)? as usize;
    let header_size = r.u32(14)? as usize;

    let (width, raw_height, bpp, compression, colors_used, palette_entry_size);
    if header_size == 12 {
        // OS/2 BITMAPCOREHEADER
        width = r.u16(18)? as i32;
        raw_height = r.u16(20)? as i16 as i32;
        bpp = r.u16(24)?;
        compression = BI_RGB;
        colors_used = 0;
        palette_entry_size = 3;
    } else if header_size >= 40 {
        width = r.i32(18)?;
        raw_height = r.i32(22)?;
        bpp = r.u16(28)?;
        compression = r.u32(30)?;
        colors_used = r.u32(46)?;
        palette_entry_size = 4;
    } else {
        return Err(ImageError::Unsupported("bmp header size"));
    }

    if width <= 0 || raw_height == 0 || raw_height == i32::MIN {
        return Err(ImageError::Invalid("bmp dimensions"));
    }
    let top_down = raw_height < 0;
    let width = width as u32;
    let height = raw_height.unsigned_abs();
    check_dimensions(width, height)?;

    // Channel masks for 16/32-bit.
    let mut masks = None;
    if compression == BI_BITFIELDS || compression == BI_ALPHABITFIELDS {
        if bpp != 16 && bpp != 32 {
            return Err(ImageError::Unsupported("bitfields depth"));
        }
        let base = 14 + 40; // masks follow a 40-byte header, or are inside a v4/v5 header
        let rm = r.u32(base)?;
        let gm = r.u32(base + 4)?;
        let bm = r.u32(base + 8)?;
        let am = if compression == BI_ALPHABITFIELDS || header_size >= 56 {
            r.u32(base + 12)?
        } else {
            0
        };
        masks = Some([Mask::new(rm), Mask::new(gm), Mask::new(bm), Mask::new(am)]);
    } else if compression == BI_RGB && bpp == 16 {
        masks = Some([
            Mask::new(0x7C00),
            Mask::new(0x03E0),
            Mask::new(0x001F),
            Mask::new(0),
        ]);
    } else if compression == BI_RGB && bpp == 32 {
        // Plain 32-bit BMPs leave the high byte undefined; treat as opaque.
        masks = Some([
            Mask::new(0x00FF_0000),
            Mask::new(0x0000_FF00),
            Mask::new(0x0000_00FF),
            Mask::new(0),
        ]);
    }

    // Palette.
    let mut palette: Vec<[u8; 4]> = Vec::new();
    if bpp <= 8 {
        if !matches!(bpp, 1 | 2 | 4 | 8) {
            return Err(ImageError::Unsupported("bmp bit depth"));
        }
        let max = 1usize << bpp;
        let count = if colors_used == 0 {
            max
        } else {
            (colors_used as usize).min(max)
        };
        let mut at = 14 + header_size;
        if compression == BI_BITFIELDS {
            at += 12;
        }
        for _ in 0..count {
            // Tolerate short palettes (some tools truncate); missing entries stay black.
            let Some(b) = data.get(at..at + 3) else { break };
            palette.push([b[2], b[1], b[0], 255]);
            at += palette_entry_size;
        }
        palette.resize(max, [0, 0, 0, 255]);
    }

    let pixels = data.get(pixel_offset..).ok_or(ImageError::Truncated)?;
    let n = width as usize * height as usize;
    let mut rgba = vec![0u8; n * 4];

    // Writes the pixel at bitmap row `row` (0 = first row stored in the file).
    let put = |rgba: &mut [u8], x: u32, row: u32, px: [u8; 4]| {
        if x >= width || row >= height {
            return;
        }
        let y = if top_down { row } else { height - 1 - row };
        let i = (y as usize * width as usize + x as usize) * 4;
        rgba[i..i + 4].copy_from_slice(&px);
    };

    match compression {
        BI_RLE8 | BI_RLE4 => {
            if (compression == BI_RLE8 && bpp != 8) || (compression == BI_RLE4 && bpp != 4) {
                return Err(ImageError::Invalid("rle depth mismatch"));
            }
            if top_down {
                return Err(ImageError::Invalid("rle bitmaps must be bottom-up"));
            }
            // Pixels not covered by RLE data (delta skips, early end) stay transparent black.
            for p in rgba.as_chunks_mut::<4>().0 {
                p[3] = 0;
            }
            decode_rle(pixels, compression == BI_RLE4, &palette, |x, row, px| {
                put(&mut rgba, x, row, px)
            })?;
        }
        BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS => {
            let stride = (width as usize * bpp as usize).div_ceil(32) * 4;
            if (stride as u64) * height as u64 > pixels.len() as u64 {
                return Err(ImageError::Truncated);
            }
            for row in 0..height {
                let line = &pixels[row as usize * stride..row as usize * stride + stride];
                for x in 0..width {
                    let xi = x as usize;
                    let px = match bpp {
                        1 | 2 | 4 | 8 => {
                            let bit = xi * bpp as usize;
                            let byte = line[bit / 8];
                            let shift = 8 - bpp as usize - (bit % 8);
                            let idx = (byte >> shift) & ((1u16 << bpp) - 1) as u8;
                            palette[idx as usize]
                        }
                        16 => {
                            let v = u16::from_le_bytes([line[xi * 2], line[xi * 2 + 1]]) as u32;
                            mask_pixel(masks.as_ref(), v)
                        }
                        24 => [line[xi * 3 + 2], line[xi * 3 + 1], line[xi * 3], 255],
                        32 => {
                            let b = &line[xi * 4..xi * 4 + 4];
                            mask_pixel(masks.as_ref(), u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        }
                        _ => return Err(ImageError::Unsupported("bmp bit depth")),
                    };
                    put(&mut rgba, x, row, px);
                }
            }
        }
        _ => return Err(ImageError::Unsupported("bmp compression")),
    }

    Ok(Image {
        width,
        height,
        rgba,
    })
}

fn mask_pixel(masks: Option<&[Mask; 4]>, v: u32) -> [u8; 4] {
    let Some(m) = masks else {
        return [0, 0, 0, 255];
    };
    [
        m[0].extract(v).unwrap_or(0),
        m[1].extract(v).unwrap_or(0),
        m[2].extract(v).unwrap_or(0),
        m[3].extract(v).unwrap_or(255),
    ]
}

/// RLE8/RLE4 decoder. Calls `put(x, row, rgba)` for each decoded pixel, where `row` counts
/// from the first stored (bottom) row. Stops cleanly at end-of-bitmap or end of data.
fn decode_rle(
    data: &[u8],
    four_bit: bool,
    palette: &[[u8; 4]],
    mut put: impl FnMut(u32, u32, [u8; 4]),
) -> Result<(), ImageError> {
    let color = |i: u8| palette.get(i as usize).copied().unwrap_or([0, 0, 0, 255]);
    let mut i = 0usize;
    let mut x: u32 = 0;
    let mut row: u32 = 0;
    while i + 1 < data.len() {
        let count = data[i];
        let value = data[i + 1];
        i += 2;
        if count > 0 {
            for k in 0..count {
                let idx = if four_bit {
                    if k % 2 == 0 { value >> 4 } else { value & 0x0F }
                } else {
                    value
                };
                put(x, row, color(idx));
                x = x.saturating_add(1);
            }
            continue;
        }
        match value {
            0 => {
                x = 0;
                row = row.saturating_add(1);
            }
            1 => return Ok(()),
            2 => {
                let d = data.get(i..i + 2).ok_or(ImageError::Truncated)?;
                x = x.saturating_add(d[0] as u32);
                row = row.saturating_add(d[1] as u32);
                i += 2;
            }
            n => {
                let n = n as usize;
                let bytes = if four_bit { n.div_ceil(2) } else { n };
                let run = data.get(i..i + bytes).ok_or(ImageError::Truncated)?;
                for k in 0..n {
                    let idx = if four_bit {
                        let b = run[k / 2];
                        if k % 2 == 0 { b >> 4 } else { b & 0x0F }
                    } else {
                        run[k]
                    };
                    put(x, row, color(idx));
                    x = x.saturating_add(1);
                }
                // Absolute runs are padded to a 16-bit boundary.
                i += bytes + (bytes & 1);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a BMP file with a 40-byte header.
    fn bmp(
        width: i32,
        height: i32,
        bpp: u16,
        compression: u32,
        palette: &[[u8; 3]],
        pixels: &[u8],
    ) -> Vec<u8> {
        let pal_size = palette.len() * 4;
        let offset = 14 + 40 + pal_size;
        let mut v = Vec::new();
        v.extend_from_slice(b"BM");
        v.extend_from_slice(&((offset + pixels.len()) as u32).to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&(offset as u32).to_le_bytes());
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&width.to_le_bytes());
        v.extend_from_slice(&height.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&bpp.to_le_bytes());
        v.extend_from_slice(&compression.to_le_bytes());
        v.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
        v.extend_from_slice(&[0; 8]);
        v.extend_from_slice(&(palette.len() as u32).to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        for c in palette {
            v.extend_from_slice(&[c[2], c[1], c[0], 0]);
        }
        v.extend_from_slice(pixels);
        v
    }

    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    #[test]
    fn bmp_8bit_bottom_up() {
        // 2x2: bottom row (stored first) = red, green; top row = blue, red. Rows pad to 4.
        let px = [0, 1, 0, 0, 2, 0, 0, 0];
        let img = decode_image(&bmp(2, 2, 8, 0, &[RED, GREEN, BLUE], &px)).unwrap();
        assert_eq!(img.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(img.pixel(1, 0), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(0, 1), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(1, 1), Some([0, 255, 0, 255]));
    }

    #[test]
    fn bmp_8bit_top_down() {
        let px = [0, 1, 0, 0, 2, 0, 0, 0];
        let img = decode_image(&bmp(2, -2, 8, 0, &[RED, GREEN, BLUE], &px)).unwrap();
        assert_eq!(img.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(0, 1), Some([0, 0, 255, 255]));
    }

    #[test]
    fn bmp_1bit_and_4bit() {
        let img = decode_image(&bmp(3, 1, 1, 0, &[RED, BLUE], &[0b1010_0000, 0, 0, 0])).unwrap();
        assert_eq!(img.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(img.pixel(1, 0), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(2, 0), Some([0, 0, 255, 255]));

        let img = decode_image(&bmp(3, 1, 4, 0, &[RED, GREEN, BLUE], &[0x21, 0x00, 0, 0])).unwrap();
        assert_eq!(img.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(img.pixel(1, 0), Some([0, 255, 0, 255]));
        assert_eq!(img.pixel(2, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn bmp_24_and_16_and_32bit() {
        // 24-bit: one pixel BGR + 1 pad byte.
        let img = decode_image(&bmp(1, 1, 24, 0, &[], &[10, 20, 30, 0])).unwrap();
        assert_eq!(img.pixel(0, 0), Some([30, 20, 10, 255]));
        // 16-bit 555: pure red = 0x7C00.
        let img = decode_image(&bmp(1, 1, 16, 0, &[], &[0x00, 0x7C, 0, 0])).unwrap();
        assert_eq!(img.pixel(0, 0), Some([255, 0, 0, 255]));
        // 32-bit BI_RGB: high byte ignored.
        let img = decode_image(&bmp(1, 1, 32, 0, &[], &[1, 2, 3, 0])).unwrap();
        assert_eq!(img.pixel(0, 0), Some([3, 2, 1, 255]));
    }

    #[test]
    fn bmp_bitfields_565() {
        let mut file = bmp(1, 1, 16, BI_BITFIELDS, &[], &[]);
        // Insert masks after the 40-byte header, then the pixel; fix up offset.
        let masks: Vec<u8> = [0xF800u32, 0x07E0, 0x001F]
            .iter()
            .flat_map(|m| m.to_le_bytes())
            .collect();
        file.extend_from_slice(&masks);
        let offset = file.len() as u32;
        file[10..14].copy_from_slice(&offset.to_le_bytes());
        file.extend_from_slice(&[0xE0, 0x07, 0, 0]); // pure green
        let img = decode_image(&file).unwrap();
        assert_eq!(img.pixel(0, 0), Some([0, 255, 0, 255]));
    }

    #[test]
    fn bmp_rle8() {
        // 4x2: row0 = 3x red + absolute [green]; row1 = delta skip then blue.
        let px = [
            3, 0, // 3x red
            1, 1, // 1x green
            0, 0, // end of line
            0, 2, 2, 0, // delta +2 x
            2, 2, // 2x blue
            0, 1, // end of bitmap
        ];
        let img = decode_image(&bmp(4, 2, 8, BI_RLE8, &[RED, GREEN, BLUE], &px)).unwrap();
        // Bottom row (y = 1) is row0.
        assert_eq!(img.pixel(0, 1), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(3, 1), Some([0, 255, 0, 255]));
        // Top row: skipped pixels transparent, then blue.
        assert_eq!(img.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(img.pixel(2, 0), Some([0, 0, 255, 255]));
        assert_eq!(img.pixel(3, 0), Some([0, 0, 255, 255]));
    }

    #[test]
    fn bmp_rle8_absolute_and_rle4() {
        let px = [0, 3, 0, 1, 2, 0 /* pad */, 0, 1];
        let img = decode_image(&bmp(3, 1, 8, BI_RLE8, &[RED, GREEN, BLUE], &px)).unwrap();
        assert_eq!(img.pixel(1, 0), Some([0, 255, 0, 255]));
        assert_eq!(img.pixel(2, 0), Some([0, 0, 255, 255]));

        // RLE4: run of 3 alternating 1,2 then an absolute run of 3 nibbles (0, 1, 2).
        let px = [3, 0x12, 0, 3, 0x01, 0x20, 0, 1];
        let img = decode_image(&bmp(6, 1, 4, BI_RLE4, &[RED, GREEN, BLUE], &px)).unwrap();
        assert_eq!(img.pixel(0, 0), Some([0, 255, 0, 255]));
        assert_eq!(img.pixel(1, 0), Some([0, 0, 255, 255]));
        assert_eq!(img.pixel(2, 0), Some([0, 255, 0, 255]));
        assert_eq!(img.pixel(3, 0), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(4, 0), Some([0, 255, 0, 255]));
        assert_eq!(img.pixel(5, 0), Some([0, 0, 255, 255]));
    }

    #[test]
    fn png_palette_roundtrip() {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 1);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(vec![255, 0, 0, 0, 0, 255]);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[1, 0]).unwrap();
        }
        let mut img = decode_image(&out).unwrap();
        assert_eq!(img.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(img.pixel(1, 0), Some([255, 0, 0, 255]));
        img.key_black_transparent();
        assert_eq!(img.pixel(1, 0), Some([255, 0, 0, 255]));
    }

    /// The smallest baseline JPEG: one 8×8 grey block (value 128, so every DCT
    /// coefficient is 0), a quantisation table of ones and one-symbol Huffman tables
    /// (DC category 0, AC end-of-block), so the scan is two zero bits plus padding.
    fn grey_jpeg() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8]; // SOI
        v.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]); // DQT, table 0
        v.extend_from_slice(&[1; 64]);
        // SOF0: 8-bit, 8×8, one component (id 1, 1×1 sampling, table 0).
        v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        for class in [0x00, 0x10] {
            // DHT: one code of length 1 for symbol 0.
            v.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x14, class, 1]);
            v.extend_from_slice(&[0; 15]);
            v.push(0);
        }
        // SOS: component 1 with tables 0/0, full spectral range.
        v.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 1, 1, 0x00, 0, 63, 0]);
        v.push(0b0011_1111); // DC "0", EOB "0", padding ones
        v.extend_from_slice(&[0xFF, 0xD9]); // EOI
        v
    }

    #[test]
    fn decodes_jpeg_and_rejects_garbage() {
        let img = decode_image(&grey_jpeg()).unwrap();
        assert_eq!((img.width, img.height), (8, 8));
        let [r, g, b, a] = img.pixel(3, 5).unwrap();
        assert!((126..=130).contains(&r) && r == g && g == b, "{r} {g} {b}");
        assert_eq!(a, 255);
        let mut cut = grey_jpeg();
        cut.truncate(40);
        assert!(decode_image(&cut).is_err());
    }

    #[test]
    fn decodes_gif_first_frame_on_its_screen() {
        let mut data = Vec::new();
        {
            let palette = [0, 0, 0, 255, 0, 0, 0, 0, 255];
            let mut enc = gif::Encoder::new(&mut data, 4, 3, &palette).unwrap();
            // A 2×2 frame at (1, 1): red, blue / blue, red.
            let frame = gif::Frame {
                left: 1,
                top: 1,
                width: 2,
                height: 2,
                buffer: std::borrow::Cow::Owned(vec![1, 2, 2, 1]),
                ..Default::default()
            };
            enc.write_frame(&frame).unwrap();
        }
        let img = decode_image(&data).unwrap();
        assert_eq!((img.width, img.height), (4, 3));
        assert_eq!(img.pixel(1, 1), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(2, 1), Some([0, 0, 255, 255]));
        assert_eq!(img.pixel(0, 0).map(|p| p[3]), Some(0), "outside the frame");
        assert!(decode_image(&data[..20]).is_err());
    }

    #[test]
    fn black_key_and_crop() {
        let mut img = Image::filled(4, 4, [0, 0, 0, 255]).unwrap();
        img.key_black_transparent();
        assert_eq!(img.pixel(3, 3), Some([0, 0, 0, 0]));
        let c = img.crop(2, 2, 10, 10);
        assert_eq!((c.width, c.height), (2, 2));
    }

    #[test]
    fn malformed_inputs_do_not_panic() {
        let good = bmp(4, 2, 8, BI_RLE8, &[RED], &[4, 0, 0, 0, 4, 0, 0, 1]);
        for len in 0..good.len() {
            let _ = decode_image(&good[..len]);
        }
        let mut huge = bmp(1, 1, 8, 0, &[RED], &[0, 0, 0, 0]);
        huge[18..22].copy_from_slice(&100_000i32.to_le_bytes());
        assert!(matches!(decode_image(&huge), Err(ImageError::TooLarge(..))));
        // Every byte flipped to 0xFF in turn.
        for i in 0..good.len() {
            let mut b = good.clone();
            b[i] = 0xFF;
            let _ = decode_image(&b);
        }
        assert_eq!(decode_image(b"nope"), Err(ImageError::UnknownFormat));
    }
}
