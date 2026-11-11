//! `oss-bot`: a headless SubSpace client for tests and load swarms.
//!
//! Connects (VIE), logs in, enters an arena, takes the settings and map (cached),
//! answers security checks, and either sits in spectator mode or flies a simulated ship
//! around, sending positions (and optionally firing). Many bots can run in one process.

use oss_session::security::{Identities, SecurityMode, client_handshake, fingerprint};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use oss_session::map::{MapCache, default_cache_dir};
use oss_session::sim::{arena_world, own_position, weapon_for};
use oss_session::udp::UdpDriver;
use oss_session::{SPECTATOR, Session, SessionConfig, SessionEvent, State, chat_type};
use oss_sim::fixed::Rng;
use oss_sim::{ShipInput, World};

const USAGE: &str = "\
Usage: oss-bot --server <host:port> [options]

Options:
  --server <host:port>   Zone to connect to (required)
  --name <name>          Login name (default oss-bot; numbered when --count > 1)
  --password <pw>        Password (default empty)
  --arena <name>         Arena to enter (default: any public arena)
  --ship <1-8|spec>      Ship to fly (default spec)
  --count <n>            Run n bots (default 1)
  --fly                  Fly around (needs a ship): thrust, turn and shoot at random
  --chat <text>          Say this in public chat once in the arena; :name:text
                         sends it privately to that player once they're here
  --new-user             Register the name instead of expecting it to exist.
                         Subgame forgets its accounts when the zone restarts, so a
                         bot needs this against a zone that has just come up
  --duration <secs>      Disconnect after this long (default: run until Ctrl+C)
  --map-cache <dir>      Where downloaded maps are kept
  --security <mode>      vie (default), opensec, or auto (OpenSec, falling back
                         to VIE); bots don't pin server keys
  --identity <file>      Offer an OpenSec identity key, one per bot number and
                         zone, kept in <file> (made if missing); a renamed bot
                         keeps its key
  -v, --verbose          Log every bot's events (default: bot 1 only)
  -h, --help             Show this help";

struct Options {
    server: String,
    name: String,
    password: String,
    arena: Option<String>,
    ship: u8,
    count: usize,
    fly: bool,
    chat: Option<String>,
    duration: Option<Duration>,
    map_cache: Option<PathBuf>,
    verbose: bool,
    security: SecurityMode,
    identity: Option<PathBuf>,
    new_user: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut o = Options {
        server: String::new(),
        name: "oss-bot".into(),
        password: String::new(),
        arena: None,
        ship: SPECTATOR,
        count: 1,
        fly: false,
        chat: None,
        duration: None,
        map_cache: None,
        verbose: false,
        security: SecurityMode::Vie,
        identity: None,
        new_user: false,
    };
    while let Some(a) = args.next() {
        let mut value = |n: &str| args.next().ok_or_else(|| format!("{n} needs a value"));
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--server" => o.server = value("--server")?,
            "--name" => o.name = value("--name")?,
            "--password" => o.password = value("--password")?,
            "--arena" => o.arena = Some(value("--arena")?),
            "--new-user" => o.new_user = true,
            "--ship" => {
                let v = value("--ship")?;
                o.ship = if v.eq_ignore_ascii_case("spec") {
                    SPECTATOR
                } else {
                    v.parse::<u8>()
                        .ok()
                        .filter(|s| (1..=8).contains(s))
                        .ok_or("--ship must be 1-8 or spec")?
                        - 1
                }
            }
            "--count" => {
                o.count = value("--count")?
                    .parse()
                    .ok()
                    .filter(|&n| (1..=1000).contains(&n))
                    .ok_or("--count must be 1-1000")?
            }
            "--fly" => o.fly = true,
            "--security" => {
                o.security = SecurityMode::from_name(&value("--security")?)
                    .ok_or("--security must be vie, opensec or auto")?
            }
            "--identity" => o.identity = Some(value("--identity")?.into()),
            "--chat" => o.chat = Some(value("--chat")?),
            "--duration" => {
                let s: f64 = value("--duration")?
                    .parse()
                    .map_err(|_| "--duration must be seconds")?;
                o.duration = Some(Duration::from_secs_f64(s.max(0.0)));
            }
            "--map-cache" => o.map_cache = Some(PathBuf::from(value("--map-cache")?)),
            "-v" | "--verbose" => o.verbose = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if o.server.is_empty() {
        return Err("--server is required".into());
    }
    Ok(Some(o))
}

/// One bot: a session, its socket, and (when flying) a simulation of its ship.
struct Bot {
    label: String,
    session: Session,
    driver: UdpDriver,
    loud: bool,
    map: Option<Vec<u8>>,
    green_seed: u32,
    world: Option<World>,
    rng: Rng,
    chatted: bool,
    last_step_ms: u64,
}

impl Bot {
    fn log(&self, msg: impl std::fmt::Display) {
        if self.loud {
            log::info!("[{}] {msg}", self.label);
        }
    }

    fn handle_events(&mut self, opts: &Options, cache: &MapCache) {
        while let Some(ev) = self.session.poll_event() {
            match ev {
                SessionEvent::Connected => {
                    let cipher = self.session.cipher_name().unwrap_or("?");
                    self.log(format!("connected ({cipher})"));
                }
                SessionEvent::LoginFailed { message, .. } => {
                    log::warn!("[{}] login failed: {message}", self.label)
                }
                SessionEvent::LoggedIn => self.log("logged in"),
                SessionEvent::EnteredArena { pid } => {
                    self.log(format_args!("entering arena as pid {pid}"))
                }
                SessionEvent::MapNeeded { name, checksum } => {
                    if cache
                        .load(&name)
                        .is_some_and(|d| self.session.use_cached_map(&d))
                    {
                        self.log(format_args!("map {name} from cache"));
                    } else {
                        self.log(format_args!("downloading map {name} (crc {checksum:08x})"));
                        self.session.download_map();
                    }
                }
                SessionEvent::MapReady {
                    name,
                    data,
                    downloaded,
                } => {
                    if downloaded && let Err(e) = cache.store(&name, &data) {
                        log::warn!("[{}] couldn't cache map {name}: {e}", self.label);
                    }
                    self.log(format_args!("map {name} ready ({} bytes)", data.len()));
                    self.map = Some(data);
                }
                SessionEvent::Seeds { green_seed, .. } => self.green_seed = green_seed,
                SessionEvent::Playing => {
                    let n = self.session.players().count();
                    self.log(format_args!("in the arena with {n} player(s)"));
                    // Servers start you as a spectator; ask for the ship.
                    let current = self.session.me().map(|p| p.ship).unwrap_or(SPECTATOR);
                    if opts.ship != SPECTATOR && current != opts.ship {
                        self.session.set_ship(opts.ship);
                    } else if opts.fly && opts.ship != SPECTATOR {
                        self.start_flying(opts.ship);
                    }
                }
                SessionEvent::PlayerEntered(pid) => {
                    if let Some(p) = self.session.player(pid) {
                        self.log(format_args!(
                            "{} entered (ship {}, freq {})",
                            p.name,
                            p.ship + 1,
                            p.freq
                        ));
                    }
                }
                SessionEvent::PlayerLeft(pid) => self.log(format_args!("pid {pid} left")),
                SessionEvent::ShipChanged { pid, ship, freq }
                    if Some(pid) == self.session.pid() =>
                {
                    self.log(format_args!("now in ship {} on freq {freq}", ship + 1));
                    if opts.fly && ship != SPECTATOR && self.world.is_none() {
                        self.start_flying(ship);
                    }
                }
                SessionEvent::Chat {
                    kind, name, text, ..
                } => {
                    let who = name.unwrap_or_default();
                    self.log(format_args!("chat({kind}) {who}> {text}"));
                }
                SessionEvent::Kill { killer, killed, .. } => {
                    self.log(format_args!("{killer} killed {killed}"))
                }
                SessionEvent::ShipReset => {
                    if let Some(w) = self.world.as_mut()
                        && !w.ships.is_empty()
                    {
                        w.respawn(0);
                    }
                }
                SessionEvent::Closed(reason) => {
                    log::warn!("[{}] disconnected: {reason:?}", self.label)
                }
                _ => {}
            }
        }
    }

    fn start_flying(&mut self, ship: u8) {
        let (Some(map), Some(settings)) = (self.map.as_ref(), self.session.settings()) else {
            return;
        };
        let (mut world, _) = arena_world(map, settings, self.green_seed);
        world.prize_seed = self.green_seed;
        let pid = self.session.pid().unwrap_or(0) as u16;
        let freq = self.session.me().map(|p| p.freq).unwrap_or(0) as u16;
        world.add_ship(pid, ship, freq);
        self.world = Some(world);
    }

    /// Advances the simulated ship to `now` and sends positions/weapons.
    fn fly(&mut self, now: u64) {
        let Some(world) = self.world.as_mut() else {
            return;
        };
        let mut fired = None;
        while self.last_step_ms + 10 <= now {
            self.last_step_ms += 10;
            // Random walk: mostly thrust, turn in bursts, shoot now and then.
            let r = self.rng.next_u32();
            let t = world.tick;
            let input = ShipInput {
                forward: t % 400 < 300,
                left: (t / 150) % 3 == 0,
                right: (t / 150) % 3 == 1,
                gun: r.is_multiple_of(40),
                bomb: r % 600 == 1,
                afterburner: t % 1000 < 100,
                ..Default::default()
            };
            for ev in world.step(&[input]) {
                if fired.is_none() {
                    fired = weapon_for(&ev, &world.ships[0]);
                }
            }
        }
        let ship = &world.ships[0];
        if fired.is_some() || self.session.position_due(now) {
            let pos = own_position(ship, fired);
            self.session.send_position(now, &pos);
        }
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let opts = match parse_args(std::env::args().skip(1)) {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("oss-bot {}\n\n{USAGE}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let cache = MapCache::new(
        opts.map_cache
            .clone()
            .unwrap_or_else(|| default_cache_dir(&opts.server)),
    );

    let mut bots = Vec::new();
    for i in 0..opts.count {
        let name = if opts.count > 1 {
            format!("{}{}", opts.name, i + 1)
        } else {
            opts.name.clone()
        };
        let driver = match UdpDriver::connect(&opts.server) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error: {}: {e}", opts.server);
                std::process::exit(1);
            }
        };
        let cfg = SessionConfig {
            name: name.clone(),
            password: opts.password.clone(),
            arena: opts.arena.clone(),
            ship: opts.ship,
            machine_id: 0x0B07_0000 + i as u32,
            new_user: opts.new_user,
            ..Default::default()
        };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1)
            .wrapping_add(i as u32 * 7919);
        // One identity per bot number and zone, kept across runs and renames.
        let identity = opts.identity.as_deref().map(|path| {
            let id = Identities::load(path)
                .for_server(&format!("bot{}@{}", i + 1, opts.server))
                .unwrap_or_else(|e| {
                    eprintln!("error: {}: {e}", path.display());
                    std::process::exit(1);
                });
            if i == 0 || opts.verbose {
                println!("[{name}] identity {}", fingerprint(&id.public()));
            }
            id
        });
        let handshake = client_handshake(opts.security, None, identity, seed);
        let session = Session::with_handshake(cfg, driver.now_ms(), handshake);
        bots.push(Bot {
            label: name,
            session,
            driver,
            loud: i == 0 || opts.verbose,
            map: None,
            green_seed: 0,
            world: None,
            rng: Rng::new(seed | 1),
            chatted: false,
            last_step_ms: 0,
        });
    }
    log::info!("{} bot(s) connecting to {}", bots.len(), opts.server);

    let start = Instant::now();
    loop {
        let mut open = 0;
        for bot in &mut bots {
            if bot.session.state() == State::Closed {
                continue;
            }
            open += 1;
            if let Err(e) = bot.driver.pump(&mut bot.session) {
                log::warn!("[{}] socket error: {e}", bot.label);
                bot.session.disconnect();
            }
            bot.handle_events(&opts, &cache);
            let now = bot.driver.now_ms();
            if bot.world.is_none() {
                bot.last_step_ms = now;
            }
            if bot.session.state() == State::Playing {
                if let Some(text) = opts.chat.as_ref().filter(|_| !bot.chatted) {
                    match private_chat(text) {
                        // :name:text waits until that player is in the arena.
                        Some((to, msg)) => {
                            let pid = bot
                                .session
                                .players()
                                .find(|p| p.name.eq_ignore_ascii_case(to))
                                .map(|p| p.pid);
                            if let Some(pid) = pid {
                                bot.session.send_chat(chat_type::PRIVATE, pid, msg);
                                bot.chatted = true;
                            }
                        }
                        None => {
                            bot.session.send_chat(chat_type::PUBLIC, 0, text);
                            bot.chatted = true;
                        }
                    }
                }
                if bot.world.is_some() {
                    bot.fly(now);
                } else if bot.session.position_due(now) {
                    // Spectators still report in so the server knows they're alive.
                    bot.session.send_position(now, &Default::default());
                }
            }
            let _ = bot.driver.send_pending(&mut bot.session);
        }
        if open == 0 {
            log::info!("all bots disconnected");
            break;
        }
        if opts.duration.is_some_and(|d| start.elapsed() >= d) {
            for bot in &mut bots {
                bot.session.disconnect();
                let _ = bot.driver.send_pending(&mut bot.session);
            }
            log::info!("time is up; disconnected");
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `:name:text` → (name, text), Continuum's private-message syntax.
fn private_chat(text: &str) -> Option<(&str, &str)> {
    let (to, msg) = text.strip_prefix(':')?.split_once(':')?;
    (!to.is_empty()).then_some((to, msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Option<Options>, String> {
        parse_args(s.split_whitespace().map(String::from))
    }

    #[test]
    fn parses_options() {
        let o = parse("--server 127.0.0.1:5000 --ship 3 --count 20 --fly --duration 1.5")
            .unwrap()
            .unwrap();
        assert_eq!((o.ship, o.count, o.fly), (2, 20, true));
        assert_eq!(o.duration, Some(Duration::from_millis(1500)));
        assert_eq!(
            parse("--server x:1 --ship spec").unwrap().unwrap().ship,
            SPECTATOR
        );
        assert!(parse("--ship 2").is_err());
        assert!(parse("--server x:1 --ship 9").is_err());
        assert!(parse("--help").unwrap().is_none());
    }

    #[test]
    fn private_chat_syntax() {
        assert_eq!(private_chat(":Bob:?ban -t 5"), Some(("Bob", "?ban -t 5")));
        assert_eq!(private_chat("hello :there:"), None);
        assert_eq!(private_chat("::x"), None);
        let o = parse("--server x:1 --identity ids.txt").unwrap().unwrap();
        assert_eq!(o.identity, Some(PathBuf::from("ids.txt")));
    }
}
