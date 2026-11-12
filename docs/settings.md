# Settings, graphics and sound

## The settings file

`oss-client.ini` lives in your settings folder — `%APPDATA%\OpenSubSpace` on Windows,
`~/.config/open-subspace` elsewhere — or wherever `--config <file>` says. It is written with
the defaults on the first run, and command-line flags override it for that run without
rewriting it.

It holds video and audio settings, your player name, every key and gamepad binding, macro
keys (`[macros]`, e.g. `Ctrl+F1 = 'help at base`), the accessibility palette and interface
scale, the directory servers for the zone browser (`[directory] servers = host:port`), and
the encryption policy (`[network] security`, `[network] identity`).

`known_servers.txt` and `identities.txt` sit beside it (see [encryption.md](encryption.md)),
Print Screen and F11 save into a `screenshots` folder there, and maps, LVZ files and zone
news are cached per server. Those two keys print the file they just wrote into the chat log,
so a screenshot taken after another one shows a local path; use `--screenshot` (see
[testing.md](testing.md)) for anything you intend to publish.

## Accessibility and scaling

`[accessibility] palette` picks a colour-blind or high-contrast palette; `ui_scale` draws the
interface larger than the world, so the chat, stat box and menus can be read at a size that
suits you without zooming the game. `--scale <n>` sets the integer pixel scale for the world
(the default comes from the display's DPI), `Ctrl +/-` changes it in game, and `--fullscreen`
is borderless. `--vsync` caps the frame rate to the display refresh; `--mute` turns the sound
off.

## Where graphics and sounds come from

Each file is looked up, one at a time, in this order — so a folder that holds only a few
files overrides only those:

1. `--assets-dir <dir>` / `OSS_ASSETS_DIR` — your own folder
2. `--continuum-dir <dir>` / `OSS_CONTINUUM_DIR` — a Continuum install
3. `assets/classic` beside the program, and the repository's `assets/classic` for a binary
   built from a checkout (any profile, so `cargo run --release` works too)
4. a Continuum install found automatically (Steam or the standard paths)

`assets/classic` in this repository is the default set, so a fresh clone plays with no
Continuum install anywhere. If you do have one, `--continuum-dir` gives you the
graphics and sounds you are used to: `oss-assets` reads Continuum's own `.bm2` and `.wa2`
containers as well as `.bmp`, `.png`, `.jpg`, `.gif` and `.wav`.

## LVZ

Online, an arena's LVZ files — zone graphics: map and screen objects, images, animations and
the server's toggles for them — are downloaded and cached along with the map. Offline you can
point at one directly:

```sh
oss-client --sandbox path/to/map.lvl --lvz path/to/art.lvz
```

`--lvz` is repeatable and also works in a zone, on top of whatever the arena sends.
