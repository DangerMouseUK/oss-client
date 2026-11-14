//! Open SubSpace game client: the graphical client as a library, so a binary
//! outside this repository can start it.
//!
//! The binary in this crate is the whole of this library's own use of it:
//!
//! ```no_run
//! oss_client::init_logging();
//! let Some(options) = oss_client::options(std::env::args().skip(1))? else {
//!     println!("{}", oss_client::USAGE);
//!     return Ok(());
//! };
//! oss_client::run(options)?;
//! # Ok::<(), String>(())
//! ```
//!
//! # Supplying a client this repository does not have
//!
//! [`Options::supplied`] is the seam. Set it between [`options`] and [`run`] and
//! every zone the client joins is joined with the handshake you supply, as the
//! client you supply:
//!
//! ```no_run
//! use std::sync::Arc;
//! use oss_client::{HandshakeRequest, Supplied};
//! use oss_net::Handshake;
//! use oss_session::security::SecurityProfile;
//! use oss_session::{ClientProfile, LoginForm, SecurityAnswer, SecurityChallenge};
//!
//! #[derive(Debug)]
//! struct MyBuild;
//!
//! impl SecurityProfile for MyBuild {
//!     fn handshake(&self, request: HandshakeRequest) -> Box<dyn Handshake> {
//!         # let _ = request;
//!         // Whatever this build connects with.
//!         # unimplemented!()
//!     }
//! }
//!
//! impl ClientProfile for MyBuild {
//!     fn login(&self) -> LoginForm {
//!         # unimplemented!()
//!     }
//!     fn security(&self, challenge: SecurityChallenge) -> SecurityAnswer {
//!         # let _ = challenge;
//!         # unimplemented!()
//!     }
//! }
//!
//! let mut options = oss_client::options(std::env::args().skip(1))?.unwrap();
//! options.supplied = Supplied {
//!     security: Some(Arc::new(MyBuild)),
//!     client: Some(Arc::new(MyBuild)),
//! };
//! oss_client::run(options)?;
//! # Ok::<(), String>(())
//! ```
//!
//! **Both halves or neither.** A zone picks which login form it expects from the
//! encryption rather than from the packet's own type byte, so a connection given
//! a handshake without the client to go with it is dropped at login with nothing
//! on the wire to say why. [`oss_session::client`] has that in full.
//!
//! Nothing in this repository implements either half for a client that is not
//! here, and nothing here knows what one would return: the handshake is an
//! `oss_net::Handshake` like any other and this crate only hands it to the
//! session. [`oss_session::security::SecurityProfile`] is the seam itself and
//! [`oss_session::ClientProfile`] the matching one for the login packets.

pub mod app;
pub mod cli;

mod audio;
mod config;
mod game;
mod gfx;

pub use app::{Mode, Options, run};
pub use cli::{USAGE, init_logging, options};
pub use game::input::{Binding, Bindings, PadBindings};
pub use game::online::{ServerOptions, Supplied};
pub use game::palette::PaletteKind;
pub use oss_session::security::{HandshakeRequest, Security, SecurityProfile, built_in};
pub use oss_session::{ClientProfile, Profile};
