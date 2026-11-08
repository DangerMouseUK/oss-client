//! The network side of the client: a session to a zone, its socket and map cache.

use oss_session::map::{MapCache, default_cache_dir};
use oss_session::udp::UdpDriver;
use oss_session::{Session, SessionConfig, SessionEvent, State};

/// How to reach a zone and who to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerOptions {
    pub server: String,
    pub name: String,
    pub password: String,
    pub arena: Option<String>,
    /// 0..=7, or 8 for spectator.
    pub ship: u8,
    /// Team to join after entering, if not the server's choice.
    pub freq: Option<i16>,
}

pub struct Online {
    pub session: Session,
    driver: UdpDriver,
    cache: MapCache,
    /// The arena's map once loaded: (name, `.lvl` bytes).
    pub map: Option<(String, Vec<u8>)>,
    /// Latest seeds from the server: (green, door, server tick they date from).
    pub seeds: Option<(u32, u32, u32)>,
    /// Human-readable progress while connecting.
    pub status: String,
    /// Set when the session failed or closed.
    pub error: Option<String>,
    /// Team requested on the command line.
    pub freq: Option<i16>,
    /// LVZ files received and not yet loaded by the game: (name, bytes).
    pub files: Vec<(String, Vec<u8>)>,
}

impl Online {
    pub fn connect(opts: &ServerOptions, resolution: (u16, u16)) -> Result<Self, String> {
        let driver =
            UdpDriver::connect(&opts.server).map_err(|e| format!("{}: {e}", opts.server))?;
        let cfg = SessionConfig {
            name: opts.name.clone(),
            password: opts.password.clone(),
            arena: opts.arena.clone(),
            ship: opts.ship,
            resolution,
            want_lvz: true,
            ..Default::default()
        };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
            .unwrap_or(1);
        let session = Session::new(cfg, driver.now_ms(), seed);
        Ok(Self {
            session,
            driver,
            cache: MapCache::new(default_cache_dir(&opts.server)),
            map: None,
            seeds: None,
            status: format!("Connecting to {}...", opts.server),
            error: None,
            freq: opts.freq,
            files: Vec::new(),
        })
    }

    pub fn now_ms(&self) -> u64 {
        self.driver.now_ms()
    }

    /// Sends and receives, handles the map and connection progress itself and
    /// returns the remaining events for the game.
    pub fn pump(&mut self) -> Vec<SessionEvent> {
        if let Err(e) = self.driver.pump(&mut self.session) {
            self.error = Some(format!("network error: {e}"));
            self.session.disconnect();
        }
        let mut out = Vec::new();
        while let Some(ev) = self.session.poll_event() {
            match &ev {
                SessionEvent::Connected => self.status = "Logging in...".into(),
                SessionEvent::LoggedIn => self.status = "Entering arena...".into(),
                SessionEvent::LoginFailed { message, .. } => {
                    self.error = Some(format!("Login failed: {message}"))
                }
                SessionEvent::MapNeeded { name, .. } => {
                    if self
                        .cache
                        .load(name)
                        .is_some_and(|d| self.session.use_cached_map(&d))
                    {
                        log::info!("map {name} from cache");
                    } else {
                        self.status = format!("Downloading {name}...");
                        self.session.download_map();
                    }
                }
                SessionEvent::MapProgress { received, total } if *total > 0 => {
                    self.status = format!("Downloading... {}%", received * 100 / total);
                }
                SessionEvent::MapReady {
                    name,
                    data,
                    downloaded,
                } => {
                    if *downloaded && let Err(e) = self.cache.store(name, data) {
                        log::warn!("couldn't cache map {name}: {e}");
                    }
                    self.map = Some((name.clone(), data.clone()));
                }
                SessionEvent::Seeds {
                    green_seed,
                    door_seed,
                    timestamp,
                } => self.seeds = Some((*green_seed, *door_seed, *timestamp)),
                SessionEvent::FilesListed(files) => {
                    for f in files {
                        let cached = self.cache.load(&f.name);
                        if cached.is_some_and(|d| self.session.use_cached_file(&f.name, &d)) {
                            log::info!("{} from cache", f.name);
                        } else {
                            self.session.download_file(&f.name);
                        }
                    }
                }
                SessionEvent::FileReady {
                    name,
                    data,
                    downloaded,
                } => {
                    if *downloaded && let Err(e) = self.cache.store(name, data) {
                        log::warn!("couldn't cache {name}: {e}");
                    }
                    self.files.push((name.clone(), data.clone()));
                }
                SessionEvent::Closed(reason) if self.error.is_none() => {
                    self.error = Some(format!("Disconnected ({reason:?})"));
                }
                _ => {}
            }
            out.push(ev);
        }
        let _ = self.driver.send_pending(&mut self.session);
        out
    }

    /// Whether the arena is ready to play (entered, settings and map loaded).
    /// The arena is entered and every file is in. Downloads must finish before you
    /// take a ship: ASSS-style servers put anyone who requests a file into spectator
    /// mode (Continuum downloads everything before it plays).
    pub fn ready(&self) -> bool {
        self.session.state() == State::Playing
            && self.map.is_some()
            && self.session.settings().is_some()
            && !self.session.downloads_pending()
    }

    pub fn flush(&mut self) {
        let now = self.now_ms();
        self.session.flush(now);
        let _ = self.driver.send_pending(&mut self.session);
    }
}
