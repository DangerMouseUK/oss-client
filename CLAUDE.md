# oss-client — agent conventions

Rust game client for SubSpace/Continuum. Licence **GPL-2.0-or-later**; `oss-assets` is **MIT** (reusable by bots/tools). Behavioural references: nullspace (MIT, C++ client), mervbot (bot core), ASSS (server-side truth for physics/settings semantics).

## Architecture
- Protocol, transport and formats come from the sibling `../oss-core` crates (`oss-proto`, `oss-net`, `oss-formats`). Don't fork them here; request changes there.
- Stack: `winit` (window/input), `wgpu` (rendering; keep it WebGPU-compatible for the later browser build), `gilrs` (gamepads), a cpal-based audio crate, `egui` for menus/tools. In-game HUD is a custom batched sprite renderer that defaults to Continuum's layout.
- `oss-sim` lives in `../oss-core` and is shared with the server, which uses its validation API for anti-cheat. Deterministic 100 Hz fixed-step simulation (ships, weapons, collisions) driven purely by arena settings; no rendering, no I/O, no floating-point nondeterminism in anything that must match the server/Continuum. Changes to it belong in oss-core: request them rather than working around them here, since a divergence between the client's physics and the server's would cause false anti-cheat findings.
- Rendering interpolates between sim ticks so 60–240+ Hz displays are smooth; pixel-art scaling is integer nearest by default.
- The network layer is a library usable headless: a `oss-bot`-style CLI drives it for tests and load swarms.
- Security paths are pluggable traits; Continuum's handshake is not implemented in this repo.

## Rules
- The client ships its default asset set in `assets/classic/` (graphics and sound). A user's own folder (`--assets-dir`/`OSS_ASSETS_DIR`) or a Continuum install (`--continuum-dir`) overrides it. Never commit `Continuum.exe` or its DLLs, third-party zone downloads or captures.
- Untrusted network/file data never panics the client.
- Tests use synthetic data; no captures, personal data or machine paths in the repo.
- Before committing: `cargo fmt` (**not** `cargo fmt --all`), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
  `cargo fmt --all` takes its package list from `cargo metadata`, which includes path
  dependencies outside this workspace. In this repo it therefore hands rustfmt 30 source
  roots in the sibling `../oss-core` checkout — every crate there, including
  `oss-conformance` and `oss-bot-sdk`, which this repo does not depend on — and
  **rewrites oss-core in place**, across a repo boundary and without saying so. Plain
  `cargo fmt` at the workspace root covers exactly this repo's four crates, which is what
  is wanted. Verified 2026-11-11 with `cargo fmt --all --verbose --check`.
