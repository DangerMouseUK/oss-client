//! Map download helpers: decompression and an on-disk cache keyed by name and CRC.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Marks a cache file whose name had to be derived. Not in the set
/// [`MapCache::is_plain`] accepts, so derived and plain names cannot collide.
const DERIVED_PREFIX: &str = "~";

/// Largest decompressed map we accept (real maps are well under 1 MiB).
const MAX_MAP: u64 = 16 * 1024 * 1024;

/// Whether Windows would read this name as a device rather than a file.
///
/// `CON.lvl` opens the console, not a file, and the rule applies to the stem
/// before the first dot -- so a zone could otherwise name its map at one of these
/// and have the cache write into a device. Same class of problem as the rest of
/// this module: a name off the network used as a path. Checked on every platform
/// so a cache written on one is readable on another.
fn is_reserved_on_windows(name: &str) -> bool {
    const DEVICES: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    DEVICES.iter().any(|d| stem.eq_ignore_ascii_case(d))
}

/// Inflates a zlib-compressed map from S2C `0x2A`. Uncompressed data (some servers
/// send `.lvl` files as stored) is returned as-is.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    // zlib streams start with a CMF byte of 0x?8 and a header checksum.
    let zlib = data.len() >= 2
        && data[0] & 0x0F == 8
        && u16::from_be_bytes([data[0], data[1]]).is_multiple_of(31);
    if !zlib {
        return Ok(data.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(MAX_MAP + 1)
        .read_to_end(&mut out)?;
    if out.len() as u64 > MAX_MAP {
        return Err(std::io::Error::other("map too large"));
    }
    Ok(out)
}

/// Downloaded maps, stored as `<dir>/<name>` (Continuum keeps them per zone the same
/// way). A cached file is only used when its CRC matches the server's.
#[derive(Debug, Clone)]
pub struct MapCache {
    dir: PathBuf,
}

impl MapCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether `name` can be used as a file name exactly as the server sent it.
    ///
    /// Deliberately narrow: the name arrives over the network, so this is the set
    /// that cannot escape the cache directory or mean something to the filesystem.
    fn is_plain(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && !name.starts_with('.')
            && !name.starts_with(DERIVED_PREFIX)
            && !is_reserved_on_windows(name)
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
    }

    /// Where `name` is cached.
    ///
    /// A name made only of safe characters is used as it stands, which keeps the
    /// cache readable and keeps already-cached maps valid. Anything else gets a
    /// derived name rather than being refused.
    ///
    /// Refusing was the old behaviour and it was wrong: `(public).lvl` is an
    /// ordinary map name -- ASSS's own arenas are `(public)` and `(default)`, and
    /// zones name maps after them -- and because `load` and `store` share this
    /// function, such a map could be neither written nor read, so it downloaded
    /// again on every single join. Widening the character set would only move the
    /// next zone's map into the same hole, so the name is derived instead: a CRC of
    /// the original (so two names never share a file) and a flattened copy of it
    /// (so the cache stays legible). The prefix is a character `is_plain` rejects,
    /// so a derived name can never collide with a plain one.
    fn path(&self, name: &str) -> Option<PathBuf> {
        if name.is_empty() || name.len() > 64 {
            return None;
        }
        if Self::is_plain(name) {
            return Some(self.dir.join(name));
        }
        let flat: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let crc = oss_proto::checksum::crc32(name.as_bytes());
        Some(self.dir.join(format!("{DERIVED_PREFIX}{crc:08x}-{flat}")))
    }

    /// The cached map `name`, if present (the caller checks the CRC).
    pub fn load(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.path(name)?;
        let meta = std::fs::metadata(&path).ok()?;
        if meta.len() > MAX_MAP {
            return None;
        }
        std::fs::read(path).ok()
    }

    pub fn store(&self, name: &str, data: &[u8]) -> std::io::Result<()> {
        let path = self
            .path(name)
            .ok_or_else(|| std::io::Error::other("unsafe map name"))?;
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(path, data)
    }
}

/// Per-user cache folder for maps from `server` ("host:port"):
/// `%LOCALAPPDATA%\OpenSubSpace\maps\<server>` on Windows,
/// `$XDG_CACHE_HOME/open-subspace/maps/<server>` (or `~/.cache/...`) elsewhere.
pub fn default_cache_dir(server: &str) -> PathBuf {
    let safe: String = server
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("OpenSubSpace"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .map(|d| d.join("open-subspace"))
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("maps")
        .join(safe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn inflates_zlib_and_passes_raw() {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(b"BM hello map").unwrap();
        let z = enc.finish().unwrap();
        assert_eq!(inflate(&z).unwrap(), b"BM hello map");
        assert_eq!(inflate(b"BMraw").unwrap(), b"BMraw");
        assert!(inflate(&[0x78, 0x9C, 1, 2, 3]).is_err());
    }

    /// A hostile name must not escape the cache directory -- but it must still
    /// cache. Refusing it outright is what broke `(public).lvl`.
    #[test]
    fn cache_keeps_every_name_inside_its_directory() {
        let dir = std::env::temp_dir().join(format!("oss-map-tricks-{}", std::process::id()));
        let c = MapCache::new(&dir);
        for name in [
            "../evil.lvl",
            "a/b.lvl",
            "..",
            ".hidden",
            r"C:\windows\system32",
            "(public).lvl",
            "CON.lvl",
            "nul",
        ] {
            let path = c.path(name).unwrap_or_else(|| panic!("{name} got no path"));
            assert_eq!(
                path.parent(),
                Some(dir.as_path()),
                "{name} escaped the cache"
            );
            assert_eq!(path.components().count(), dir.components().count() + 1);
        }
        // Length and emptiness are still refused outright: there is nothing to cache.
        assert!(c.path("").is_none());
        assert!(c.path(&"x".repeat(65)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The bug this guards: `(public).lvl` is an ordinary ASSS map name, and a live
    /// zone serving it had its map refused by the cache, so it downloaded again on
    /// every join.
    #[test]
    fn awkward_names_round_trip() {
        let dir = std::env::temp_dir().join(format!("oss-map-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let c = MapCache::new(&dir);
        for name in ["(public).lvl", "(default).lvl", "zone [beta].lvl", "é.lvl"] {
            c.store(name, name.as_bytes())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(c.load(name).as_deref(), Some(name.as_bytes()), "{name}");
        }
        // A plain name is still stored under exactly that file name, so caches
        // written before this change stay valid and the directory stays readable.
        c.store("pub.lvl", b"data").unwrap();
        assert!(dir.join("pub.lvl").is_file());
        assert_eq!(c.load("pub.lvl").unwrap(), b"data");
        assert!(c.load("missing.lvl").is_none());
        // Two different awkward names must never share a file.
        assert_ne!(c.path("(a).lvl").unwrap(), c.path("(b).lvl").unwrap());
        // A Windows device name is a file in the cache, not the device.
        for device in ["CON.lvl", "nul", "LPT1.lvl", "com3 "] {
            let f = c.path(device).unwrap();
            let base = f.file_name().unwrap().to_str().unwrap();
            assert!(base.starts_with(DERIVED_PREFIX), "{device} -> {base}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
