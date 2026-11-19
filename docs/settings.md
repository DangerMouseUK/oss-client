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
Print Screen saves into a `screenshots` folder there, and maps, LVZ files and zone
news are cached per server. The key prints the file they just wrote into the chat log,
so a screenshot taken after another one shows a local path; use `--screenshot` (see
[testing.md](testing.md)) for anything you intend to publish.

## The settings window

Most of the file can be changed from inside the client. Press **F2** (or click **Options**) on
the start-up screen, or **Esc then O** in the game. The window is drawn in the game's own box:
the sections down the left — Video, Audio, Accessibility, Keys, Gamepad, Macros — and the
picked section's settings on the right.

| Section | Holds |
|---|---|
| Video | Pixel scale (or "from display"), fullscreen, vsync |
| Audio | Mute, volume |
| Accessibility | Colour palette, interface size |
| Keys | The keys for every action: Change, Add or Clear each one, or restore Continuum's defaults |
| Gamepad | The button for every action: Change or Clear each one, or restore the default layout |
| Macros | Keys that send a line of chat: add one, type its line, or remove it |

It works fully from the keyboard and fully from the pointer:

| Key | Does | With the pointer |
|---|---|---|
| Tab / Shift+Tab | Next or previous section | Click a section |
| Up / Down (PgUp, PgDn, Home, End) | Move between lines | Click a line; the wheel, or `^` `v` beside the heading, scrolls a long list |
| Left / Right | Change a number or a choice, flip a switch, or move along a line's buttons | Click `<` or `>`, a switch, or a button |
| Enter | Change the picked line or press the picked button | Click it |
| Ctrl+S | Save to the settings file | Click `Ctrl+S = Save` |
| Esc | Close. With changes not yet saved, the first Esc says so and the second closes without saving | Click `Esc = Close` |

Change or Add on a key waits for the next key you press, with Shift or Ctrl if held; Esc
cancels the wait, and so does a click. On a macro, typing goes into the line it sends.
The line at the foot says what the picked line, or the one under the pointer, does.

Save writes the file and applies at once everything that can change while playing: keys,
buttons, macros, volume, pixel scale, fullscreen, palette and interface size. Vsync and
mute take effect at the next start. The window shows the palette you are picking as you
pick it, so you see it before you save it. It does not have a player name: that is set on
the start-up screen.

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

`assets/classic` in this repository holds Continuum's own graphics and sounds, bundled so
that a fresh clone plays with no Continuum install anywhere. They are not this project's
work and are not covered by its GPL licence;
[assets/classic/README.md](../assets/classic/README.md) says what they are and whose. If you
have an install of your own, `--continuum-dir` uses it instead: `oss-assets` reads
Continuum's `.bm2` and `.wa2` containers as well as `.bmp`, `.png`, `.jpg`, `.gif` and
`.wav`.

## LVZ

Online, an arena's LVZ files — zone graphics: map and screen objects, images, animations and
the server's toggles for them — are downloaded and cached along with the map. Offline you can
point at one directly:

```sh
oss-client --sandbox path/to/map.lvl --lvz path/to/art.lvz
```

`--lvz` is repeatable and also works in a zone, on top of whatever the arena sends.
