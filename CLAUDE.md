# oss-client — agent conventions

Rust game client for SubSpace/Continuum. Licence **GPL-2.0-or-later**; `oss-sim` and `oss-assets` are **MIT** (reusable by bots/tools, may later move to oss-core). Behavioural references: nullspace (MIT, C++ client), mervbot (bot core), ASSS (server-side truth for physics/settings semantics).

## Architecture
- Protocol, transport and formats come from the sibling `../oss-core` crates (`oss-proto`, `oss-net`, `oss-formats`). Don't fork them here; request changes there.
- Stack: `winit` (window/input), `wgpu` (rendering; keep it WebGPU-compatible for the later browser build), `gilrs` (gamepads), a cpal-based audio crate, `egui` for menus/tools. In-game HUD is a custom batched sprite renderer that defaults to Continuum's layout.
- `oss-sim`: deterministic 100 Hz fixed-step simulation (ships, weapons, collisions) driven purely by arena settings; no rendering, no I/O, no floating-point nondeterminism in anything that must match the server/Continuum.
- Rendering interpolates between sim ticks so 60–240+ Hz displays are smooth; pixel-art scaling is integer nearest by default.
- The network layer is a library usable headless: a `oss-bot`-style CLI drives it for tests and load swarms.
- Security paths are pluggable traits; Continuum's handshake is not implemented in this repo.

## Rules
- The client ships its default asset set in `assets/classic/` (graphics and sound). A user's own folder (`--assets-dir`/`OSS_ASSETS_DIR`) or a Continuum install (`--continuum-dir`) overrides it. Never commit `Continuum.exe` or its DLLs, third-party zone downloads or captures.
- Untrusted network/file data never panics the client.
- Tests use synthetic data; no captures, personal data or machine paths in the repo.
- Before committing: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
