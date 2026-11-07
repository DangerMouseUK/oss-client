//! Map download helpers: decompression and an on-disk cache keyed by name and CRC.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest decompressed map we accept (real maps are well under 1 MiB).
const MAX_MAP: u64 = 16 * 1024 * 1024;

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

    fn path(&self, name: &str) -> Option<PathBuf> {
        // Map names come from the network: allow plain file names only.
        let ok = !name.is_empty()
            && name.len() <= 64
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '));
        ok.then(|| self.dir.join(name))
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

    #[test]
    fn cache_rejects_path_tricks() {
        let dir = std::env::temp_dir().join(format!("oss-map-cache-{}", std::process::id()));
        let c = MapCache::new(&dir);
        assert!(c.store("../evil.lvl", b"x").is_err());
        assert!(c.store("a/b.lvl", b"x").is_err());
        c.store("pub.lvl", b"data").unwrap();
        assert_eq!(c.load("pub.lvl").unwrap(), b"data");
        assert!(c.load("missing.lvl").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
