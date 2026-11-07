//! The asset search path: where graphics and sounds are looked up, in order.
//!
//! 1. `--assets-dir` / `OSS_ASSETS_DIR` (a folder of your own; may hold only some files)
//! 2. `--continuum-dir` / `OSS_CONTINUUM_DIR` (a Continuum install)
//! 3. `assets/classic` next to the executable
//! 4. the repository's `assets/classic` (development builds)
//! 5. an automatically discovered Continuum install
//!
//! Each file is taken from the first folder that has it, so a folder early in the list
//! can override individual files. At least one folder must hold a complete set
//! (`graphics/ships.bm2` is the marker).

use std::path::{Path, PathBuf};

use crate::image::{Image, decode_image};
use crate::locate::{AssetError, ContinuumDir, find_entry, is_install, read_file};
use crate::sound::{Sound, decode_wave};

/// Environment variable naming an asset folder that overrides the defaults.
pub const ASSETS_ENV_VAR: &str = "OSS_ASSETS_DIR";

/// Where to look, as configured by the user and the program.
#[derive(Debug, Clone, Default)]
pub struct AssetSearch {
    /// `--assets-dir`.
    pub assets_dir: Option<PathBuf>,
    /// `--continuum-dir`.
    pub continuum_dir: Option<PathBuf>,
    /// The repository's bundled folder, for development builds.
    pub dev_dir: Option<PathBuf>,
    /// Also try to find a Continuum install automatically.
    pub discover_install: bool,
}

/// An ordered list of asset folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetDirs {
    roots: Vec<PathBuf>,
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

impl AssetDirs {
    /// Builds the search path. A folder given explicitly (flag or environment) that
    /// doesn't exist is an error, so typos are noticed.
    pub fn locate(search: &AssetSearch) -> Result<Self, AssetError> {
        let mut roots = Vec::new();
        let mut tried = Vec::new();
        if let Some(dir) = search
            .assets_dir
            .clone()
            .or_else(|| env_path(ASSETS_ENV_VAR))
        {
            if !dir.is_dir() {
                return Err(AssetError::NotADirectory(dir));
            }
            roots.push(dir);
        }
        if let Some(dir) = search
            .continuum_dir
            .clone()
            .or_else(|| env_path(crate::ENV_VAR))
        {
            roots.push(ContinuumDir::open(dir)?.root().to_path_buf());
        }
        let bundled = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("assets").join("classic")));
        for dir in [bundled, search.dev_dir.clone()].into_iter().flatten() {
            if is_install(&dir) {
                roots.push(dir);
            } else {
                tried.push(dir);
            }
        }
        if !roots.iter().any(|r| is_install(r)) && search.discover_install {
            match ContinuumDir::locate(None) {
                Ok(c) => roots.push(c.root().to_path_buf()),
                Err(AssetError::NotFound(more)) => tried.extend(more),
                Err(e) => return Err(e),
            }
        }
        if !roots.iter().any(|r| is_install(r)) {
            tried.splice(0..0, roots);
            return Err(AssetError::NotFound(tried));
        }
        let mut seen = std::collections::HashSet::new();
        roots.retain(|r| seen.insert(r.clone()));
        Ok(Self { roots })
    }

    /// A search path of exactly these folders, in order.
    pub fn from_roots(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Finds `<root>/<subdir>/<stem>.<ext>` in the first root that has it, trying the
    /// extensions in order within each root; names match case-insensitively.
    pub fn find(&self, subdir: &str, stem: &str, exts: &[&str]) -> Option<PathBuf> {
        self.roots
            .iter()
            .find_map(|root| find_in(root, subdir, stem, exts))
    }

    /// Loads `graphics/<name>` (`.bm2`, `.bmp`, then `.png`) as RGBA.
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

fn find_in(root: &Path, subdir: &str, stem: &str, exts: &[&str]) -> Option<PathBuf> {
    let dir = find_entry(root, subdir)?;
    exts.iter()
        .find_map(|ext| find_entry(&dir, &format!("{stem}.{ext}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 1×1 24-bit BMP with the given colour.
    fn bmp(rgb: [u8; 3]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"BM");
        v.extend_from_slice(&58u32.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&54u32.to_le_bytes());
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&1i32.to_le_bytes());
        v.extend_from_slice(&1i32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&24u16.to_le_bytes());
        v.extend_from_slice(&[0; 24]);
        v.extend_from_slice(&[rgb[2], rgb[1], rgb[0], 0]);
        v
    }

    #[test]
    fn earlier_folders_override_files() {
        let base = std::env::temp_dir().join(format!("oss-asset-dirs-{}", std::process::id()));
        let (full, over) = (base.join("full"), base.join("override"));
        std::fs::create_dir_all(full.join("graphics")).unwrap();
        std::fs::create_dir_all(over.join("Graphics")).unwrap();
        std::fs::write(full.join("graphics/ships.bm2"), bmp([1, 2, 3])).unwrap();
        std::fs::write(full.join("graphics/tiles.bm2"), bmp([4, 5, 6])).unwrap();
        std::fs::write(over.join("Graphics/TILES.png.bm2"), b"ignored").unwrap();
        std::fs::write(over.join("Graphics/tiles.bmp"), bmp([7, 8, 9])).unwrap();

        let search = AssetSearch {
            assets_dir: Some(over.clone()),
            dev_dir: Some(full.clone()),
            ..Default::default()
        };
        let dirs = AssetDirs::locate(&search).unwrap();
        assert_eq!(dirs.roots(), &[over.clone(), full.clone()]);
        assert_eq!(
            dirs.load_graphic("tiles").unwrap().pixel(0, 0),
            Some([7, 8, 9, 255])
        );
        assert_eq!(
            dirs.load_graphic("ships").unwrap().pixel(0, 0),
            Some([1, 2, 3, 255])
        );

        // An override folder alone isn't enough: something must hold a full set.
        let partial = AssetSearch {
            assets_dir: Some(over),
            ..Default::default()
        };
        assert!(matches!(
            AssetDirs::locate(&partial),
            Err(AssetError::NotFound(_))
        ));
        let missing = AssetSearch {
            assets_dir: Some(base.join("nope")),
            ..Default::default()
        };
        assert!(matches!(
            AssetDirs::locate(&missing),
            Err(AssetError::NotADirectory(_))
        ));
        let _ = std::fs::remove_dir_all(&base);
    }
}
