//! Open SubSpace game client.
//!
//! Everything is in the library beside this file; a build outside this repository
//! starts the same client the same way, with a handshake and a client profile of
//! its own (see the `oss_client` crate documentation).

fn main() {
    oss_client::init_logging();
    match oss_client::options(std::env::args().skip(1)) {
        Ok(None) => println!(
            "oss-client {}\n\n{}",
            env!("CARGO_PKG_VERSION"),
            oss_client::USAGE
        ),
        Ok(Some(opts)) => {
            if let Err(e) = oss_client::run(opts) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{}", oss_client::USAGE);
            std::process::exit(2);
        }
    }
}
