# assets/classic — Continuum's graphics and sounds

**These files are not this project's work.** The 95 `.bm2` graphics and 81 `.wa2` sounds in
this directory are the original SubSpace/Continuum artwork and sound effects, in Continuum's
own container formats and under its own filenames. They originate with those games, not with
this project, and this project claims no ownership of them.

**They are not covered by this repository's licence.** The GPL-2.0-or-later in
[`LICENSE`](../../LICENSE) applies to the code. It does not apply to these files, and
nothing here relicenses them or makes any claim about what their licence is.

## Why they are bundled

So that the client is playable from a fresh clone, with no separate download and no
Continuum install. Bundling Continuum's assets is what other SubSpace client projects do,
and this follows that practice.

## The bundle is a convenience, not a dependency

The client finds graphics and sounds by searching, file by file, in this order
([docs/settings.md](../../docs/settings.md) has the detail):

1. `--assets-dir <dir>` / `OSS_ASSETS_DIR` — your own folder
2. `--continuum-dir <dir>` / `OSS_CONTINUUM_DIR` — a Continuum install
3. `assets/classic` beside the program, and this directory for a binary built from a
   checkout
4. a Continuum install found automatically (Steam or the standard paths)

Anything earlier in that list wins, file by file, so a single replacement file overrides a
single bundled one. A build with this directory removed still works for anyone who has
Continuum installed, or who points the client at their own asset folder: steps 1, 2 and 4
need nothing from here. A packaged build can also ship its own `assets/classic` beside the
executable instead of this one.

Nothing in the client's code depends on these particular files existing.
