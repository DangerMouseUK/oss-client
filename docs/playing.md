# Playing

Everything the client does at the keyboard, for someone who knows SubSpace and for someone
who does not. The defaults are Continuum's, so a veteran should not have to read this.

**The keys below are those defaults.** Change one on the settings screen and this page
cannot know: **F1 in the game is the one place that shows the keys you actually have**,
because it reads them out of the table rather than having them written down beside it.
That is how Continuum's own help works, and it is why this page says "the defaults"
rather than "the keys".

## Flying

| Key | Does |
|---|---|
| Arrows | Turn and thrust |
| Shift | Afterburner |
| Ctrl | Guns |
| Tab | Bombs |
| Shift+Tab | Mines |
| Delete | Multifire |
| Esc | Menu — 1-8 change ship, S spectate, A arena list (in a zone), N zone news, O options, F frame rate, Q quit the game. F2 and F12 work with it open, and so do the flying keys |
| Enter | Send what you have typed |
| F1 | Help pages (F1 or Page Down turns the page, Page Up goes back, Esc closes). A zone that sends its own help pages, as some do inside their LVZ files, has them shown first, and this client's own page follows |
| F2 | Cycle the stat box views |
| Page Up / Page Down | Select a player in the stat box (and scroll the chat log while the menu is open) |
| F3 | Zone browser (on the login screen) |
| F5 | Offline sandbox (on the login screen) |
| F11 | Spectator mode on and off, as in Continuum (off goes back to the ship you last flew) |
| Print Screen | Save a screenshot (a `screenshots` folder beside the settings file) |
| F12 | Frame rate and ping; press again for the technical detail (pixel scale, how frames reach the screen, tick count), and again to hide it |
| Ctrl +/- | Pixel scale |

Gamepads: left stick or D-pad steers, RT/LT thrust and reverse, bumpers afterburner, A guns,
B bombs, X mines, Y multifire. Every key and button is rebindable in the settings file.

While the menu is open the whole chat log is on screen, and Page Up/Down scroll it.

Esc then A, or typing `?arena`, shows the zone's arenas in a box at the top of the screen,
yours first and marked `>`, each with how many are playing in it. Page Up and Page Down
choose one, Enter goes there, and a click on a row goes there too. Esc puts the list away.
The game carries on underneath, so you can still fly and type while it is up.

## Chat

There is no chat box to open. Type, and the characters go to the chat line; Enter sends it.
That is how SubSpace and Continuum work, and the original client's own help says so: "To
send a public message simply type it, then hit Enter."

One key is both a character and a game key — the backquote, which is repel. It repels while
the chat line is empty and types a backquote once there is something on the line, which is
the rule the originals use for any such key, macro keys included. Everything else that flies
the ship types nothing, so you keep flying, shooting and laying mines while you talk.
Ctrl+Backspace clears the line.

The prefixes are Continuum's:

| Type | Sends |
|---|---|
| `text` | Public |
| `'text` or `//text` | Your team |
| `"text` | The selected player's team |
| `:name:text` | Private to *name* (Tab completes it) |
| `::text` | Reply to the last private message |
| `/text` | Private to the player selected in the stat box |
| `#text` | Your squad, wherever its members are in the zone (refused, not sent, if you are in none) |
| `;text` or `;N;text` | Chat channel (the current one, or channel *N*) |
| `=N` | Change team |
| `?command` | To the server |

Remote players and `:#squad:name` go through the biller. Typing `::` on an empty line fills
in the last person who messaged you, and `::` again walks back through the five before them.

### Commands the client answers itself

As Continuum does, a few `?commands` never reach the server:

| Command | Does |
|---|---|
| `?go [arena]` | Change arena (no argument: the public arena) |
| `?namelen N` | Width of the sender column in chat (`?namelen=N` works too: nullspace takes the space, SubSpace's own help documents the `=`) |
| `?status` | How far your ship is upgraded |
| `?news` | Open the zone news (also N in the menu). This one is ours: Continuum opens the news in a window of its own, which this client has no equivalent of |
| `?quit` | Leave the game for the login screen, the same as Q in the menu. Ours — neither the original client nor ASSS has a `?quit` |

### Leaving

Q in the menu leaves the **game**, not the program: it disconnects and puts you back on
the login screen, with the zone and name you were playing under still filled in. That is
what SubSpace and Continuum do — their Quit returns you to the front end, and only the
front end's own Quit (here, Esc on the login screen) ends the program.

Like both originals, it waits until you have half your energy back before it takes
effect, so a fight cannot be quit out of; it says `Exiting game...` while it waits, which
is the original's own wording. Spectators and a dead ship leave at once, and offline
there is nobody to dodge.

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

F3 on the login screen lists zones from the directory servers, with population and ping, and
the selected zone's own description underneath. **Three are configured by default** — the
game's own — and the client asks them for the zone list when the login screen opens, as well
as each time you press F3.

Set your own with `[directory] servers = host:port` in the settings file, or
`--directory host:port` on the command line (repeatable). **The flag adds to the file's list
rather than replacing it**, so pointing the client at a local test directory does not stop the
built-in ones being asked as well.

**`[directory] servers = none` turns the zone browser off** and the client then contacts no
directory at all. An empty value brings the built-in list back.

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
