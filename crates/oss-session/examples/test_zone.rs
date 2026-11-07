//! Runs the minimal relay zone for local client/bot testing:
//!
//! ```text
//! cargo run -p oss-session --example test_zone -- [--bind 127.0.0.1:5000] <map.lvl>
//! ```
//!
//! Serves the map with stock settings and relays positions and chat. Not a real
//! server: use `oss-server` for anything beyond smoke tests.

fn main() {
    env_logger_init();
    let mut bind = "127.0.0.1:5000".to_string();
    let mut map = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bind" => bind = args.next().unwrap_or(bind),
            _ => map = Some(std::path::PathBuf::from(a)),
        }
    }
    let Some(map) = map else {
        eprintln!("usage: test_zone [--bind host:port] <map.lvl>");
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
