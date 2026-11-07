//! Decodes every graphic and sound in a real Continuum install. Skipped unless
//! `OSS_CONTINUUM_DIR` points at one (assets are never part of this repo).

use oss_assets::{ContinuumDir, decode_image, decode_wave};

fn install() -> Option<ContinuumDir> {
    let dir = std::env::var_os(oss_assets::ENV_VAR)?;
    Some(
        ContinuumDir::open(std::path::PathBuf::from(dir))
            .expect("OSS_CONTINUUM_DIR is not a Continuum folder"),
    )
}

#[test]
fn decodes_every_graphic() {
    let Some(dir) = install() else { return };
    let mut count = 0;
    for entry in std::fs::read_dir(dir.root().join("graphics"))
        .unwrap()
        .flatten()
    {
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
    let ships = dir.load_graphic("ships").unwrap();
    assert_eq!((ships.width, ships.height), (360, 1152));
    let tiles = dir.load_graphic("tiles").unwrap();
    assert_eq!((tiles.width, tiles.height), (304, 160));
}

#[test]
fn decodes_every_sound() {
    let Some(dir) = install() else { return };
    let mut count = 0;
    for entry in std::fs::read_dir(dir.root().join("sound"))
        .unwrap()
        .flatten()
    {
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
