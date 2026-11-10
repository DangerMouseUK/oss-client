# oss-client

An open-source, cross-platform game client for SubSpace/Continuum, written in Rust.

- Windows, macOS and Linux (browser later), any resolution, high refresh rates, gamepads.
- Plays on existing zones; feels like Continuum to veterans.
- Graphics and sounds ship in `assets/classic`, which your own folder can override.

Status: in development. The client joins real zones: it logs in, downloads and caches
the map, LVZ graphics and the zone news, renders the arena, flies, fights, and has
chat, macros, replays, a settings screen, screenshots, accessibility palettes,
gamepad remapping and a zone browser. Continuum's own encryption is not in this
repository, so zones that require it cannot be joined from here.

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
Up/Down in the chat box bring back lines you sent.

The client answers a few commands itself, as Continuum does: `?go [arena]` changes
arena, `?namelen N` sets the width of the sender column in chat, `?status` shows how
far your ship is upgraded, and `?news` opens the zone's news (also N in the menu).
Continuum's `%` substitutions work in anything you send: `%selfname`, `%tickname`,
`%squad`, `%freq`, `%bounty`, `%flags`, `%energy`, `%killed`, `%killer`, `%coord`,
`%redname`, `%redbounty`, `%redflags`.

F2 cycles the stat box views, Page Up/Down select a player. Esc opens the menu: 1-8
change ship, S spectate, N zone news, O options, Q quit; the whole chat log is on
screen while the menu is open, and Page Up/Down scroll it. Maps, LVZ files and the
news are cached per server.

## Zone browser

F3 on the login screen lists zones from the directory servers you configure
(`[directory] servers = host:port` in the settings file, or `--directory host:port`),
with population and ping. None are configured by default. For local testing:

```
cargo run -p oss-session --example test_directory -- "Local=127.0.0.1:5000"
cargo run --release -p oss-client -- --directory 127.0.0.1:4990 --zones
```

## Replays

`--record visit.ossr` records a visit (everything the server sent and the client sent,
plus the map and LVZ files), and `--replay visit.ossr` plays it back without a server,
spectating and following the player who recorded it. A recording cut short still plays.

## Encryption

Connections try OpenSec first and fall back to VIE for servers that don't speak it.
The first time a server proves an OpenSec key, the key is remembered in
`known_servers.txt` beside the settings file. From then on that server must prove
the same key over OpenSec, with no fallback. `--security auto|opensec|vie` or
`[network] security` changes the policy.

Over OpenSec the client also offers the zone an identity key, which zones can use
for bans. Each zone gets its own key, so zones can't link your sessions. The keys
are kept in `identities.txt` beside the settings file. Keep it private: anyone
with a copy can use your identity at those zones. `[network] identity = false`
turns this off.

## Settings

`oss-client.ini` in your settings folder (`%APPDATA%\OpenSubSpace` on Windows,
`~/.config/open-subspace` elsewhere, or `--config <file>`) holds video, audio, your
player name, every key and gamepad binding, macro keys (`[macros]`, e.g.
`Ctrl+F1 = 'help at base`), the accessibility palette and interface scale, and the
encryption policy. It is written with the defaults on first run; command-line flags
override it.

Print Screen or F11 saves a screenshot beside the settings file. `[accessibility]
palette` picks a colour-blind or high-contrast palette, and `ui_scale` draws the
interface larger than the world.

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

A minimal relay zone (login, one arena, map download, position/chat relay; not a real
server) is there for tests that should not need a server:

```
cargo run -p oss-session --example test_zone -- --bind 127.0.0.1:5000 path/to/map.lvl
```

## Crates

- `oss-client` — the game (GPL-2.0-or-later).
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

GPL-2.0-or-later (see `LICENSE`). The `oss-assets` crate is MIT so bots and tools can reuse it. The simulation (`oss-sim`, MIT) lives in [oss-core](https://github.com/DangerMouseUK/oss-core), shared with the server.

SubSpace and Continuum are the names of the original games; this project is an independent, compatible implementation and is not affiliated with their authors.
