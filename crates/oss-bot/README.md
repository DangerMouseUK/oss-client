# oss-bot

**A SubSpace client with no window.** It connects to a zone, logs in, enters an arena, takes the
arena's settings and map, answers the zone's security checks, and then either sits in spectator
mode or flies a ship around — thrusting, turning and firing — sending its position as a player
would. Many bots can run in one process.

It is for people who run or build zones: checking that a zone is up and accepting players,
watching a new arena from inside, or putting a load of players on a server before real ones
arrive. It is built on `oss-session`, the same session layer the graphical
[oss-client](https://github.com/DangerMouseUK/oss-client) uses, so what it proves about a server
applies to the client too.

It draws nothing and plays no sound, so it needs no graphics or sound files.

## Run it

```sh
oss-bot --server 127.0.0.1:5000 --name MyBot --password secret
```

Replace the address with the zone's, and the name and password with an account the zone accepts.
On a zone that does not know the name -- one that has just started and keeps no accounts, say --
add `--new-user`: when the zone offers to create the name, the bot says yes. Without it the bot
declines and disconnects, since it has nobody to ask. The bot prints each step as it happens — connected, logged
in, entering the arena, downloading the map — so a zone that stops it part-way shows where.

**`oss-bot --help` lists the options the program you have accepts** — not every build of the bot
takes every option below, and the list it prints is the one to trust. The main ones:

| option | what it does |
|---|---|
| `--server <host:port>` | the zone to connect to (required) |
| `--name <name>`, `--password <pw>` | the account to log in with |
| `--arena <name>` | the arena to enter; otherwise any public one |
| `--ship <1-8\|spec>` | the ship to fly, or spectator mode (the default) |
| `--fly` | fly around and fire at random (needs a ship) |
| `--count <n>` | run *n* bots in one process; their names are numbered |
| `--chat <text>` | say this once in the arena |
| `--duration <secs>` | leave after this long, instead of running until Ctrl+C |
| `--security <mode>` | which encryption to use: `vie`, `opensec`, `auto` or `none` |

A swarm of three ships flying around a local zone:

```sh
oss-bot --server 127.0.0.1:5000 --ship 1 --count 3 --fly --new-user
```

## Building it from source

From a checkout of [oss-client](https://github.com/DangerMouseUK/oss-client), with
[oss-core](https://github.com/DangerMouseUK/oss-core) checked out beside it:

```sh
cargo build --release -p oss-bot
```

[`docs/testing.md`](../../docs/testing.md) shows the bot driving a local zone, alongside the
other ways this project tests a server.

## Licence

GPL-2.0-or-later, as the rest of `oss-client`. The full text is in `LICENSE` beside this file in
a download, and in [`LICENSE`](../../LICENSE) at the top of the repository.

SubSpace and Continuum are the names of the original games; this project is an independent,
compatible implementation and is not affiliated with their authors.
