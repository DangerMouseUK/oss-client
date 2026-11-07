//! Decodes every graphic and sound in an asset folder: `OSS_ASSETS_DIR`,
//! `OSS_CONTINUUM_DIR`, or the repository's `assets/classic`. Skipped when none exists.

use oss_assets::{AssetDirs, decode_image, decode_wave};

fn install() -> Option<std::path::PathBuf> {
    // OSS_ASSETS_DIR / OSS_CONTINUUM_DIR, else the repository's bundled folder.
    let dir = std::env::var_os(oss_assets::ASSETS_ENV_VAR)
        .or_else(|| std::env::var_os(oss_assets::ENV_VAR))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/classic")
        });
    let dirs = AssetDirs::from_roots(vec![dir.clone()]);
    dirs.find("graphics", "ships", &["bm2"]).map(|_| dir)
}

#[test]
fn decodes_every_graphic() {
    let Some(dir) = install() else { return };
    let mut count = 0;
    for entry in std::fs::read_dir(dir.join("graphics")).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !(name.ends_with(".bm2") || name.ends_with(".bmp")) {
            continue;
        }
        let data = std::fs::read(entry.path()).unwrap();
        let img = decode_image(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(img.rgba.len(), img.width as usize * img.height as usize * 4);
        count += 1;
    }
    assert!(count > 50, "only {count} graphics decoded");
    let dirs = AssetDirs::from_roots(vec![dir.clone()]);
    let ships = dirs.load_graphic("ships").unwrap();
    assert_eq!((ships.width, ships.height), (360, 1152));
    let tiles = dirs.load_graphic("tiles").unwrap();
    assert_eq!((tiles.width, tiles.height), (304, 160));
}

#[test]
fn decodes_every_sound() {
    let Some(dir) = install() else { return };
    let mut count = 0;
    for entry in std::fs::read_dir(dir.join("sound")).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !(name.ends_with(".wa2") || name.ends_with(".wav")) {
            continue;
        }
        let data = std::fs::read(entry.path()).unwrap();
        let snd = decode_wave(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(snd.frames() > 0, "{name} is empty");
        count += 1;
    }
    assert!(count > 50, "only {count} sounds decoded");
}
