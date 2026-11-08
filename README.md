# oss-client

An open-source, cross-platform game client for SubSpace/Continuum, written in Rust.

- Windows, macOS and Linux (browser later), any resolution, high refresh rates, gamepads.
- Plays on existing zones; feels like Continuum to veterans.
- Graphics and sounds come from an asset folder, which your own folder can override.

Status: early development. The offline sandbox works, and the client can join a zone
(tested against the local test zone below; `oss-server` networking is in progress).

## Play online

Run `oss-client` with no arguments for the login screen (zone, name, password, arena,
ship; F5 there starts the offline sandbox), or skip it:

```
cargo run --release -p oss-client -- --server 127.0.0.1:5000 --name YourName --ship 1
```

Enter opens the chat box: `'text` or `//text` team, `"text` the selected player's team,
`:name:text` private (Tab completes the name; remote players and `:#squad:` go through
the biller), `::text` reply, `/text` private to the player selected in the stat box,
`;text` or `;N;text` chat channels, `=N` change team, `?command` goes to the server.
F2 cycles the stat box views, Page Up/Down select a player. Esc opens the menu: 1-8
change ship, S spectate, Q quit. Maps you download are cached per server.

## Settings

`oss-client.ini` in your settings folder (`%APPDATA%\OpenSubSpace` on Windows,
`~/.config/open-subspace` elsewhere, or `--config <file>`) holds video, audio, your
player name, every key binding and macro keys (`[macros]`, e.g. `Ctrl+F1 = 'help at base`). It is written with the defaults on first run;
command-line flags override it.

## Try the sandbox

Graphics and sounds are looked up, file by file, in this order:

1. `--assets-dir <dir>` / `OSS_ASSETS_DIR` (your own folder; it may hold only some files)
2. `--continuum-dir <dir>` / `OSS_CONTINUUM_DIR` (a Continuum install)
3. `assets/classic` next to the program (and the repository's `assets/classic` in
   development builds)
4. a Continuum install found automatically (Steam or standard paths)

Run the sandbox on the built-in arena, or on any `.lvl` map:

```
cargo run --release -p oss-client -- --sandbox
cargo run --release -p oss-client -- --sandbox path/to/map.lvl --lvz path/to/art.lvz
```

Online, the arena's LVZ files (zone graphics: map and screen objects, BMP/PNG/JPEG/GIF
images, animations, server toggles) are downloaded and cached with the map.

Default Continuum keys: arrows fly, Shift afterburner, Ctrl guns, Tab bombs,
Shift+Tab mines, Delete multifire, Esc menu (1-8 change ship, Q quit). Ctrl +/- changes
the pixel scale, F12 shows the frame rate. Gamepads: left stick/D-pad steer, RT/LT
thrust/reverse, bumpers afterburner, A guns, B bombs, X mines, Y multifire.
Run with `--help` for more options.

## Headless bot

`oss-bot` connects to a zone without graphics, for testing and load:

```
cargo run --release -p oss-bot -- --server 127.0.0.1:5000 --ship 1 --count 20 --fly --chat "hi"
```

Until `oss-server` accepts network clients, a minimal relay zone (login, one arena, map
download, position/chat relay; not a real server) is available for local testing:

```
cargo run -p oss-session --example test_zone -- --bind 127.0.0.1:5000 path/to/map.lvl
```

## Crates

- `oss-client` — the game (GPL-2.0-or-later).
- `oss-sim` — deterministic 100 Hz ship/weapon/collision simulation (MIT).
- `oss-assets` — `.bm2`/`.bmp`/`.png`/`.jpg`/`.gif` and `.wa2`/`.wav` decoders, the asset search path (MIT).
- `oss-session` — the client side of a game session over `oss-net`: login, arena entry,
  settings, map download/cache, security answers, players, positions, chat. Sans-IO,
  with a UDP driver.
- `oss-bot` — headless client / load swarm on `oss-session`.

## Building

`oss-client` uses the shared crates in [oss-core](https://github.com/DangerMouseUK/oss-core) as a sibling checkout:

```
git clone https://github.com/DangerMouseUK/oss-core
git clone https://github.com/DangerMouseUK/oss-client
cd oss-client
cargo run --release
```

## Licence

GPL-2.0-or-later (see `LICENSE`). The `oss-sim` and `oss-assets` crates are MIT so bots and tools can reuse them.

SubSpace and Continuum are the names of the original games; this project is an independent, compatible implementation and is not affiliated with their authors.
