# oss-client

An open-source, cross-platform game client for SubSpace/Continuum, written in Rust.

- Windows, macOS and Linux (browser later), any resolution, high refresh rates, gamepads.
- Plays on existing zones; feels like Continuum to veterans.
- Reads graphics and sounds from your own Continuum install; ships none.

Status: early development. The offline sandbox works, and the client can join a zone
(tested against the local test zone below; `oss-server` networking is in progress).

## Play online

```
cargo run --release -p oss-client -- --server 127.0.0.1:5000 --name YourName --ship 1
```

Enter opens the chat box (`'text` team chat, `:name:text` private, `=N` change team,
`?command` goes to the server). Esc opens the menu: 1-8 change ship, S spectate, Q quit.
Maps you download are cached per server.

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
- `oss-assets` — `.bm2`/`.bmp`/`.png` and `.wa2`/`.wav` decoders, install discovery (MIT).
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
