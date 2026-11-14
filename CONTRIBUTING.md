# Contributing to oss-client

Thanks for looking. This file is the short version of what a change needs to be
mergeable here, so that you find out before you spend an evening on it rather
than after.

## Before you commit anything

**Read [the "What must never reach this repository" section of
`CLAUDE.md`](CLAUDE.md#what-must-never-reach-this-repository).** It is not long,
it is the same in all four of the project's public repositories, and it applies
to commit messages, branch names and pull-request text as well as to files.

Two things about it that are easy to miss: it is a **test**, not just a list —
the list is worked examples — and the local guard that enforces part of it is
**not present in a fresh clone**. If you are working from your own clone, the
written rule and review are the only checks there are. If you think something
you want to write might be covered, ask in an issue first. Nobody will mind.

## Licences, and the one trap inside this repository

This repository is **GPL-2.0-or-later** — except for one crate.

> **`oss-assets` is MIT**, so that bots and tools can reuse it without taking on
> a GPL obligation. **GPL-licensed code must not go into `oss-assets`.** It may
> go elsewhere in this repository.

That boundary runs *inside* the repository, which is easy to miss. If your
change touches `crates/oss-assets`, everything in it has to be MIT-compatible:
your own work, or MIT sources such as nullspace with attribution in a comment.

Elsewhere in the repository there is no licence barrier to GPL code. Nothing
here has ever come from ASSS — no source and no data, as
[the README's licence section](README.md#licence) says — but that is a
**description of the code that is here**, not a prohibition on what may arrive.
A contribution carrying GPL code into the GPL part of this repository is not
forbidden and will not be refused on principle. Tell us in the pull request that
that is what you did, and name the source in the file headers.

**A note on the other repository:**
[oss-core](https://github.com/DangerMouseUK/oss-core) is **MIT throughout**, and
GPL code must not go into it at all. Do not carry this page's answer across —
read [oss-core's `CONTRIBUTING.md`](https://github.com/DangerMouseUK/oss-core/blob/main/CONTRIBUTING.md).

## The bundled graphics and sounds

`assets/classic/` holds Continuum's own artwork and sound effects. **They are
not this project's work, they are not covered by the GPL, and no ownership of
them is claimed** —
[`assets/classic/README.md`](assets/classic/README.md) says so in full.

Practically, for a contributor: do not edit those files, do not add more of
them, and do not write code that assumes they are present. The client finds
assets by searching a user's own folder, a Continuum install, and then the
bundle — so a build with that directory removed still has to work.

## The invariants

These are what review will actually check:

- **Untrusted network and file data never panics the client.** A zone you
  connect to is a stranger's machine, and it sends you settings, maps and
  archives. Treat all of it as hostile input.
- **Simulation changes belong in `oss-core`, not here.** `oss-sim` is shared:
  the client uses it for prediction, the server uses it for plausibility checks,
  from one body of code. Working around it locally makes the two disagree, and a
  disagreement produces false anti-cheat findings against real players. Request
  the change upstream instead.
- **No floating-point nondeterminism** in anything that has to match the server
  or Continuum. The simulation is fixed-step integer work for this reason.
- **Keep rendering WebGPU-compatible.** There is a browser build later, and
  `wgpu` code that only works natively costs more to unpick than to avoid.
- Tests use synthetic data. Do not commit third-party binaries, zone downloads,
  captures, personal data or machine paths.

## Before you open a pull request

Run all four. CI runs the same ones, so this only saves you a round trip:

```sh
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

**`cargo fmt`, not `cargo fmt --all`.** This one matters and it is not a style
preference: `--all` takes its package list from `cargo metadata`, which includes
path dependencies outside this workspace. In this repository that hands rustfmt
every source root in a sibling `oss-core` checkout — including crates this
repository does not even depend on — and **rewrites it in place**, across a
repository boundary, without saying so. Plain `cargo fmt` covers exactly this
repository's own crates.

**The fourth gate is not optional and the other three do not stand in for it.**
It is the only one that catches a rustdoc fault — a public item linking to a
private one, a broken intra-doc link, a bad code fence — and `main` has gone red
on exactly that after the other three passed locally.

## Changes that span repositories

Protocol, transport, format and simulation code lives in
[oss-core](https://github.com/DangerMouseUK/oss-core) and is shared with the
server. **Do not fork it into this repository**; request the change there, and
it lands there first because this repository builds against it.

## Practical things

- **Open an issue first for anything large**, especially anything that changes
  how the game feels to play. For a typo or an obvious fix, just send the pull
  request.
- Commits: imperative subject, small and focused.
- There is no CLA and no copyright assignment. Your contribution is yours, under
  this repository's licence.
- One maintainer, volunteering. Reviews may be slow. Pinging a quiet pull
  request after a week or so is welcome, not rude.

## Security

**Do not report a security vulnerability in a pull request or a public issue.**
See [`SECURITY.md`](SECURITY.md) — there is a private channel, and it is the
right way in. The case that matters most here is what a hostile zone can do to
somebody who connects to it.
