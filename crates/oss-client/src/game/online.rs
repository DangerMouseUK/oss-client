//! The network side of the client: a session to a zone, its socket and its cache of
//! maps, LVZ files and the zone's news; or a replay file played back through the
//! same session code.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use oss_session::map::{MapCache, default_cache_dir};
use oss_session::replay::{Player as ReplayPlayer, Recorder};
use oss_session::security::{
    Identities, KnownServers, SecurityMode, client_handshake, fingerprint,
};
use oss_session::udp::UdpDriver;
use oss_session::{Session, SessionConfig, SessionEvent, State};

/// The zone's news, cached beside its maps under Continuum's name for it.
const NEWS_FILE: &str = "news.txt";

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
///
/// `Debug` is written out rather than derived so the password cannot be printed.
#[derive(Clone, PartialEq, Eq)]
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
    /// Encryption: OpenSec first with VIE fallback (auto), or one of them.
    pub security: SecurityMode,
    /// Where OpenSec server keys are remembered (trust on first use).
    pub known_servers: Option<PathBuf>,
    /// Your OpenSec identity keys, one per zone; `None` offers no identity.
    pub identities: Option<PathBuf>,
    /// Register the name on login instead of expecting the zone to know it
    /// ([`SessionConfig::new_user`]).
    pub new_user: bool,
}

/// Everything but the password, which prints as `<redacted>`. Redacted at the
/// field rather than on the type: the rest is what anyone debugging a connection
/// came for.
impl fmt::Debug for ServerOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerOptions")
            .field("server", &self.server)
            .field("name", &self.name)
            .field("password", &oss_proto::wire::Redacted)
            .field("arena", &self.arena)
            .field("ship", &self.ship)
            .field("freq", &self.freq)
            .field("follow", &self.follow)
            .field("attach", &self.attach)
            .field("record", &self.record)
            .field("security", &self.security)
            .field("known_servers", &self.known_servers)
            .field("identities", &self.identities)
            .field("new_user", &self.new_user)
            .finish()
    }
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
    /// Transfers started and finished for this arena, so the progress line can say
    /// where it is in the set. A zone sends a map and anything up to a dozen LVZ
    /// files, and a bare percentage that restarts at each one reads as a fault.
    transfers_started: usize,
    transfers_done: usize,
    /// Chat that arrived before there was a game screen to show it in.
    ///
    /// While the client is still connecting, `pump()` is driven by the connecting
    /// screen, which has nowhere to put chat and drops what `pump()` returns. The
    /// zone has already said things by then -- a login that answers
    /// `NO_SCORES` reports it there -- so those lines are kept here and the game
    /// takes them with [`Online::take_early_chat`] when it starts.
    early_chat: Vec<SessionEvent>,
    /// OpenSec keys on record, and whether this server's was already known.
    known: KnownServers,
    pinned: bool,
    server: String,
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
            new_user: opts.new_user,
            ..Default::default()
        };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
            .unwrap_or(1);
        let now = driver.now_ms();
        let known = opts
            .known_servers
            .as_deref()
            .map(KnownServers::load)
            .unwrap_or_default();
        let pin = known.get(&opts.server);
        let identity = opts.identities.as_deref().and_then(|path| {
            match Identities::load(path).for_server(&opts.server) {
                Ok(id) => {
                    log::info!(
                        "offering OpenSec identity {} to {}",
                        fingerprint(&id.public()),
                        opts.server
                    );
                    Some(id)
                }
                Err(e) => {
                    log::warn!("couldn't save an identity key, offering none: {e}");
                    None
                }
            }
        });
        let handshake = client_handshake(opts.security, pin, identity, seed);
        let mut session = Session::with_handshake(cfg, now, handshake);
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
            early_chat: Vec::new(),
            transfers_started: 0,
            transfers_done: 0,
            known,
            pinned: pin.is_some(),
            server: opts.server.clone(),
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
            early_chat: Vec::new(),
            transfers_started: 0,
            transfers_done: 0,
            known: KnownServers::default(),
            pinned: false,
            server: String::new(),
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
                SessionEvent::Connected => {
                    let cipher = self.session.cipher_name().unwrap_or("?");
                    self.status = format!("Logging in ({cipher})...");
                    log::info!("connected to {} with {cipher}", self.server);
                    // Trust on first use: remember an OpenSec server's key.
                    if let (false, Some(key)) = (self.pinned, self.session.server_key()) {
                        match self.known.remember(&self.server, key) {
                            Ok(()) => log::info!("remembered {}'s OpenSec key", self.server),
                            Err(e) => log::warn!("couldn't save the server key: {e}"),
                        }
                    }
                }
                SessionEvent::LoggedIn => {
                    // Logged, because its absence is a diagnostic hole: without it a
                    // log cannot distinguish "the zone refused us" from "it took us
                    // and dropped us later".
                    log::info!("logged in to {}", self.server);
                    self.status = "Entering arena...".into();
                    // Continuum takes the zone's news on login, and only when its
                    // checksum differs from the copy it kept.
                    let cached = self.stored_file(NEWS_FILE);
                    self.session.request_news_if_new(cached.as_deref());
                }
                SessionEvent::LoginFailed { message, .. } => {
                    self.error = Some(format!("Login failed: {message}"))
                }
                SessionEvent::MapNeeded { name, .. } => {
                    // The arena's content starts here, so the count starts here too.
                    self.transfers_started = 0;
                    self.transfers_done = 0;
                    if self
                        .stored_file(name)
                        .is_some_and(|d| self.session.use_cached_map(&d))
                    {
                        log::info!("map {name} from cache");
                    } else {
                        self.transfers_started += 1;
                        self.status = format!("Downloading {name}...");
                        self.session.download_map();
                    }
                }
                SessionEvent::TransferProgress {
                    name,
                    received,
                    total,
                } if *total > 0 => {
                    self.status = progress_line(
                        name,
                        *received,
                        *total,
                        self.transfers_done,
                        self.transfers_started,
                    );
                }
                SessionEvent::MapReady {
                    name,
                    data,
                    downloaded,
                } => {
                    if *downloaded {
                        self.transfers_done += 1;
                        if !self.is_replay()
                            && let Err(e) = self.cache.store(name, data)
                        {
                            log::warn!("couldn't cache map {name}: {e}");
                        }
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
                            self.transfers_started += 1;
                            self.session.download_file(&f.name);
                        }
                    }
                }
                SessionEvent::FileReady {
                    name,
                    data,
                    downloaded,
                } => {
                    if *downloaded {
                        self.transfers_done += 1;
                        if !self.is_replay()
                            && let Err(e) = self.cache.store(name, data)
                        {
                            log::warn!("couldn't cache {name}: {e}");
                        }
                    }
                    self.files.push((name.clone(), data.clone()));
                }
                SessionEvent::News(text) => {
                    if !self.is_replay()
                        && let Err(e) = self.cache.store(NEWS_FILE, text.as_bytes())
                    {
                        log::warn!("couldn't cache the zone news: {e}");
                    }
                }
                SessionEvent::Closed(reason) => {
                    // Logged before anything else is decided. A disconnect used to
                    // reach only the login screen, so the log simply stopped
                    // mid-session with no reason in it -- which looks complete, and
                    // is worse than no log at all. Diagnosing one meant
                    // screenshotting the window: no use to anyone reading the log
                    // afterwards, and impossible headless.
                    log::warn!("disconnected from {}: {reason:?}", self.server);
                    if self.error.is_none() {
                        let handshake = matches!(reason, oss_session::Reason::HandshakeFailed(_));
                        self.error = Some(if handshake && self.pinned {
                            format!(
                                "{} didn't prove the OpenSec key on record. If the zone really \
                             changed its key, remove its line from known_servers.txt.",
                                self.server
                            )
                        } else {
                            format!("Disconnected ({reason:?})")
                        });
                    }
                }
                // Keep chat that arrives before the game screen exists; see
                // `early_chat`. `ready()` needs the arena, its settings and its
                // files, so anything the zone says during login would otherwise
                // be dropped by the connecting screen.
                SessionEvent::Chat { .. } if !self.ready() => self.early_chat.push(ev.clone()),
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

    /// Chat the zone sent before the game screen existed, in arrival order, taken
    /// once. The game shows these as its first lines so nothing the zone said
    /// during login is lost.
    pub fn take_early_chat(&mut self) -> Vec<SessionEvent> {
        std::mem::take(&mut self.early_chat)
    }

    /// Tells the zone we are leaving, and puts it on the wire before we go.
    ///
    /// Nothing sends this on drop, so without it every quit reached the server as a
    /// timeout: it keeps the slot and waits the connection out. That is rude on our
    /// own zone and worse on someone else's.
    pub fn disconnect(&mut self) {
        self.session.disconnect();
        self.flush();
    }

    pub fn flush(&mut self) {
        let now = self.now_ms();
        self.session.flush(now);
        if let Source::Live(d) = &mut self.source {
            let _ = d.send_pending(&mut self.session);
        }
    }
}

/// How far through the arena's whole set of transfers we are.
///
/// Across the set, not within the current file: a zone sends a map and up to a
/// dozen LVZ files, and a per-file percentage restarts at every one of them. The
/// first person to watch it reported the figures "rapidly bounce randomly between
/// 0 and 100 about 20-30 times", which is exactly what a dozen restarts look like.
/// This rises once and never falls back.
///
/// `done` is the number finished, so `done * 100` is the ground already covered and
/// the current file contributes its own fraction of one slot.
fn overall_percent(received: usize, total: usize, done: usize, started: usize) -> usize {
    let within_file = (received * 100).checked_div(total).unwrap_or(0);
    match started {
        // Nothing to average over: the one file's own progress is the honest answer.
        0 | 1 => within_file,
        // Clamped because a zone may send a file that was not in the list it gave.
        n => ((done * 100 + within_file) / n).min(100),
    }
}

/// The line shown while a zone's content is coming down: what is being fetched,
/// where it sits in the set, and one percentage for the set as a whole.
fn progress_line(name: &str, received: usize, total: usize, done: usize, started: usize) -> String {
    let percent = overall_percent(received, total, done, started);
    match started {
        0 | 1 => format!("Downloading {name}... {percent}%"),
        n => format!(
            "Downloading {name} ({} of {n})... {percent}%",
            (done + 1).min(n)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{overall_percent, progress_line};

    #[test]
    fn progress_names_the_file_and_its_place_in_the_set() {
        // Second of fourteen, one finished, this one a tenth in.
        assert_eq!(
            progress_line("hz_sb.lvz", 10, 100, 1, 14),
            "Downloading hz_sb.lvz (2 of 14)... 7%"
        );
        // The last one, nearly done.
        assert_eq!(
            progress_line("tutorial.lvz", 99, 100, 13, 14),
            "Downloading tutorial.lvz (14 of 14)... 99%"
        );
        // A file beyond the list must not read "15 of 14", nor over 100%.
        assert_eq!(
            progress_line("extra.lvz", 50, 100, 14, 14),
            "Downloading extra.lvz (14 of 14)... 100%"
        );
        // A lone transfer reports itself, without a meaningless "1 of 1".
        assert_eq!(
            progress_line("smallmap.lvl", 40, 100, 0, 1),
            "Downloading smallmap.lvl... 40%"
        );
    }

    /// The whole point of the change: the figure must never fall back. Walks a
    /// fourteen-file set from nothing to finished, the way a real arena arrives.
    #[test]
    fn the_percentage_never_goes_backwards_across_a_set() {
        let files = 14;
        let mut last = 0;
        for done in 0..files {
            for received in (0..=100).step_by(7) {
                let now = overall_percent(received, 100, done, files);
                assert!(
                    now >= last,
                    "fell from {last}% to {now}% at file {done}, {received}/100"
                );
                assert!(now <= 100, "{now}% over 100 at file {done}");
                last = now;
            }
        }
        // A set that finishes reads as finished.
        assert_eq!(overall_percent(100, 100, files - 1, files), 100);
    }

    /// A zone that reports a zero-length transfer must not divide by zero.
    #[test]
    fn a_zero_length_transfer_is_survivable() {
        assert_eq!(overall_percent(0, 0, 3, 14), 21);
    }
}
