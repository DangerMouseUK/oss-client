//! Runs the minimal relay zone for local client/bot testing:
//!
//! ```text
//! cargo run -p oss-session --example test_zone -- [--bind 127.0.0.1:5000] [--ask-registration] [--redirect host:port[:arena]] [--keep-first-handshake] [--lenient-requests] <map.lvl>
//! ```
//!
//! Serves the map with stock settings and relays positions and chat. Not a real
//! server: use `oss-server` for anything beyond smoke tests.
//!
//! `--ask-registration` asks every player for the registration form, as a zone
//! whose billing server wants one does, and logs the real name and e-mail of
//! each form that arrives.
//!
//! `--redirect` answers every arena request with a redirect (S2C `0x3B`) to
//! that zone, and that arena there when one is given, as a zone that only
//! forwards players elsewhere does. Run a second test zone at that address to
//! see a client follow it.
//!
//! `--keep-first-handshake` behaves as a zone that keeps a connection's first
//! handshake until it is closed: a second connection request from the same
//! address is answered, but the connection keeps what it was opened with.
//!
//! `--lenient-requests` reads any connection request by its first eight bytes,
//! whatever its length, as a lenient zone does: it can take another
//! mechanism's opening for its own request and answer it.

fn main() {
    env_logger_init();
    let mut bind = "127.0.0.1:5000".to_string();
    let mut map = None;
    let mut ask_registration = false;
    let mut redirect = None;
    let mut keep_first_handshake = false;
    let mut lenient_requests = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bind" => bind = args.next().unwrap_or(bind),
            "--ask-registration" => ask_registration = true,
            "--keep-first-handshake" => keep_first_handshake = true,
            "--lenient-requests" => lenient_requests = true,
            "--redirect" => {
                let value = args.next().unwrap_or_default();
                let (addr, arena) = match value.splitn(3, ':').collect::<Vec<_>>()[..] {
                    [host, port] => (format!("{host}:{port}"), None),
                    [host, port, arena] => (format!("{host}:{port}"), Some(arena.to_string())),
                    _ => (value.clone(), None),
                };
                match addr.parse::<std::net::SocketAddrV4>() {
                    Ok(a) => redirect = Some((a, arena)),
                    Err(e) => {
                        eprintln!("--redirect {value}: {e}");
                        std::process::exit(2);
                    }
                }
            }
            _ => map = Some(std::path::PathBuf::from(a)),
        }
    }
    let Some(map) = map else {
        eprintln!(
            "usage: test_zone [--bind host:port] [--ask-registration] [--redirect host:port[:arena]] [--keep-first-handshake] [--lenient-requests] <map.lvl>"
        );
        std::process::exit(2);
    };
    let data = match std::fs::read(&map) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}: {e}", map.display());
            std::process::exit(1);
        }
    };
    let name = map
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("map.lvl")
        .to_string();
    let settings = oss_sim::ArenaSettings::default().to_client_settings();
    let mut zone = match oss_session::test_zone::TestZone::bind(&bind, &name, &data, &settings) {
        Ok(z) => z,
        Err(e) => {
            eprintln!("bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    zone.ask_for_registration(ask_registration);
    zone.redirect_to(redirect);
    zone.keep_first_handshake(keep_first_handshake);
    zone.lenient_requests(lenient_requests);
    eprintln!(
        "test zone on {} serving {name}",
        zone.local_addr().map(|a| a.to_string()).unwrap_or(bind)
    );
    loop {
        zone.pump();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn env_logger_init() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Info
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("{} {}", r.level(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let _ = log::set_logger(&Stderr).map(|()| log::set_max_level(log::LevelFilter::Info));
}
