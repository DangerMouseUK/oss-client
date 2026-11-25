//! Open SubSpace game client: the graphical client as a library, so a binary
//! outside this repository can start it.
//!
//! The binary in this crate is the whole of this library's own use of it:
//!
//! ```no_run
//! oss_client::init_logging();
//! let Some(options) = oss_client::options(std::env::args().skip(1))? else {
//!     println!("{}", oss_client::usage(&oss_client::Supplied::default().mechanisms));
//!     return Ok(());
//! };
//! oss_client::run(options)?;
//! # Ok::<(), String>(())
//! ```
//!
//! # Adding mechanisms and a client this repository does not have
//!
//! [`Supplied`] is the seam. Its mechanisms start as the built-in ones; a build
//! registers its own beside them, one line each, and hands the result to
//! [`options_with`] **before the arguments are read**, so that `--security`, the
//! settings file, [`usage`] and the start-up lines all know them:
//!
//! ```no_run
//! use std::sync::Arc;
//! use oss_client::Supplied;
//! use oss_session::security::Mechanism;
//! use oss_session::{ClientProfile, LoginForm, SecurityAnswer, SecurityChallenge};
//!
//! # fn my_mechanism() -> Mechanism { unimplemented!() }
//! #[derive(Debug)]
//! struct MyClient;
//!
//! impl ClientProfile for MyClient {
//!     fn login(&self) -> LoginForm {
//!         # unimplemented!()
//!     }
//!     fn security(&self, challenge: SecurityChallenge) -> SecurityAnswer {
//!         # let _ = challenge;
//!         # unimplemented!()
//!     }
//! }
//!
//! let mut supplied = Supplied {
//!     client: Some(Arc::new(MyClient)),
//!     ..Supplied::default()
//! };
//! supplied.mechanisms.register(my_mechanism()).map_err(|e| e.to_string())?;
//! let Some(options) = oss_client::options_with(std::env::args().skip(1), supplied)? else {
//!     return Ok(());
//! };
//! oss_client::run(options)?;
//! # Ok::<(), String>(())
//! ```
//!
//! **A mechanism is added, never swapped in.** The built-ins stay, the automatic
//! order is read from the registry by strength, and a mechanism named with
//! `--security` (or in the settings file) is that mechanism alone. A mechanism
//! declares its era, and the session checks the login form the client sends
//! against it: a zone picks which form it expects from the encryption rather than
//! from the packet's own type byte, and drops a mismatch with nothing on the wire
//! to say why. [`oss_session::client`] has that in full.
//!
//! Nothing in this repository implements a mechanism or a client that is not
//! here, and nothing here knows what one would do: a mechanism's handshake is an
//! `oss_net::Handshake` like any other. [`oss_session::security`] reads the
//! registry and [`oss_session::ClientProfile`] is the matching seam for the login
//! packets.

pub mod app;
pub mod cli;

mod audio;
mod config;
mod game;
mod gfx;

pub use app::{Mode, Options, run, startup_lines};
pub use cli::{init_logging, options, options_with, usage};
pub use game::input::{Binding, Bindings, PadBindings};
pub use game::online::{ServerOptions, Supplied};
pub use game::palette::PaletteKind;
pub use oss_session::security::{Mechanism, MechanismRegistry, SecurityMode};
pub use oss_session::{ClientProfile, Profile};
