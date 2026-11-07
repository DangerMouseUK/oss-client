//! Finding the user's Continuum install and loading named assets from it.
//!
//! Search order: an explicitly configured folder, then `OSS_CONTINUUM_DIR`, then Steam
//! libraries, then standard install folders (including Wine prefixes on Unix). A folder
//! counts as an install when it has `graphics/ships.bm2` (any letter case).

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::image::{Image, ImageError, decode_image};
use crate::sound::{Sound, SoundError, decode_wave};

/// Environment variable naming a Continuum folder; overrides automatic discovery.
pub const ENV_VAR: &str = "OSS_CONTINUUM_DIR";

/// Upper bound for asset files we read into memory.
const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("no Continuum install found (tried: {})", display_paths(.0))]
    NotFound(Vec<PathBuf>),
    #[error("{0} is not a Continuum folder (no graphics/ships.bm2)")]
    NotAnInstall(PathBuf),
    #[error("asset {0} not found")]
    Missing(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: file too large")]
    TooLarge { path: PathBuf },
    #[error("{path}: {source}")]
    Image { path: PathBuf, source: ImageError },
    #[error("{path}: {source}")]
    Sound { path: PathBuf, source: SoundError },
}

fn display_paths(p: &[PathBuf]) -> String {
    p.iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A validated Continuum install folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuumDir {
    root: PathBuf,
}

impl ContinuumDir {
    /// Opens `path` if it looks like a Continuum install.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, AssetError> {
        let root = path.into();
        if is_install(&root) {
            Ok(Self { root })
        } else {
            Err(AssetError::NotAnInstall(root))
        }
    }

    /// Finds an install: `configured` first, then `OSS_CONTINUUM_DIR`, then well-known
    /// locations. An explicitly configured or env folder that is invalid is an error
    /// rather than silently falling through, so users notice typos.
    pub fn locate(configured: Option<&Path>) -> Result<Self, AssetError> {
        if let Some(p) = configured {
            return Self::open(p);
        }
        if let Some(p) = std::env::var_os(ENV_VAR).filter(|v| !v.is_empty()) {
            return Self::open(PathBuf::from(p));
        }
        let candidates = candidate_dirs();
        for c in &candidates {
            if is_install(c) {
                return Ok(Self { root: c.clone() });
            }
        }
        Err(AssetError::NotFound(candidates))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Finds `<root>/<subdir>/<stem>.<ext>` for the first extension that exists, matching
    /// names case-insensitively (Continuum installs copied from Windows have mixed case).
    pub fn find(&self, subdir: &str, stem: &str, exts: &[&str]) -> Option<PathBuf> {
        let dir = find_entry(&self.root, subdir)?;
        for ext in exts {
            if let Some(p) = find_entry(&dir, &format!("{stem}.{ext}")) {
                return Some(p);
            }
        }
        None
    }

    /// Loads `graphics/<name>` (`.bm2`, then `.bmp`, then `.png`) as RGBA.
    pub fn load_graphic(&self, name: &str) -> Result<Image, AssetError> {
        let path = self
            .find("graphics", name, &["bm2", "bmp", "png"])
            .ok_or_else(|| AssetError::Missing(format!("graphics/{name}")))?;
        let data = read_file(&path)?;
        decode_image(&data).map_err(|source| AssetError::Image { path, source })
    }

    /// Loads `sound/<name>` (`.wa2`, then `.wav`).
    pub fn load_sound(&self, name: &str) -> Result<Sound, AssetError> {
        let path = self
            .find("sound", name, &["wa2", "wav"])
            .ok_or_else(|| AssetError::Missing(format!("sound/{name}")))?;
        let data = read_file(&path)?;
        decode_wave(&data).map_err(|source| AssetError::Sound { path, source })
    }
}

/// Reads a whole file with a size cap.
pub fn read_file(path: &Path) -> Result<Vec<u8>, AssetError> {
    let io = |source| AssetError::Io {
        path: path.to_path_buf(),
        source,
    };
    let meta = std::fs::metadata(path).map_err(io)?;
    if meta.len() > MAX_FILE_SIZE {
        return Err(AssetError::TooLarge {
            path: path.to_path_buf(),
        });
    }
    std::fs::read(path).map_err(io)
}

/// Case-insensitive lookup of one directory entry.
fn find_entry(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    let want = name.to_ascii_lowercase();
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let n = e.file_name();
        (n.to_string_lossy().to_ascii_lowercase() == want).then(|| e.path())
    })
}

fn is_install(dir: &Path) -> bool {
    find_entry(dir, "graphics")
        .and_then(|g| find_entry(&g, "ships.bm2"))
        .is_some()
}

/// Every folder automatic discovery checks, in order.
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for lib in steam_libraries() {
        let common = lib.join("steamapps").join("common");
        if let Ok(entries) = std::fs::read_dir(&common) {
            let mut found: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .contains("continuum")
                })
                .map(|e| e.path())
                .collect();
            found.sort();
            out.extend(found);
        }
        out.push(common.join("SubSpace Continuum"));
    }

    #[cfg(windows)]
    {
        for var in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
            if let Some(pf) = std::env::var_os(var) {
                out.push(PathBuf::from(pf).join("Continuum"));
            }
        }
        out.push(PathBuf::from(r"C:\Continuum"));
        out.push(PathBuf::from(r"C:\Games\Continuum"));
    }

    #[cfg(not(windows))]
    if let Some(home) = home_dir() {
        let wine = home.join(".wine").join("drive_c");
        out.push(wine.join("Program Files (x86)").join("Continuum"));
        out.push(wine.join("Program Files").join("Continuum"));
        out.push(home.join("Continuum"));
        out.push(home.join("Games").join("Continuum"));
    }

    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

#[cfg(not(windows))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Steam root folders plus any extra libraries listed in `libraryfolders.vdf`.
fn steam_libraries() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(pf) = std::env::var_os(var) {
                roots.push(PathBuf::from(pf).join("Steam"));
            }
        }
    }
    #[cfg(not(windows))]
    if let Some(home) = home_dir() {
        roots.push(home.join(".steam").join("steam"));
        roots.push(home.join(".local").join("share").join("Steam"));
        roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        roots.push(home.join("Library/Application Support/Steam"));
    }

    let mut libs = Vec::new();
    for root in roots {
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(&vdf) {
            libs.extend(parse_library_folders(&text));
        }
        libs.push(root);
    }
    libs
}

/// Extracts `"path" "..."` values from Steam's `libraryfolders.vdf`.
pub fn parse_library_folders(vdf: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in vdf.lines() {
        let mut parts = line.split('"').filter(|s| !s.trim().is_empty());
        if parts.next() == Some("path")
            && let Some(p) = parts.next()
        {
            out.push(PathBuf::from(p.replace("\\\\", "\\")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vdf() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"D:\\SteamLibrary"
		"label"		""
	}
	"1"
	{
		"path"		"/mnt/games/steam"
	}
}"#;
        assert_eq!(
            parse_library_folders(vdf),
            vec![
                PathBuf::from(r"D:\SteamLibrary"),
                PathBuf::from("/mnt/games/steam")
            ]
        );
    }

    #[test]
    fn open_validates_and_finds_case_insensitively() {
        let tmp = std::env::temp_dir().join(format!("oss-assets-test-{}", std::process::id()));
        let gfx = tmp.join("Graphics");
        std::fs::create_dir_all(&gfx).unwrap();
        assert!(ContinuumDir::open(&tmp).is_err());
        // Minimal synthetic 1x1 24-bit BMP.
        let mut bmp = Vec::new();
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&58u32.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&[1, 2, 3, 0]);
        std::fs::write(gfx.join("SHIPS.BM2"), &bmp).unwrap();
        let dir = ContinuumDir::open(&tmp).unwrap();
        let img = dir.load_graphic("ships").unwrap();
        assert_eq!(img.pixel(0, 0), Some([3, 2, 1, 255]));
        assert!(matches!(
            dir.load_graphic("nope"),
            Err(AssetError::Missing(_))
        ));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
