# Headless clients and local test servers

Everything here runs without a zone operator's permission and without graphics, which makes
it the right way to try things, reproduce a bug, or put load on a server you run.

## `oss-bot` — a client with no window

```sh
cargo run --release -p oss-bot -- --server 127.0.0.1:5000 --ship 1 --count 3 --fly --chat "hi"
```

```
3 bot(s) connecting to 127.0.0.1:5000
[oss-bot1] connected (vie)
[oss-bot1] logged in
[oss-bot1] entering arena as pid 0
[oss-bot1] downloading map smallmap.lvl (crc b2fb5159)
[oss-bot1] in the arena with 3 player(s)
[oss-bot1] chat(2) oss-bot2> hi
```

`--count n` runs *n* of them in one process for a load swarm (names are numbered),
`--duration secs` makes them leave on their own, `--security opensec|vie|auto` picks the
cipher, `--new-user` registers the name rather than expecting it to exist, and `--identity
<file>` gives each bot number its own OpenSec identity. `--help` lists the rest.

`oss-bot` is built on `oss-session`, the same session layer the graphical client uses, so
what it proves about a server applies to the client too.

## A minimal relay zone

For tests that should not need a real server — login, one arena, map download, position and
chat relay, and nothing else:

```sh
cargo run -p oss-session --example test_zone -- --bind 127.0.0.1:5000 path/to/map.lvl
```

```
test zone on 127.0.0.1:5000 serving smallmap.lvl
```

It is deliberately not a game server: no scoring, no flags, no modules. Use
[oss-server](https://github.com/DangerMouseUK/oss-server) for anything beyond a smoke test.

## A local directory server

To try the zone browser without contacting anybody:

```sh
cargo run -p oss-session --example test_directory -- "Local Zone=127.0.0.1:5000"
cargo run --release -p oss-client -- --directory 127.0.0.1:4990 --zones
```

It lists only the zones you name on its command line, binds to localhost by default, and
contacts nothing itself. The client fills in the player counts and pings by pinging the zones
directly, as it does with a real directory.

## Driving the client without hands

For screenshots, soak tests and CI:

| Flag | Does |
|---|---|
| `--screenshot <file.png>` | Save a frame and exit |
| `--screenshot-at <secs>` | When to take it (default 2 s) |
| `--autopilot` | Fly and shoot automatically |
| `--send <secs>:<line>` | Type a line in game that many seconds after the arena loads, exactly as if you had typed it (repeatable) |
| `--follow <name>` | Spectate, following that player |
| `--record` / `--replay` | See [playing.md](playing.md#replays) |

```sh
oss-client --server 127.0.0.1:5000 --name Pilot --ship 1 \
  --autopilot --screenshot shot.png --screenshot-at 30
```

The screenshots in [the README](../README.md) were taken this way, against the zone that
ships in [oss-server](https://github.com/DangerMouseUK/oss-server).

**Use `--screenshot`, not F11, for anything you intend to publish.** F11 and Print Screen
print the full path of the file they just wrote into the chat log, so the *next* frame you
take shows a local path — `C:\Users\<name>\AppData\Roaming\OpenSubSpace\screenshots\…` or
its equivalent — in the bottom-left corner. That is how a machine path reaches a public
repository. `--screenshot` writes the file and exits without announcing it, and a frame
taken in a fresh session has an empty chat log anyway.
