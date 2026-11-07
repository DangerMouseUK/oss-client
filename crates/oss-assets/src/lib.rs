//! Loaders for SubSpace/Continuum art and sound formats. Assets are read from a
//! user's existing Continuum install; this crate never ships any.
//!
//! - [`decode_image`]: `.bm2`/`.bmp` (BMP 1/2/4/8/16/24/32 bpp, RLE4/RLE8, BITFIELDS) and PNG.
//! - [`decode_wave`]: `.wa2`/`.wav` (RIFF PCM 8/16/24-bit).
//! - [`AssetDirs`]: the asset search path (your folder, a Continuum install, the
//!   bundled `assets/classic`) and loading named assets from it.

pub mod dirs;
pub mod image;
pub mod locate;
pub mod sound;

pub use dirs::{ASSETS_ENV_VAR, AssetDirs, AssetSearch};
pub use image::{Image, ImageError, decode_bmp, decode_image, decode_png};
pub use locate::{AssetError, ContinuumDir, ENV_VAR};
pub use sound::{Sound, SoundError, decode_wave};
