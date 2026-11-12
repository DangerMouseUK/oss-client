# Playing

Everything the client does at the keyboard, for someone who knows SubSpace and for someone
who does not. The defaults are Continuum's, so a veteran should not have to read this.

## Flying

| Key | Does |
|---|---|
| Arrows | Turn and thrust |
| Shift | Afterburner |
| Ctrl | Guns |
| Tab | Bombs |
| Shift+Tab | Mines |
| Delete | Multifire |
| Esc | Menu — 1-8 change ship, S spectate, N zone news, O options, Q quit |
| Enter | Chat box |
| F2 | Cycle the stat box views |
| Page Up / Page Down | Select a player in the stat box (and scroll the chat log while the menu is open) |
| F3 | Zone browser (on the login screen) |
| F5 | Offline sandbox (on the login screen) |
| F11 / Print Screen | Save a screenshot (a `screenshots` folder beside the settings file) |
| F12 | Frame rate |
| Ctrl +/- | Pixel scale |

Gamepads: left stick or D-pad steers, RT/LT thrust and reverse, bumpers afterburner, A guns,
B bombs, X mines, Y multifire. Every key and button is rebindable in the settings file.

While the menu is open the whole chat log is on screen, and Page Up/Down scroll it.

## Chat

Enter opens the chat box. The prefixes are Continuum's:

| Type | Sends |
|---|---|
| `text` | Public |
| `'text` or `//text` | Your team |
| `"text` | The selected player's team |
| `:name:text` | Private to *name* (Tab completes it) |
| `::text` | Reply to the last private message |
| `/text` | Private to the player selected in the stat box |
| `;text` or `;N;text` | Chat channel (the current one, or channel *N*) |
| `=N` | Change team |
| `?command` | To the server |

Remote players and `:#squad:name` go through the biller. Up and Down in the chat box bring
back lines you have sent.

### Commands the client answers itself

As Continuum does, a few `?commands` never reach the server:

| Command | Does |
|---|---|
| `?go [arena]` | Change arena (no argument: the public arena) |
| `?namelen N` | Width of the sender column in chat |
| `?status` | How far your ship is upgraded |
| `?news` | Open the zone news (also N in the menu). This one is ours: Continuum opens the news in a window of its own, which this client has no equivalent of |

### Substitutions

Continuum's `%` substitutions work in anything you send: `%selfname`, `%tickname`, `%squad`,
`%freq`, `%bounty`, `%flags`, `%energy`, `%killed`, `%killer`, `%coord`, `%redname`,
`%redbounty`, `%redflags`.

### Macros

`[macros]` in the settings file binds a key to a line of chat:

```ini
[macros]
Ctrl+F1 = 'help at base
```

## The zone browser

F3 on the login screen lists zones from the directory servers you have configured, with
population and ping, and the selected zone's own description underneath. Configure them with
`[directory] servers = host:port` in the settings file or `--directory host:port` on the
command line (repeatable; the flag adds to the file's list). **None are configured by
default** — the client contacts nothing you have not asked it to.

For a local test directory, see [testing.md](testing.md).

## Replays

```sh
oss-client --server zone:5000 --name YourName --record visit.ossr
oss-client --replay visit.ossr
```

`--record` writes everything the server sent and everything the client sent, plus the map and
the LVZ files, into one `.ossr` file. `--replay` plays it back with no server at all,
spectating and following the player who recorded it. A recording that was cut short still
plays — the format is described in
[oss-core's docs/formats/replay.md](https://github.com/DangerMouseUK/oss-core/blob/main/docs/formats/replay.md).

## The offline sandbox

F5 on the login screen, or:

```sh
oss-client --sandbox
oss-client --sandbox path/to/map.lvl --lvz path/to/art.lvz
```

Stock settings, no server, any `.lvl` map. Useful for looking at a map, checking an LVZ file,
or trying the controls without logging in anywhere.
