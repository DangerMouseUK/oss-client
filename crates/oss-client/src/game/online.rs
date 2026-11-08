//! The network side of the client: a session to a zone, its socket and map cache;
//! or a replay file played back through the same session code.

use std::path::{Path, PathBuf};
use std::time::Instant;

use oss_session::map::{MapCache, default_cache_dir};
use oss_session::replay::{Player as ReplayPlayer, Recorder};
use oss_session::udp::UdpDriver;
use oss_session::{Session, SessionConfig, SessionEvent, State};

/// Where packets come from.
enum Source {
    Live(UdpDriver),
    Replay {
        player: Box<ReplayPlayer>,
        start: Instant,
        /// Said "end of replay" already.
        ended: bool,
    },
}

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
    /// Spectate and follow this player (name) once they're in a ship.
    pub follow: Option<String>,
    /// Attach to this player (name) as a turret once you're both in ships.
    pub attach: Option<String>,
    /// Record the visit to this replay file.
    pub record: Option<PathBuf>,
}

pub struct Online {
    pub session: Session,
    source: Source,
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
    /// Player to follow (`--follow`), until found.
    pub follow: Option<String>,
    /// Player to attach to (`--attach`), until done.
    pub attach: Option<String>,
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
        let now = driver.now_ms();
        let mut session = Session::new(cfg, now, seed);
        if let Some(path) = &opts.record {
            let started = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
                .to_string();
            let meta = [
                ("zone", opts.server.as_str()),
                ("player", opts.name.as_str()),
                ("arena", opts.arena.as_deref().unwrap_or("")),
                ("client", concat!("oss-client ", env!("CARGO_PKG_VERSION"))),
                ("started", started.as_str()),
            ];
            let rec = Recorder::create(path, now, &meta)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            session.record_to(rec, now);
            log::info!("recording to {}", path.display());
        }
        Ok(Self {
            session,
            source: Source::Live(driver),
            cache: MapCache::new(default_cache_dir(&opts.server)),
            map: None,
            seeds: None,
            status: format!("Connecting to {}...", opts.server),
            error: None,
            freq: opts.freq,
            follow: opts.follow.clone(),
            attach: opts.attach.clone(),
            files: Vec::new(),
        })
    }

    /// Plays a replay file back: the session is fed from the recording, and the game
    /// spectates, following the player who recorded it.
    pub fn open_replay(path: &Path) -> Result<Self, String> {
        let player = ReplayPlayer::open(path)?;
        let recorder = player.replay.meta_value("player").map(str::to_string);
        let session = Session::playback(SessionConfig::default(), 0);
        let secs = player.duration_ms() / 1000;
        Ok(Self {
            session,
            source: Source::Replay {
                player: Box::new(player),
                start: Instant::now(),
                ended: false,
            },
            cache: MapCache::new(std::env::temp_dir()),
            map: None,
            seeds: None,
            status: format!(
                "Playing {} ({}:{:02})...",
                path.display(),
                secs / 60,
                secs % 60
            ),
            error: None,
            freq: None,
            follow: recorder,
            attach: None,
            files: Vec::new(),
        })
    }

    pub fn is_replay(&self) -> bool {
        matches!(self.source, Source::Replay { .. })
    }

    /// Playing back and every record has been shown (true once).
    pub fn replay_just_ended(&mut self) -> bool {
        match &mut self.source {
            Source::Replay { player, ended, .. } if player.finished() && !*ended => {
                *ended = true;
                true
            }
            _ => false,
        }
    }

    pub fn now_ms(&self) -> u64 {
        match &self.source {
            Source::Live(d) => d.now_ms(),
            Source::Replay { start, .. } => start.elapsed().as_millis() as u64,
        }
    }

    /// A file for the session: from the replay when playing back, else the cache.
    fn stored_file(&self, name: &str) -> Option<Vec<u8>> {
        match &self.source {
            Source::Replay { player, .. } => player.file(name).map(<[u8]>::to_vec),
            Source::Live(_) => self.cache.load(name),
        }
    }

    /// Sends and receives, handles the map and connection progress itself and
    /// returns the remaining events for the game.
    pub fn pump(&mut self) -> Vec<SessionEvent> {
        let now = self.now_ms();
        match &mut self.source {
            Source::Live(driver) => {
                if let Err(e) = driver.pump(&mut self.session) {
                    self.error = Some(format!("network error: {e}"));
                    self.session.disconnect();
                }
            }
            Source::Replay { player, .. } => {
                let t = now.min(u32::MAX as u64) as u32;
                let due: Vec<_> = player.due(t).to_vec();
                for r in &due {
                    self.session.feed(now, r);
                }
            }
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
                        .stored_file(name)
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
                    if *downloaded
                        && !self.is_replay()
                        && let Err(e) = self.cache.store(name, data)
                    {
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
                        let cached = self.stored_file(&f.name);
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
                    if *downloaded
                        && !self.is_replay()
                        && let Err(e) = self.cache.store(name, data)
                    {
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
        if let Source::Live(d) = &mut self.source {
            let _ = d.send_pending(&mut self.session);
        }
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
        if let Source::Live(d) = &mut self.source {
            let _ = d.send_pending(&mut self.session);
        }
    }
}
