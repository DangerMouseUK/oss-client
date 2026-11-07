# oss-client

An open-source, cross-platform game client for SubSpace/Continuum, written in Rust.

- Windows, macOS and Linux (browser later), any resolution, high refresh rates, gamepads.
- Plays on existing zones; feels like Continuum to veterans.
- Reads graphics and sounds from your own Continuum install; ships none.

Status: early development. The offline sandbox works; networked play is in progress.

## Try the sandbox

You need a Continuum install for graphics and sounds (the client finds Steam and
standard installs automatically, or set `OSS_CONTINUUM_DIR` / pass `--continuum-dir`).
Any `.lvl` map works, e.g. one from a zone you've visited (`Continuum/zones/<zone>/`):

```
cargo run --release -p oss-client -- --sandbox path/to/map.lvl
```

Default Continuum keys: arrows fly, Shift afterburner, Ctrl guns, Tab bombs,
Shift+Tab mines, Delete multifire, Esc menu (1-8 change ship, Q quit). Ctrl +/- changes
the pixel scale, F12 shows the frame rate. Gamepads: left stick/D-pad steer, RT/LT
thrust/reverse, bumpers afterburner, A guns, B bombs, X mines, Y multifire.
Run with `--help` for more options.

## Crates

- `oss-client` — the game (GPL-2.0-or-later).
- `oss-sim` — deterministic 100 Hz ship/weapon/collision simulation (MIT).
- `oss-assets` — `.bm2`/`.bmp`/`.png` and `.wa2`/`.wav` decoders, install discovery (MIT).

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
