//! A client-side SubSpace game session: the protocol layer between the transport
//! (`oss-net`) and a game (the graphical client or a headless bot).
//!
//! It is sans-IO like `oss-net`: feed it datagrams and the time, poll it for datagrams
//! to send and for [`SessionEvent`]s. [`udp::UdpDriver`] connects it to a real socket.
//!
//! The session logs in, enters an arena, keeps the arena settings and player list,
//! fetches the map (or accepts one from a cache), answers security checks and sends
//! your position. Packet handling follows ASSS (server truth) and the nullspace and
//! mervbot clients.

pub mod client;
pub mod directory;
pub mod map;
pub mod replay;
pub mod security;
pub mod sim;
pub mod test_zone;
pub mod udp;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use oss_net::{
    CipherKind, Config, Connection, DisconnectReason, Event as NetEvent, Handshake,
    VieClientHandshake,
};
use oss_proto::c2s::{
    self, GoArena, Login, MapRequest, Registration, SecurityResponse, SetFreq, SetShip,
};
use oss_proto::s2c::ChatBody;
use oss_proto::s2c::{self, MapFilename, login_code};
use oss_proto::wire::Redacted;
use oss_proto::{C2SPosition, ClientSettings, FixedStr, Packet, S2CPacket, WeaponInfo, checksum};

pub use client::{ClientProfile, LoginForm, Profile, SecurityAnswer, SecurityChallenge, VieClient};
pub use oss_net::DisconnectReason as Reason;
pub use oss_proto::common::chat_type;
pub use oss_proto::common::{Brick, ObjectChange, object_change};
pub use oss_proto::s2c::BallPosition;

/// Spectator "ship".
pub const SPECTATOR: u8 = 8;

/// Who to log in as and where to go.
///
/// `Debug` is written out rather than derived so the password cannot be printed.
/// See the impl below.
#[derive(Clone)]
pub struct SessionConfig {
    pub name: String,
    pub password: String,
    /// Arena to enter; `None` for any public arena.
    pub arena: Option<String>,
    /// Ship to enter in: 0..=7, or [`SPECTATOR`].
    pub ship: u8,
    /// Screen size reported to the server (affects what it sends you).
    pub resolution: (u16, u16),
    pub machine_id: u32,
    /// Minutes from UTC.
    pub timezone_bias: i16,
    /// Set the arena-entry packet's optional-graphics byte: *"send me every LVZ
    /// file, including the ones this arena marks optional"*. Bots usually don't
    /// need any of them.
    ///
    /// **It does two different jobs depending on what kind of client the server
    /// thinks you are, and the difference is worth knowing before changing it.**
    /// ASSS decides what to list in `core/mapnewsdl.c`:
    ///
    /// ```text
    /// if (p->type == T_CONT || p->flags.want_all_lvz)    /* line 137 */
    ///     for each file: if (!data->optional || p->flags.want_all_lvz)  /* 145 */
    /// else
    ///     /* the map, and nothing else */
    /// ```
    ///
    /// `want_all_lvz` is this byte (`core/arenaman.c:599`), and `T_CONT` means the
    /// session negotiated Continuum's own cipher — which this repository does not
    /// implement, so a client built from here is `T_VIE`. So:
    ///
    /// - **A Continuum-capable build with this set** is listed the arena's files
    ///   *including the optional ones*, which Continuum itself is never offered.
    ///   That is strictly more downloading than the real client does, and
    ///   clearing the byte is what makes it match.
    /// - **A build from this repository with this set** is listed the same
    ///   everything. Clearing the byte leaves it the map and no graphics at all,
    ///   because the byte is the only way a non-Continuum client is offered any.
    ///
    /// A zone marks a file optional by prefixing it with `+` in its arena
    /// settings (`General:LevelFiles`, read in `core/mapdata.c:521`);
    /// `SubspaceServer.NET` does the same thing in
    /// `Modules/MapNewsDownload.cs:155,161`. Both read on 2026-11-17.
    pub want_lvz: bool,
    /// Say yes, without asking, when the zone offers to create the name.
    ///
    /// A zone that does not know a name answers the login with
    /// [`login_code::NEW_USER`]. The real clients then ask the player and, on a
    /// yes, send the same login again on the same connection with the packet's
    /// "new user" byte set (mervbot `generatePassword`, `clientprot.cpp`). This
    /// session does the same: the first login always goes out with the byte
    /// clear, the offer arrives as [`SessionEvent::NewUserRequired`], and
    /// [`Session::create_account`] is the yes. With this set, the session gives
    /// that yes itself, which is what a script or a bot wants.
    ///
    /// So this can never create an account the zone did not first say was new.
    /// Subgame's accounts do not survive a zone restart, so against a freshly
    /// started subgame **every** name is new and every login takes the extra
    /// round trip. ASSS reads the byte only on the billing path (`billing_ssc.c`
    /// forwards it as `MakeNew`); a billerless ASSS zone takes any name either way.
    pub new_user: bool,
    /// Which client to present as on the wire: the login form and the answer to a
    /// security challenge. Defaults to [`VieClient`].
    ///
    /// A Continuum cipher needs the Continuum forms with it — an ASSS zone picks
    /// which it expects from the encryption, not from the packet's type byte, and
    /// drops a mismatched login without saying so. See [`mod@client`].
    pub client: Profile,
    /// The login id a redirect handed over ([`SessionEvent::Redirected`]),
    /// carried in the login packet to the zone it pointed at. Zero for an
    /// ordinary login, and zero in every redirect seen so far: both servers
    /// whose source we have always send it as zero.
    pub redirect_id: u32,
}

/// Everything but the password, which prints as `<redacted>`.
///
/// This is the config handed to a [`Session`], so it is the likeliest of all of
/// them to be logged while someone is working out why a connection failed. Redacted
/// at the field rather than on the type: every other field is worth reading, and a
/// type that prints nothing is a type nobody can debug.
impl fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionConfig")
            .field("name", &self.name)
            .field("password", &Redacted)
            .field("arena", &self.arena)
            .field("ship", &self.ship)
            .field("resolution", &self.resolution)
            .field("machine_id", &self.machine_id)
            .field("timezone_bias", &self.timezone_bias)
            .field("want_lvz", &self.want_lvz)
            .field("new_user", &self.new_user)
            .field("client", &self.client)
            .field("redirect_id", &self.redirect_id)
            .finish()
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            name: "oss-player".into(),
            password: String::new(),
            arena: None,
            ship: SPECTATOR,
            resolution: (1920, 1080),
            machine_id: 0x0C55_0C55,
            timezone_bias: 0,
            want_lvz: false,
            new_user: false,
            client: client::vie(),
            redirect_id: 0,
        }
    }
}

/// Where the session is in the login sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connecting,
    LoggingIn,
    EnteringArena,
    /// Waiting for the map (from the cache or the server).
    WaitingForMap,
    Playing,
    Closed,
}

/// A player in the arena (including you).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub pid: i16,
    pub name: String,
    pub squad: String,
    /// 0..=7, or [`SPECTATOR`].
    pub ship: u8,
    pub freq: i16,
    pub kill_points: i32,
    pub flag_points: i32,
    pub wins: i16,
    pub losses: i16,
    /// Flags carried.
    pub flags: i16,
    /// Last position received (pixels, pixels/10 s), if any.
    pub position: Option<RemotePosition>,
}

/// A position update for another player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemotePosition {
    pub x: i16,
    pub y: i16,
    pub x_speed: i16,
    pub y_speed: i16,
    /// 0..40.
    pub rotation: u8,
    /// [`oss_proto::position::status`] bits.
    pub status: u8,
    pub bounty: u16,
    /// Energy, when the server sends it.
    pub energy: Option<u16>,
    /// Server time of the update (low 16 bits for small packets, extended).
    pub server_ticks: u32,
    /// A weapon fired with this update.
    pub weapon: Option<WeaponInfo>,
}

/// Your own ship, as reported to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OwnPosition {
    /// Pixels.
    pub x: i16,
    pub y: i16,
    /// Pixels per 10 seconds.
    pub x_speed: i16,
    pub y_speed: i16,
    /// 0..40.
    pub rotation: u8,
    pub status: u8,
    pub bounty: u16,
    pub energy: i16,
    pub weapon: WeaponInfo,
    /// Dead/respawning: the server is told you are nowhere.
    pub dead: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// The transport handshake finished; logging in.
    Connected,
    /// The zone does not know this name and offers to create it
    /// ([`login_code::NEW_USER`]). The connection is still open: answer with
    /// [`Session::create_account`], or [`Session::disconnect`] to decline.
    ///
    /// Show the player `name`. Quoting it back is the guard against a typo
    /// becoming a permanent account: the player believes they typed their own
    /// name correctly until they are shown otherwise.
    NewUserRequired {
        name: String,
        /// The zone's own words, when it sent some (S2C `0x33`).
        zone_text: Option<String>,
    },
    /// The zone let the player in and asks for its registration form (the login
    /// reply's demographics byte). The session waits, short of the arena, until
    /// [`Session::send_registration`] or [`Session::skip_registration`].
    RegistrationWanted,
    /// The zone refused the login.
    LoginFailed {
        code: u8,
        /// What the code means, in a sentence for the player, with what to do
        /// about it.
        message: &'static str,
        /// The zone's own words, when it sent some (S2C `0x33`, just before the
        /// refusal). They are what the player should read: a zone that locks a
        /// name out after too many wrong passwords says so, and for how long,
        /// while the code it refuses with only says "busy".
        zone_text: Option<String>,
    },
    LoggedIn,
    /// Your player id in the arena.
    ///
    /// **A deliberate divergence in pace.** Measured against Trench Wars on
    /// 2026-11-12, a real Continuum client took about eight seconds between the
    /// login reply and entering the arena, where this one takes about 27
    /// milliseconds. The eight seconds were the news file and the settings
    /// arriving over a connection nobody has any more; nothing in the protocol
    /// asks for the wait, no zone met so far has objected to arriving quickly,
    /// and the news is still fetched -- [`Session::request_news_if_new`], on
    /// login, skipping it when the cached copy matches the zone's checksum,
    /// which is what Continuum does. So the gap is recorded rather than
    /// reproduced: if a zone is ever found that needs the pause, it belongs
    /// here, with that zone named.
    EnteredArena {
        pid: i16,
    },
    /// Arena settings (sent on entry and whenever they change).
    Settings(Box<ClientSettings>),
    /// The arena's map. Answer with [`Session::use_cached_map`] or
    /// [`Session::download_map`].
    MapNeeded {
        name: String,
        checksum: u32,
    },
    /// A sized transfer is partway through. Fires for the map, for every LVZ file
    /// and for anything else fetched this way -- it was called `MapProgress` and
    /// carried no name, so a client could only show a bare percentage that
    /// restarted at every file and looked like it was stuck in a loop.
    TransferProgress {
        /// What is being fetched, for a caller that wants to say so.
        name: String,
        received: usize,
        total: usize,
    },
    /// The map is ready (decompressed `.lvl` bytes). Cache it if it was downloaded.
    /// The arena's extra files (LVZ), listed after its map. Answer each with
    /// [`Session::use_cached_file`] or [`Session::download_file`].
    FilesListed(Vec<ArenaFile>),
    /// An extra file is ready (raw bytes). Cache it if it was downloaded.
    FileReady {
        name: String,
        data: Vec<u8>,
        downloaded: bool,
    },
    /// The zone's news file, as text ([`Session::request_news`] asked for it).
    News(String),
    /// A flag is on the map at tile (x, y), owned by `freq` (`0xFFFF` neutral)
    /// (S2C `0x12`).
    FlagLocation {
        id: u16,
        x: u16,
        y: u16,
        freq: u16,
    },
    /// A player picked up (or, for turf flags, claimed) a flag (S2C `0x13`).
    FlagPickup {
        id: u16,
        pid: i16,
    },
    /// A player dropped every flag they carried (S2C `0x16`).
    FlagsDropped {
        pid: i16,
    },
    /// A team won the flag game (S2C `0x14`).
    FlagVictory {
        freq: i16,
        points: i32,
    },
    /// Owner of every static (turf) flag, by flag id; -1 = none (S2C `0x22`).
    TurfFlags(Vec<i16>),
    /// Periodic flag reward: (freq, points per player) (S2C `0x23`).
    FlagReward(Vec<(i16, i16)>),
    /// A ball's position, carrier or shooter (S2C `0x2E`).
    Ball(BallPosition),
    /// A team scored a goal (S2C `0x0B`).
    Goal {
        freq: i16,
        points: i32,
    },
    /// `pid` attached to `host` as a turret, or detached (`host` -1) (S2C `0x0E`).
    Attached {
        pid: i16,
        host: i16,
    },
    /// Everyone attached to `pid` was detached (S2C `0x15`).
    TurretsDetached {
        pid: i16,
    },
    /// Bricks placed (S2C `0x21`); an empty list clears them all.
    Bricks(Vec<Brick>),
    /// King of the hill: a crown given (`on`) or taken, for `pid` (-1 = everyone)
    /// (S2C `0x2C`).
    Crown {
        on: bool,
        ticks: u32,
        pid: i16,
    },
    /// Your crown timer set (`add` false) or extended (S2C `0x2B`/`0x2D`).
    CrownTimer {
        ticks: u32,
        add: bool,
    },
    /// LVZ objects shown (`true`) or hidden (S2C `0x35`).
    ObjectsToggled(Vec<(u16, bool)>),
    /// LVZ object properties changed (S2C `0x36`).
    ObjectsChanged(Vec<ObjectChange>),
    MapReady {
        name: String,
        data: Vec<u8>,
        downloaded: bool,
    },
    /// The arena entry sequence finished; positions can flow.
    Playing,
    PlayerEntered(i16),
    PlayerLeft(i16),
    PlayerMoved(i16),
    ShipChanged {
        pid: i16,
        ship: u8,
        freq: i16,
    },
    FreqChanged {
        pid: i16,
        freq: i16,
    },
    Kill {
        killer: i16,
        killed: i16,
        bounty: i16,
        green: u8,
    },
    Chat {
        kind: u8,
        pid: i16,
        name: Option<String>,
        text: String,
        sound: u8,
    },
    /// Seeds for greens and doors (S2C `0x18`).
    Seeds {
        green_seed: u32,
        door_seed: u32,
        timestamp: u32,
    },
    /// The server asked for a warp to (x, y) tiles.
    WarpTo {
        x: i16,
        y: i16,
    },
    /// The server reset your ship (respawn with initial loadout).
    ShipReset,
    /// Another player picked up the green on tile (x, y) (S2C `0x08`).
    GreenTaken {
        pid: i16,
        x: i16,
        y: i16,
        prize: i16,
    },
    /// A prize from the server: `count` copies of `prize` (negative = bad).
    PrizeReceived {
        count: i16,
        prize: i16,
    },
    Closed(DisconnectReason),
    /// The zone sent the player on to another zone (S2C `0x3B`), and this
    /// session has hung up on the first. Connect to `server` with the same name
    /// and password, put `login_id` in [`SessionConfig::redirect_id`], and enter
    /// `arena` (`None`: any public arena).
    ///
    /// Some zones in the live list are only this: the zone takes the login and
    /// answers the arena request with a redirect to where the game really is.
    /// A client that ignores it waits for an arena that never comes.
    Redirected {
        /// `a.b.c.d:port`.
        server: String,
        arena: Option<String>,
        login_id: u32,
    },
    /// The zone's arenas, in the order it sent them, as (name, players, whether
    /// it is the arena you are in) (S2C `0x2F`). Zones send it in answer to
    /// `?arena`; the wire marks your own arena by sending its count negated.
    ArenaList(Vec<ArenaListed>),
    /// Any packet the session doesn't interpret itself.
    Other(Box<S2CPacket>),
}

/// One arena on the list a zone sends in answer to `?arena`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaListed {
    pub name: String,
    /// How many players are in it.
    pub players: u16,
    /// The arena you are in. Continuum shows it first, marked `>`.
    pub yours: bool,
}

/// The map the arena uses.
/// A file the arena lists after its map (an LVZ).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaFile {
    /// Index for the file request (1 = first file after the map).
    pub index: u16,
    pub name: String,
    /// CRC-32 of the file.
    pub checksum: u32,
    pub size: u32,
}

#[derive(Debug, Clone)]
struct MapInfo {
    name: String,
    checksum: u32,
    /// Row-major 1024×1024 tiles, for the security map checksum.
    tiles: Option<Vec<u8>>,
}

pub struct Session {
    conn: Connection,
    cfg: SessionConfig,
    /// The zone's address and our own bound port, for the login's tail.
    /// `(0, 0)` until a driver supplies them; see
    /// [`Session::set_local_endpoint`].
    endpoint: (u32, u16),
    state: State,
    pid: Option<i16>,
    players: BTreeMap<i16, Player>,
    settings: Option<Box<ClientSettings>>,
    settings_raw: Vec<u8>,
    map: Option<MapInfo>,
    /// Extra files listed with the map.
    files: Vec<ArenaFile>,
    /// Flag ids the server reported as turf (static) flags.
    turf_flags: std::collections::BTreeSet<u16>,
    /// File requests waiting for the current transfer to finish (0 = the map).
    downloads: VecDeque<u16>,
    /// A map or file transfer is in flight.
    downloading: Option<u16>,
    /// CRC-32 of the zone's news file from the login reply (0 = the zone has none).
    news_checksum: u32,
    /// The zone's own words about the login (S2C `0x33`), held until its answer
    /// arrives so they go out with it.
    login_text: Option<String>,
    /// A news request is in flight (the reply is an S2C `0x10` with no name).
    news_pending: bool,
    entering_done: bool,
    events: VecDeque<SessionEvent>,
    weapons_received: u32,
    last_position_ticks: Option<u32>,
    /// A security check waiting for the map to load (its checksum is part of the
    /// answer).
    pending_security: Option<u32>,
    last_now_ms: u64,
    buf: Vec<u8>,
    /// Records the session to a replay file.
    recorder: Option<replay::Recorder>,
    /// Playing a replay back: no network, the clock comes from the recording.
    playback: bool,
    /// Playback clock: (session time, server ticks then).
    replay_clock: Option<(u64, u32)>,
    /// When the server last sent anything (session ms).
    last_data_ms: u64,
    /// The login on the wire carries the "new user" byte. Set once, by
    /// [`Session::create_account`]; a second [`login_code::NEW_USER`] after it is
    /// a refusal, so a zone that keeps answering it cannot loop us.
    new_user_sent: bool,
    /// [`SessionEvent::NewUserRequired`] is waiting for an answer.
    awaiting_new_user: bool,
    /// [`SessionEvent::RegistrationWanted`] is waiting for an answer.
    awaiting_registration: bool,
}

impl Session {
    /// A session using VIE encryption (what SubSpace servers expect).
    pub fn new(cfg: SessionConfig, now_ms: u64, seed: u32) -> Self {
        Self::with_handshake(cfg, now_ms, Box::new(VieClientHandshake::new(seed)))
    }

    /// Tell the session what address it is dialling and which local port it is
    /// dialling from, so the login can carry them.
    ///
    /// The login's tail reports both, and some servers read them: a zero
    /// server address together with a zero port is taken to mean a SOCKS5
    /// proxy, and the port is compared with the datagram's source port to
    /// detect NAT. **That is the Subspace Server .NET model, and it is the only
    /// one documented here, not the only one there is**: measured over six
    /// logins on 2026-11-12, a live zone enforced the address and the port
    /// *each independently* and rejected a zero address sent with a real port,
    /// which the conjunctive rule would have let through. Fill in both.
    /// Neither is knowable here — `oss_net` owns no socket — so
    /// whatever drives the socket supplies them. [`crate::udp::UdpDriver`] does it
    /// for you; a caller driving its own socket should call this before the
    /// session connects, and one that does not simply reports zeros, as it did
    /// before this existed.
    ///
    /// `server_ip` is the IPv4 address as a host-order `u32` (what
    /// `u32::from(Ipv4Addr)` gives); 0 for IPv6, which the field cannot carry.
    pub fn set_local_endpoint(&mut self, server_ip: u32, bound_port: u16) {
        self.endpoint = (server_ip, bound_port);
    }

    /// A session with any client handshake (e.g. unencrypted, or a private cipher).
    pub fn with_handshake(cfg: SessionConfig, now_ms: u64, handshake: Box<dyn Handshake>) -> Self {
        Self {
            conn: Connection::client(Config::client(), handshake, now_ms),
            cfg,
            endpoint: (0, 0),
            state: State::Connecting,
            pid: None,
            players: BTreeMap::new(),
            settings: None,
            settings_raw: Vec::new(),
            map: None,
            files: Vec::new(),
            downloads: VecDeque::new(),
            turf_flags: Default::default(),
            downloading: None,
            news_checksum: 0,
            login_text: None,
            news_pending: false,
            entering_done: false,
            events: VecDeque::new(),
            weapons_received: 0,
            last_position_ticks: None,
            pending_security: None,
            last_now_ms: now_ms,
            buf: Vec::with_capacity(512),
            recorder: None,
            playback: false,
            replay_clock: None,
            last_data_ms: now_ms,
            new_user_sent: false,
            awaiting_new_user: false,
            awaiting_registration: false,
        }
    }

    /// A session that plays a replay back: it sends nothing, and the records given to
    /// [`Session::feed`] drive it as packets from a server would.
    pub fn playback(cfg: SessionConfig, now_ms: u64) -> Self {
        let mut s = Self::new(cfg, now_ms, 0);
        s.playback = true;
        s.state = State::LoggingIn;
        s
    }

    pub fn is_playback(&self) -> bool {
        self.playback
    }

    /// Records everything from now on into `rec` (start before connecting to record
    /// a whole visit; a recording started later lacks what came before).
    pub fn record_to(&mut self, mut rec: replay::Recorder, now_ms: u64) {
        rec.meta(
            now_ms,
            "server_ticks",
            &self.server_ticks(now_ms).to_string(),
        );
        self.recorder = Some(rec);
    }

    /// Stops recording (the file is flushed and closed).
    pub fn stop_recording(&mut self) {
        self.recorder = None;
    }

    pub fn recording(&self) -> bool {
        self.recorder.as_ref().is_some_and(|r| !r.failed())
    }

    /// Adds a bookmark to the recording.
    pub fn mark(&mut self, now_ms: u64, text: &str) {
        if let Some(r) = self.recorder.as_mut() {
            r.marker(now_ms, text);
        }
    }

    /// Playback: applies one replay record at session time `now_ms`.
    pub fn feed(&mut self, now_ms: u64, rec: &oss_formats::replay::Record) {
        use oss_formats::replay::RecordKind;
        self.last_now_ms = now_ms;
        self.last_data_ms = now_ms;
        match rec.kind {
            RecordKind::ServerPacket => self.handle_packet(now_ms, &rec.data),
            RecordKind::Meta => {
                if let Some(("server_ticks", v)) = rec.as_meta()
                    && let Ok(ticks) = v.parse::<u32>()
                {
                    self.replay_clock = Some((now_ms, ticks));
                }
            }
            // The recorder's own movement and shots, shown like anyone else's.
            RecordKind::ClientPacket => {
                if let Ok(oss_proto::C2SPacket::Position(p)) =
                    oss_proto::C2SPacket::decode(&rec.data)
                    && let Some(pid) = self.pid
                {
                    self.update_position(
                        pid,
                        RemotePosition {
                            x: p.x,
                            y: p.y,
                            x_speed: p.x_speed,
                            y_speed: p.y_speed,
                            rotation: (p.rotation as u8) % 40,
                            status: p.status,
                            bounty: p.bounty,
                            energy: Some(p.energy.max(0) as u16),
                            server_ticks: p.time,
                            weapon: (p.weapon.kind() != 0).then_some(p.weapon),
                        },
                    );
                }
            }
            _ => {}
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn config(&self) -> &SessionConfig {
        &self.cfg
    }

    /// Your player id, once in an arena.
    pub fn pid(&self) -> Option<i16> {
        self.pid
    }

    pub fn me(&self) -> Option<&Player> {
        self.pid.and_then(|p| self.players.get(&p))
    }

    pub fn players(&self) -> impl Iterator<Item = &Player> {
        self.players.values()
    }

    pub fn player(&self, pid: i16) -> Option<&Player> {
        self.players.get(&pid)
    }

    pub fn settings(&self) -> Option<&ClientSettings> {
        self.settings.as_deref()
    }

    /// Server time in ticks (1/100 s), from the transport's clock sync (or, playing
    /// a replay, from the recording's clock).
    pub fn server_ticks(&self, now_ms: u64) -> u32 {
        match self.replay_clock {
            Some((base_ms, ticks)) => {
                ticks.wrapping_add((now_ms.saturating_sub(base_ms) / 10) as u32)
            }
            None => self.conn.server_ticks(now_ms),
        }
    }

    /// Milliseconds since the server last sent anything.
    pub fn ms_since_data(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.last_data_ms)
    }

    /// The connection's encryption ("opensec", "vie", ...) once connected.
    pub fn cipher_name(&self) -> Option<&'static str> {
        self.conn.cipher_name()
    }

    /// The server's OpenSec key, when it sent one.
    pub fn server_key(&self) -> Option<[u8; 32]> {
        self.conn.peer_key()
    }

    pub fn rtt_ms(&self) -> u64 {
        self.conn.rtt_ms()
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Feeds one received datagram.
    pub fn handle_datagram(&mut self, now_ms: u64, data: &[u8]) {
        self.last_now_ms = now_ms;
        self.last_data_ms = now_ms;
        self.conn.handle_datagram(now_ms, data);
        self.drain_net(now_ms);
    }

    /// Drives timers (retransmits, keepalive, clock sync). Call every ~10 ms.
    pub fn tick(&mut self, now_ms: u64) {
        self.last_now_ms = now_ms;
        self.conn.tick(now_ms);
        self.drain_net(now_ms);
    }

    /// Packs anything queued into datagrams now (call after sending game packets).
    pub fn flush(&mut self, now_ms: u64) {
        self.conn.flush(now_ms);
        if let Some(r) = self.recorder.as_mut() {
            r.flush();
        }
    }

    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        self.conn.poll_transmit()
    }

    /// Whether the connection has stepped down to its next encryption
    /// mechanism and wants that one to start from a fresh local endpoint (true
    /// once per step down). A driver sends what [`Session::poll_transmit`]
    /// holds first, then rebinds to a new local port and reports it with
    /// [`Session::set_local_endpoint`], before the next tick. [`UdpDriver`]
    /// does this itself.
    ///
    /// [`UdpDriver`]: crate::udp::UdpDriver
    pub fn take_fresh_endpoint(&mut self) -> bool {
        self.conn.take_fresh_endpoint()
    }

    pub fn poll_event(&mut self) -> Option<SessionEvent> {
        self.events.pop_front()
    }

    pub fn disconnect(&mut self) {
        self.awaiting_new_user = false;
        self.awaiting_registration = false;
        self.conn.disconnect();
    }

    /// The yes to [`SessionEvent::NewUserRequired`]: sends the same login again,
    /// on the same connection, asking the zone to create the name with the
    /// password already given.
    ///
    /// The same connection, because ASSS (`core.c`: a failed login returns the
    /// player to `S_CONNECTED`, the one state a login is accepted in) and
    /// `oss-server` both keep it open for exactly this. Returns `false`, and sends
    /// nothing, when no offer is waiting.
    pub fn create_account(&mut self) -> bool {
        if !std::mem::take(&mut self.awaiting_new_user) {
            return false;
        }
        log::info!("asking the zone to create {:?}", self.cfg.name);
        self.new_user_sent = true;
        self.send_login();
        true
    }

    /// Whether [`SessionEvent::NewUserRequired`] is waiting for an answer.
    pub fn new_user_offered(&self) -> bool {
        self.awaiting_new_user
    }

    /// Whether [`SessionEvent::RegistrationWanted`] is waiting for an answer.
    pub fn registration_wanted(&self) -> bool {
        self.awaiting_registration
    }

    /// Answers [`SessionEvent::RegistrationWanted`] with the form (C2S `0x17`),
    /// then goes on into the arena. Returns `false`, and sends nothing, when the
    /// zone has not asked or the form has already gone.
    ///
    /// Only the real name and the e-mail come from the player. The rest of the
    /// 1998 form is [`Registration::default`] -- empty city and state, age 0, no
    /// processor -- with "connecting from home" set, as nullspace's version of
    /// the form sets it. The machine fields (the Windows registered owner and
    /// organisation, and thirteen hardware strings) go empty: nothing about the
    /// computer leaves it.
    pub fn send_registration(&mut self, real_name: &str, email: &str) -> bool {
        if !std::mem::take(&mut self.awaiting_registration) {
            return false;
        }
        let form = Registration {
            real_name: FixedStr::new(real_name.trim().as_bytes()),
            email: FixedStr::new(email.trim().as_bytes()),
            from_home: 1,
            ..Default::default()
        };
        self.send(&form, true);
        self.go_arena();
        true
    }

    /// Declines [`SessionEvent::RegistrationWanted`]: sends nothing and goes on
    /// into the arena. Legal: the form is data the zone forwards if it comes,
    /// and ASSS and Subspace Server .NET both treat its absence as nothing
    /// happening. Returns `false` when the zone has not asked.
    pub fn skip_registration(&mut self) -> bool {
        if !std::mem::take(&mut self.awaiting_registration) {
            return false;
        }
        self.go_arena();
        true
    }

    fn send(&mut self, packet: &impl Packet, reliable: bool) {
        if self.playback {
            return;
        }
        self.buf.clear();
        packet.encode(&mut self.buf);
        let data = std::mem::take(&mut self.buf);
        if let Some(r) = self.recorder.as_mut() {
            r.client_packet(self.last_now_ms, &data);
        }
        self.conn.send(&data, reliable);
        self.buf = data;
    }

    fn drain_net(&mut self, now_ms: u64) {
        while let Some(ev) = self.conn.poll_event() {
            match ev {
                NetEvent::Connected => {
                    self.state = State::LoggingIn;
                    self.events.push_back(SessionEvent::Connected);
                    self.send_login();
                }
                NetEvent::Packet(p) => self.handle_packet(now_ms, &p),
                NetEvent::SizedProgress { received, total } => {
                    let name = self.transfer_name();
                    self.events.push_back(SessionEvent::TransferProgress {
                        name,
                        received,
                        total,
                    });
                }
                NetEvent::Disconnected(reason) => {
                    self.state = State::Closed;
                    self.events.push_back(SessionEvent::Closed(reason));
                }
                _ => {}
            }
        }
    }

    fn send_login(&mut self) {
        // Which form, and the fields that go with it, come from the profile: a
        // Continuum zone drops a 101-byte login from a connection that negotiated
        // Continuum encryption. See [`mod@client`].
        let form = self.cfg.client.login();
        // The profile and the cipher are chosen separately and have to agree: a zone
        // picks the login form it expects from the *encryption*, not from the packet's
        // type byte, and drops a mismatch with nothing on the wire to say why. This is
        // the one place both are in hand. `Connection::establish` sets the cipher
        // before it pushes `Connected`, and this runs from that arm, so the kind is
        // already known here.
        //
        // Logged, not asserted: with `SecurityMode::Auto` the cipher we end up with is
        // partly the *server's* choice (OpenSec, falling back to VIE), so a remote peer
        // can decide whether this fires — and in this repo network data never panics
        // the client. The send goes ahead; the zone's refusal is the real outcome and
        // this is the line that explains it.
        if let Some(complaint) =
            login_form_mismatch(self.conn.cipher_kind(), form.kind == c2s::kind::CONT_LOGIN)
        {
            log::error!(
                "{complaint} (cipher {}); the zone will drop this login",
                self.conn.cipher_name().unwrap_or("?")
            );
        }
        let login = Login {
            kind: form.kind,
            flags: u8::from(self.new_user_sent),
            name: FixedStr::new(self.cfg.name.as_bytes()),
            password: FixedStr::new(self.cfg.password.as_bytes()),
            machine_id: self.cfg.machine_id,
            connect_type: 4,
            timezone_bias: self.cfg.timezone_bias,
            unknown1: 0,
            client_version: form.version,
            field444: 444,
            // Zero, as a real client sends for an `INADDR_ANY` bind -- not the
            // 555 this used to carry, which is not an address.
            bound_ip: 0,
            // A real client sends zero here. This used to repeat `machine_id`,
            // and it is the one field in this range ASSS reads (`core.c`,
            // `p->permid = pkt->D2`), where it feeds the alias database.
            permission_id: 0,
            server_ip: self.endpoint.0,
            bound_port: self.endpoint.1,
            redirect_id: self.cfg.redirect_id,
            using_proxy: 0,
            unused: 0,
            cont_id: form.continuum_id,
        };
        self.send(&login, true);
    }

    fn go_arena(&mut self) {
        let (arena_type, arena) = match &self.cfg.arena {
            Some(a) if !a.is_empty() => (c2s::arena_type::NAMED, FixedStr::new(a.as_bytes())),
            _ => (c2s::arena_type::ANY_PUBLIC, FixedStr::default()),
        };
        let go = GoArena {
            ship: self.cfg.ship.min(SPECTATOR),
            allow_audio: 0,
            obscenity_filter: 0,
            x_res: self.cfg.resolution.0 as i16,
            y_res: self.cfg.resolution.1 as i16,
            arena_type,
            arena,
            optional_graphics: self.optional_graphics(),
        };
        self.send(&go, true);
        self.state = State::EnteringArena;
    }

    fn handle_packet(&mut self, now_ms: u64, data: &[u8]) {
        if let Some(r) = self.recorder.as_mut() {
            r.server_packet(now_ms, data);
        }
        let packet = match S2CPacket::decode(data) {
            Ok(p) => p,
            Err(e) => {
                log::debug!(
                    "bad S2C packet {:#04x}: {e}",
                    data.first().copied().unwrap_or(0)
                );
                return;
            }
        };
        match packet {
            S2CPacket::LoginText(t) => {
                let text = String::from_utf8_lossy(&t.text).trim().to_string();
                log::info!("the zone's login text: {text}");
                self.login_text = (!text.is_empty()).then_some(text);
            }
            S2CPacket::LoginResponse(r) => {
                self.news_checksum = r.news_checksum;
                let zone_text = self.login_text.take();
                // [`login_code::NO_SCORES`] is a *success*: the zone let you in, but its
                // billing server is down so nothing you score will be kept. Continuum
                // enters and says so, and a zone whose biller is off is otherwise
                // unreachable -- a billerless subgame answers every login with it.
                // Entering silently would hide that from the player, so say it the way
                // the zone would: a green arena message.
                //
                // [`login_code::SPEC_ONLY`] is a success too: ASSS counts it with
                // the other two (`AUTH_IS_OK` in its `core.h`), and the player
                // enters to watch. This client refused it, so such a zone could
                // not be entered at all.
                // A zone that does not know the name offers to create it, and the
                // connection stays up for the answer: see `create_account`. Not
                // while playing a recording back, which holds the zone's side
                // only -- its next answer is already in it.
                if r.code == login_code::NEW_USER && !self.new_user_sent {
                    if self.playback {
                        return;
                    }
                    self.awaiting_new_user = true;
                    if self.cfg.new_user {
                        self.create_account();
                    } else {
                        self.events.push_back(SessionEvent::NewUserRequired {
                            name: self.cfg.name.clone(),
                            zone_text,
                        });
                    }
                    return;
                }
                if let Some(form) = login_admits(r.code, r.demo_data, self.cfg.name.is_empty()) {
                    self.events.push_back(SessionEvent::LoggedIn);
                    let notice = match r.code {
                        login_code::NO_SCORES => Some("Scores are not available in this zone."),
                        login_code::SPEC_ONLY => {
                            Some("This zone lets you watch only: you cannot fly here.")
                        }
                        _ => None,
                    };
                    for text in notice.map(str::to_string).into_iter().chain(zone_text) {
                        self.events.push_back(SessionEvent::Chat {
                            kind: chat_type::ARENA,
                            pid: -1,
                            name: None,
                            text,
                            sound: 0,
                        });
                    }
                    // The form is answered on the connecting screen, so the
                    // arena waits for it; a recording has nobody to ask.
                    if form && !self.playback {
                        self.awaiting_registration = true;
                        self.events.push_back(SessionEvent::RegistrationWanted);
                    } else {
                        self.go_arena();
                    }
                } else {
                    self.events.push_back(SessionEvent::LoginFailed {
                        code: r.code,
                        message: login_message(r.code),
                        zone_text,
                    });
                    self.conn.disconnect();
                }
            }
            S2CPacket::PlayerId(p) => {
                // A new arena: forget the old one.
                self.pid = Some(p.pid);
                self.players.clear();
                self.entering_done = false;
                self.last_position_ticks = None;
                self.events
                    .push_back(SessionEvent::EnteredArena { pid: p.pid });
            }
            S2CPacket::Settings(cs) => {
                self.settings_raw = data.to_vec();
                self.settings = Some(cs.clone());
                self.events.push_back(SessionEvent::Settings(cs));
            }
            S2CPacket::PlayerEntering(pe) => {
                for d in pe.players {
                    let p = Player {
                        pid: d.pid,
                        name: fixed_to_string(d.name.as_bytes()),
                        squad: fixed_to_string(d.squad.as_bytes()),
                        ship: (d.ship as u8).min(SPECTATOR),
                        freq: d.freq,
                        kill_points: d.kill_points,
                        flag_points: d.flag_points,
                        wins: d.wins,
                        losses: d.losses,
                        flags: d.flags_carried,
                        position: None,
                    };
                    let pid = p.pid;
                    self.players.insert(pid, p);
                    self.events.push_back(SessionEvent::PlayerEntered(pid));
                }
            }
            S2CPacket::PlayerLeaving(p) => {
                self.players.remove(&p.pid);
                self.events.push_back(SessionEvent::PlayerLeft(p.pid));
            }
            S2CPacket::MapFilename(m) => {
                let (name, checksum) = match &m {
                    MapFilename::Single { name, checksum } => {
                        (fixed_to_string(name.as_bytes()), *checksum)
                    }
                    MapFilename::Files(files) => match files.first() {
                        Some(f) => (fixed_to_string(f.name.as_bytes()), f.checksum),
                        None => return,
                    },
                };
                self.files = match &m {
                    MapFilename::Files(files) => files
                        .iter()
                        .enumerate()
                        .skip(1)
                        .map(|(i, f)| ArenaFile {
                            index: i.min(u16::MAX as usize) as u16,
                            name: fixed_to_string(f.name.as_bytes()),
                            checksum: f.checksum,
                            size: f.size,
                        })
                        .collect(),
                    MapFilename::Single { .. } => Vec::new(),
                };
                self.downloads.clear();
                self.map = Some(MapInfo {
                    name: name.clone(),
                    checksum,
                    tiles: None,
                });
                if self.state == State::EnteringArena || self.state == State::Playing {
                    self.state = State::WaitingForMap;
                }
                self.events
                    .push_back(SessionEvent::MapNeeded { name, checksum });
                if !self.files.is_empty() {
                    self.events
                        .push_back(SessionEvent::FilesListed(self.files.clone()));
                }
            }
            S2CPacket::IncomingFile(f) => self.on_news(&f.name, &f.data),
            S2CPacket::MapData(m) => {
                let name = fixed_to_string(m.name.as_bytes());
                let is_map = self
                    .map
                    .as_ref()
                    .is_some_and(|i| i.name.eq_ignore_ascii_case(&name));
                if !is_map && let Some(f) = self.file_named(&name).cloned() {
                    self.on_file_data(&f, m.data);
                } else {
                    self.on_map_data(&m.data);
                }
                self.downloading = None;
                self.next_download();
            }
            S2CPacket::FlagLocation(f) => self.events.push_back(SessionEvent::FlagLocation {
                id: f.flag_id,
                x: f.x,
                y: f.y,
                freq: f.freq,
            }),
            S2CPacket::FlagPickup(f) => {
                let id = f.flag_id as u16;
                // Carried (not turf) flags count toward the carrier's total.
                let carry = self.settings.as_ref().is_some_and(|s| s.carry_flags() > 0);
                if carry
                    && !self.turf_flags.contains(&id)
                    && let Some(p) = self.players.get_mut(&f.pid)
                {
                    p.flags = p.flags.saturating_add(1);
                }
                self.events
                    .push_back(SessionEvent::FlagPickup { id, pid: f.pid });
            }
            S2CPacket::FlagDrop(f) => {
                if let Some(p) = self.players.get_mut(&f.pid) {
                    p.flags = 0;
                }
                self.events
                    .push_back(SessionEvent::FlagsDropped { pid: f.pid });
            }
            S2CPacket::FlagVictory(v) => {
                for p in self.players.values_mut() {
                    p.flags = 0;
                }
                self.events.push_back(SessionEvent::FlagVictory {
                    freq: v.freq,
                    points: v.points,
                });
            }
            S2CPacket::TurfFlags(t) => {
                self.turf_flags = (0..t.owners.len().min(u16::MAX as usize) as u16).collect();
                self.events.push_back(SessionEvent::TurfFlags(t.owners));
            }
            S2CPacket::PeriodicReward(r) => self.events.push_back(SessionEvent::FlagReward(
                r.rewards.iter().map(|r| (r.freq, r.points)).collect(),
            )),
            S2CPacket::Ball(b) => self.events.push_back(SessionEvent::Ball(b)),
            S2CPacket::SoccerGoal(g) => self.events.push_back(SessionEvent::Goal {
                freq: g.freq,
                points: g.points,
            }),
            S2CPacket::Turret(t) => self.events.push_back(SessionEvent::Attached {
                pid: t.pid,
                host: t.host,
            }),
            S2CPacket::TurretKickoff(t) => self
                .events
                .push_back(SessionEvent::TurretsDetached { pid: t.pid }),
            S2CPacket::Bricks(b) => self.events.push_back(SessionEvent::Bricks(b.bricks)),
            S2CPacket::Koth(k) => self.events.push_back(SessionEvent::Crown {
                on: k.action != 0,
                ticks: k.time,
                pid: k.pid,
            }),
            S2CPacket::SetKothTimer(t) => self.events.push_back(SessionEvent::CrownTimer {
                ticks: t.time,
                add: false,
            }),
            S2CPacket::AddKothTime(t) => self.events.push_back(SessionEvent::CrownTimer {
                ticks: t.time,
                add: true,
            }),
            S2CPacket::ToggleObjects(t) => self.events.push_back(SessionEvent::ObjectsToggled(
                t.toggles.iter().map(|t| (t.id(), t.on())).collect(),
            )),
            S2CPacket::MoveObjects(m) => self
                .events
                .push_back(SessionEvent::ObjectsChanged(m.changes)),
            S2CPacket::EnteringArena(_) => {
                self.entering_done = true;
                self.maybe_playing();
            }
            S2CPacket::Security(s) => self.on_security(now_ms, &s),
            S2CPacket::Position(p) => {
                let now_ticks = self.server_ticks(now_ms);
                let pid = p.player_id as i16;
                let energy = match p.tail {
                    oss_proto::PositionTail::Energy(e) => Some(e),
                    oss_proto::PositionTail::Extra(x) => Some(x.energy),
                    oss_proto::PositionTail::None => None,
                };
                self.update_position(
                    pid,
                    RemotePosition {
                        x: p.x,
                        y: p.y,
                        x_speed: p.x_speed,
                        y_speed: p.y_speed,
                        rotation: (p.rotation as u8) % 40,
                        status: p.status,
                        bounty: p.bounty as u16,
                        energy,
                        server_ticks: extend_ticks(now_ticks, p.time),
                        weapon: None,
                    },
                );
            }
            S2CPacket::Weapon(w) => {
                self.weapons_received = self.weapons_received.wrapping_add(1);
                let now_ticks = self.server_ticks(now_ms);
                self.update_position(
                    w.player_id as i16,
                    RemotePosition {
                        x: w.x,
                        y: w.y,
                        x_speed: w.x_speed,
                        y_speed: w.y_speed,
                        rotation: (w.rotation as u8) % 40,
                        status: w.status,
                        bounty: w.bounty,
                        energy: None,
                        server_ticks: extend_ticks(now_ticks, w.time),
                        weapon: (w.weapon.kind() != 0).then_some(w.weapon),
                    },
                );
            }
            S2CPacket::ShipChange(c) => {
                let ship = (c.ship as u8).min(SPECTATOR);
                if let Some(p) = self.players.get_mut(&c.pid) {
                    p.ship = ship;
                    p.freq = c.freq;
                    p.position = None;
                }
                self.events.push_back(SessionEvent::ShipChanged {
                    pid: c.pid,
                    ship,
                    freq: c.freq,
                });
            }
            S2CPacket::FreqChange(c) => {
                if let Some(p) = self.players.get_mut(&c.pid) {
                    p.freq = c.freq;
                }
                self.events.push_back(SessionEvent::FreqChanged {
                    pid: c.pid,
                    freq: c.freq,
                });
            }
            S2CPacket::Kill(k) => {
                // Score the kill the way Continuum clients do: the killer gets the
                // bounty as points and a win, the killed a loss and drops its flags.
                if let Some(p) = self.players.get_mut(&k.killed) {
                    p.losses = p.losses.saturating_add(1);
                    p.flags = 0;
                }
                if let Some(p) = self.players.get_mut(&k.killer) {
                    p.wins = p.wins.saturating_add(1);
                    p.kill_points = p.kill_points.saturating_add(k.bounty.max(0) as i32);
                    if k.killer != k.killed {
                        p.flags = p.flags.saturating_add(k.flags.max(0));
                    }
                }
                self.events.push_back(SessionEvent::Kill {
                    killer: k.killer,
                    killed: k.killed,
                    bounty: k.bounty,
                    green: k.green,
                })
            }
            S2CPacket::ScoreUpdate(u) => {
                if let Some(p) = self.players.get_mut(&u.pid) {
                    p.kill_points = u.kill_points;
                    p.flag_points = u.flag_points;
                    p.wins = u.kills;
                    p.losses = u.deaths;
                }
            }
            S2CPacket::ScoreReset(r) => {
                for p in self.players.values_mut() {
                    if r.pid == -1 || r.pid == p.pid {
                        (p.kill_points, p.flag_points, p.wins, p.losses) = (0, 0, 0, 0);
                    }
                }
            }
            S2CPacket::Chat(c) => {
                let ChatBody {
                    kind,
                    sound,
                    pid,
                    text,
                } = c.0;
                let name = self.players.get(&pid).map(|p| p.name.clone());
                let text = String::from_utf8_lossy(&text).into_owned();
                self.events.push_back(SessionEvent::Chat {
                    kind,
                    pid,
                    name,
                    text,
                    sound,
                });
            }
            S2CPacket::WarpTo(w) => self
                .events
                .push_back(SessionEvent::WarpTo { x: w.x, y: w.y }),
            S2CPacket::ShipReset(_) => self.events.push_back(SessionEvent::ShipReset),
            S2CPacket::PrizeReceive(p) => self.events.push_back(SessionEvent::PrizeReceived {
                count: p.count,
                prize: p.prize,
            }),
            S2CPacket::Green(g) => self.events.push_back(SessionEvent::GreenTaken {
                pid: g.pid,
                x: g.x,
                y: g.y,
                prize: g.prize,
            }),
            S2CPacket::KeepAlive(_) => {}
            S2CPacket::ArenaList(list) => {
                let arenas = list
                    .arenas
                    .iter()
                    .map(|a| ArenaListed {
                        name: fixed_to_string(&a.name),
                        players: a.population.unsigned_abs(),
                        yours: a.population < 0,
                    })
                    .collect();
                self.events.push_back(SessionEvent::ArenaList(arenas));
            }
            S2CPacket::Redirect(r) => self.on_redirect(&r),
            other => self.events.push_back(SessionEvent::Other(Box::new(other))),
        }
    }

    fn update_position(&mut self, pid: i16, pos: RemotePosition) {
        if let Some(p) = self.players.get_mut(&pid) {
            // Drop out-of-order updates.
            if let Some(old) = p.position
                && (pos.server_ticks.wrapping_sub(old.server_ticks) as i32) < 0
            {
                return;
            }
            p.position = Some(pos);
            self.events.push_back(SessionEvent::PlayerMoved(pid));
        }
    }

    fn maybe_playing(&mut self) {
        let map_ready = self.map.as_ref().is_none_or(|m| m.tiles.is_some());
        if self.entering_done
            && map_ready
            && self.state != State::Playing
            && self.state != State::Closed
        {
            self.state = State::Playing;
            self.events.push_back(SessionEvent::Playing);
        }
    }

    /// S2C `0x3B`: the zone sends the player on to another zone. Hang up on this
    /// one and say where to go; the caller connects (see
    /// [`SessionEvent::Redirected`]). A recording played back reports it and
    /// carries on, since what came after is already in the recording.
    fn on_redirect(&mut self, r: &s2c::Redirect) {
        let Some((addr, arena)) = redirect_target(r) else {
            log::warn!(
                "the zone sent a redirect with no usable address ({}.{}.{}.{} port {}); ignored",
                r.ip[0],
                r.ip[1],
                r.ip[2],
                r.ip[3],
                r.port
            );
            return;
        };
        log::info!(
            "the zone redirects to {addr}, {}",
            arena.as_deref().map_or_else(
                || "any public arena".to_string(),
                |a| format!("arena {a:?}")
            )
        );
        if !self.playback {
            self.disconnect();
        }
        self.events.push_back(SessionEvent::Redirected {
            server: addr.to_string(),
            arena,
            login_id: r.login_id,
        });
    }

    /// Offers a cached copy of the map named in [`SessionEvent::MapNeeded`]. Returns
    /// false (and changes nothing) if it doesn't match the server's checksum.
    pub fn use_cached_map(&mut self, data: &[u8]) -> bool {
        let Some(info) = self.map.as_ref() else {
            return false;
        };
        if checksum::crc32(data) != info.checksum {
            return false;
        }
        let name = info.name.clone();
        self.set_map(name, data.to_vec(), false);
        true
    }

    /// Asks the server for the map named in [`SessionEvent::MapNeeded`].
    pub fn download_map(&mut self) {
        self.request_download(0);
    }

    /// A map or file transfer is running or queued. Servers put a player who
    /// requests a file into spectator mode, so take a ship only after this is false.
    pub fn downloads_pending(&self) -> bool {
        self.downloading.is_some() || !self.downloads.is_empty()
    }

    /// The extra files (LVZ) the arena listed.
    pub fn files(&self) -> &[ArenaFile] {
        &self.files
    }

    fn file_named(&self, name: &str) -> Option<&ArenaFile> {
        self.files
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// Offers a cached copy of extra file `name`; accepted (and reported as
    /// [`SessionEvent::FileReady`]) when its CRC matches the server's.
    pub fn use_cached_file(&mut self, name: &str, data: &[u8]) -> bool {
        let Some(f) = self.file_named(name) else {
            return false;
        };
        if checksum::crc32(data) != f.checksum {
            return false;
        }
        let name = f.name.clone();
        if let Some(r) = self.recorder.as_mut() {
            r.file(self.last_now_ms, &name, data);
        }
        self.events.push_back(SessionEvent::FileReady {
            name,
            data: data.to_vec(),
            downloaded: false,
        });
        true
    }

    /// CRC-32 of the zone's news file, from the login reply; 0 means the zone has
    /// no news. Continuum compares this with its cached `news.txt` and only then
    /// asks (mervbot `clientprot.cpp`, which does the same).
    pub fn news_checksum(&self) -> u32 {
        self.news_checksum
    }

    /// Asks for the zone's news file (C2S `0x0D`). It arrives as an S2C `0x10`
    /// with an all-zero name, zlib-compressed, and is reported as
    /// [`SessionEvent::News`]. Does nothing when the zone has no news.
    pub fn request_news(&mut self) {
        if self.news_checksum == 0 || self.news_pending {
            return;
        }
        self.news_pending = true;
        self.send(&c2s::NewsRequest {}, true);
    }

    /// Asks for the news file only when the zone has one and `cached` (this
    /// zone's last `news.txt`, if kept) does not already match its checksum. This
    /// is what Continuum does, and mervbot with it.
    pub fn request_news_if_new(&mut self, cached: Option<&[u8]>) {
        if self.news_checksum == 0 {
            return;
        }
        if cached.is_some_and(|d| checksum::crc32(d) == self.news_checksum) {
            return;
        }
        self.request_news();
    }

    /// Changes arena, as Continuum's `?go` does: no argument (or an empty one) for
    /// any public arena, a number for that public arena, any other text for a named
    /// arena. nullspace `ChatController::HandleInputCommands`.
    pub fn change_arena(&mut self, arena: &str) {
        let arena = arena.trim();
        let (arena_type, name) = arena_target(arena);
        self.cfg.arena = (!arena.is_empty()).then(|| arena.to_string());
        let ship = self.me().map_or(self.cfg.ship, |p| p.ship).min(SPECTATOR);
        let go = GoArena {
            ship,
            allow_audio: 0,
            obscenity_filter: 0,
            x_res: self.cfg.resolution.0 as i16,
            y_res: self.cfg.resolution.1 as i16,
            arena_type,
            arena: name,
            optional_graphics: self.optional_graphics(),
        };
        self.send(&go, true);
        self.state = State::EnteringArena;
    }

    /// The arena-entry packet's optional-graphics byte, chosen by the kind of
    /// connection (see [`SessionConfig::want_lvz`] for what the byte does).
    ///
    /// - **Continuum-encrypted**: `0`. The zone already lists a Continuum client
    ///   every file it needs; the byte would add the ones the arena marks
    ///   optional, which real Continuum is never sent. A player there downloaded
    ///   3.3 MB entering one arena, more than Continuum would have.
    /// - **Any other kind**: `1`, because for a client the zone does not take for
    ///   Continuum the byte is the only way to be offered any graphics at all.
    /// - **Not wanted at all** (a bot): no byte, the shorter form of the packet.
    fn optional_graphics(&self) -> Option<u8> {
        if !self.cfg.want_lvz {
            return None;
        }
        let continuum = matches!(self.conn.cipher_kind(), Some(CipherKind::Continuum));
        Some(u8::from(!continuum))
    }

    /// Downloads extra file `name`. Transfers run one at a time, after the map.
    pub fn download_file(&mut self, name: &str) {
        if let Some(index) = self.file_named(name).map(|f| f.index) {
            self.request_download(index);
        }
    }

    fn request_download(&mut self, index: u16) {
        if self.downloads.contains(&index) {
            return;
        }
        self.downloads.push_back(index);
        self.next_download();
    }

    /// What the transfer in flight is fetching: index 0 is the arena's map, anything
    /// else one of the files it listed.
    fn transfer_name(&self) -> String {
        match self.downloading {
            Some(0) => self
                .map
                .as_ref()
                .map(|m| m.name.clone())
                .unwrap_or_default(),
            Some(i) => self
                .files
                .iter()
                .find(|f| f.index == i)
                .map(|f| f.name.clone())
                .unwrap_or_default(),
            None => String::new(),
        }
    }

    fn next_download(&mut self) {
        if self.downloading.is_some() {
            return;
        }
        if let Some(index) = self.downloads.pop_front() {
            self.downloading = Some(index);
            let index = (index != 0).then_some(index);
            self.send(&MapRequest { index }, true);
        }
    }

    /// S2C `0x10`. An all-zero name is the news file; anything else is a file
    /// transfer this client never starts, so it is only logged.
    fn on_news(&mut self, name: &FixedStr<16>, data: &[u8]) {
        let name = fixed_to_string(name.as_bytes());
        if !name.is_empty() {
            log::debug!("ignoring an incoming file we did not ask for: {name}");
            return;
        }
        self.news_pending = false;
        let text = match map::inflate(data) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("the zone's news file is corrupt: {e}");
                return;
            }
        };
        // News is plain text; zones write it in the Windows code page, so anything
        // that is not UTF-8 is read as Latin-1 rather than dropped.
        let text = match String::from_utf8(text) {
            Ok(t) => t,
            Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
        };
        self.events.push_back(SessionEvent::News(text));
    }

    fn on_file_data(&mut self, file: &ArenaFile, data: Vec<u8>) {
        if checksum::crc32(&data) != file.checksum {
            log::warn!(
                "{} checksum differs from the server's; using it anyway",
                file.name
            );
        }
        if let Some(r) = self.recorder.as_mut() {
            r.file(self.last_now_ms, &file.name, &data);
        }
        self.events.push_back(SessionEvent::FileReady {
            name: file.name.clone(),
            data,
            downloaded: true,
        });
    }

    fn on_map_data(&mut self, compressed: &[u8]) {
        let Some(info) = self.map.as_ref() else {
            return;
        };
        let data = match map::inflate(compressed) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("map download for {} is corrupt: {e}", info.name);
                return;
            }
        };
        if checksum::crc32(&data) != info.checksum {
            log::warn!(
                "map {} checksum differs from the server's; using it anyway",
                info.name
            );
        }
        let name = info.name.clone();
        self.set_map(name, data, true);
    }

    fn set_map(&mut self, name: String, data: Vec<u8>, downloaded: bool) {
        let tiles = match oss_formats::Level::parse(&data) {
            Ok(level) => level.tile_grid().cells().to_vec(),
            Err(e) => {
                log::warn!("map {name} doesn't parse ({e}); using an empty map");
                vec![0; 1024 * 1024]
            }
        };
        if let Some(info) = self.map.as_mut() {
            info.tiles = Some(tiles);
        }
        if let Some(r) = self.recorder.as_mut() {
            r.file(self.last_now_ms, &name, &data);
        }
        self.events.push_back(SessionEvent::MapReady {
            name,
            data,
            downloaded,
        });
        if let Some(key) = self.pending_security.take() {
            self.answer_security(self.last_now_ms, key);
        }
        self.maybe_playing();
    }

    fn on_security(&mut self, now_ms: u64, s: &s2c::Security) {
        self.events.push_back(SessionEvent::Seeds {
            green_seed: s.green_seed,
            door_seed: s.door_seed,
            timestamp: s.timestamp,
        });
        if s.key == 0 {
            return;
        }
        let map_loading = self.map.as_ref().is_some_and(|m| m.tiles.is_none());
        if map_loading {
            self.pending_security = Some(s.key);
        } else {
            self.answer_security(now_ms, s.key);
        }
    }

    fn answer_security(&mut self, now_ms: u64, key: u32) {
        let settings_checksum = if self.settings_raw.is_empty() {
            0
        } else {
            checksum::settings_checksum(key, &self.settings_raw)
        };
        let tiles = self.map.as_ref().and_then(|m| m.tiles.as_ref());
        let map_checksum = tiles
            .map(|t| {
                checksum::map_checksum(key, |x, y| {
                    if (0..1024).contains(&x) && (0..1024).contains(&y) {
                        t[y as usize * 1024 + x as usize]
                    } else {
                        0
                    }
                })
            })
            .unwrap_or(0);
        // The profile supplies only what depends on the client binary; everything
        // above and below it is the same for either client.
        let answer = self.cfg.client.security(SecurityChallenge {
            key,
            map_crc32: tiles.map(|t| checksum::map_crc32(t)),
        });
        let ping = (self.conn.rtt_ms() / 10).min(u16::MAX as u64) as u16;
        let stats = self.conn.stats();
        let resp = SecurityResponse {
            weapon_count: self.weapons_received,
            settings_checksum,
            exe_checksum: answer.exe_checksum,
            map_checksum,
            s2c_slow_total: 0,
            s2c_fast_total: stats.datagrams_received.min(u32::MAX as u64) as u32,
            s2c_slow_current: 0,
            s2c_fast_current: 0,
            s2c_reliable_out: 0,
            ping_last: ping,
            ping_average: ping,
            ping_low: ping,
            ping_high: ping,
            slow_frame: 0,
            continuum: answer.continuum,
        };
        self.send(&resp, true);
        self.flush(now_ms);
    }

    /// Ticks between position packets for your current ship (Continuum sends every
    /// `SendPositionDelay` ticks while playing, once a second as a spectator).
    pub fn position_interval(&self) -> u32 {
        let ship = self.me().map(|p| p.ship).unwrap_or(SPECTATOR);
        if ship == SPECTATOR {
            return 100;
        }
        self.settings
            .as_ref()
            .map(|s| s.send_position_delay().max(5) as u32)
            .unwrap_or(20)
    }

    /// Whether a regular position update is due.
    pub fn position_due(&self, now_ms: u64) -> bool {
        if self.state != State::Playing {
            return false;
        }
        let now = self.server_ticks(now_ms);
        // Signed, because the last stamp can be *ahead* of the clock: when two
        // packets fall in one tick `send_position` stamps the second one a tick
        // later, which a weapon fired just after a routine update does all the
        // time. Read unsigned, that one-tick lead is a difference of four
        // thousand million, so every later call says "due" and the client sends a
        // position packet per frame for the rest of the session.
        self.last_position_ticks
            .is_none_or(|t| (now.wrapping_sub(t) as i32) >= self.position_interval() as i32)
    }

    /// Sends your position (also used to fire weapons: set `pos.weapon`).
    pub fn send_position(&mut self, now_ms: u64, pos: &OwnPosition) {
        let mut time = self.server_ticks(now_ms);
        // The server drops updates that aren't newer than the last one.
        if let Some(last) = self.last_position_ticks
            && (time.wrapping_sub(last) as i32) <= 0
        {
            time = last.wrapping_add(1);
        }
        self.last_position_ticks = Some(time);
        let (x, y) = if pos.dead { (-1, -1) } else { (pos.x, pos.y) };
        let packet = C2SPosition {
            rotation: (pos.rotation % 40) as i8,
            time,
            x_speed: pos.x_speed,
            y,
            checksum: 0,
            status: pos.status,
            x,
            y_speed: pos.y_speed,
            bounty: pos.bounty,
            energy: pos.energy,
            weapon: pos.weapon,
            extra: None,
        };
        // Weapon packets matter more than plain movement: send those reliably.
        self.send(&packet, pos.weapon.kind() != 0);
        self.flush(now_ms);
    }

    /// Sends a chat message. `target` is the player id for private messages.
    pub fn send_chat(&mut self, kind: u8, target: i16, text: &str) {
        let body = ChatBody {
            kind,
            sound: 0,
            pid: target,
            text: text.as_bytes().to_vec(),
        };
        self.send(&c2s::C2SChat(body), true);
    }

    /// Asks the server to place a brick at tile (x, y) (C2S `0x1C`).
    pub fn send_brick(&mut self, x: i16, y: i16) {
        self.send(&c2s::DropBrick { x, y }, true);
    }

    /// Reports that you picked up the green on tile (x, y) (C2S `0x07`). Reliable
    /// when the arena's `TakePrizeReliable` says so.
    pub fn send_take_green(&mut self, now_ms: u64, x: u16, y: u16, prize: i32) {
        let reliable = self
            .settings()
            .is_some_and(|cs| cs.take_prize_reliable() != 0);
        let time = self.server_ticks(now_ms);
        self.send(
            &c2s::TakeGreen {
                time,
                x: x.min(i16::MAX as u16) as i16,
                y: y.min(i16::MAX as u16) as i16,
                prize: prize.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            },
            reliable,
        );
    }

    /// Asks to pick up (or claim) flag `id` (C2S `0x13`).
    pub fn pickup_flag(&mut self, id: u16) {
        self.send(&c2s::PickupFlag { flag_id: id as i16 }, true);
    }

    /// Drops every flag you carry (C2S `0x15`).
    pub fn drop_flags(&mut self) {
        self.send(&c2s::DropFlags {}, true);
    }

    /// Attaches to `pid` as a turret, or detaches with -1 (C2S `0x10`).
    pub fn attach(&mut self, pid: i16) {
        self.send(&c2s::AttachTo { pid }, true);
    }

    /// Detaches everyone attached to you (C2S `0x14`).
    pub fn detach_turrets(&mut self) {
        self.send(&c2s::TurretKickoff {}, true);
    }

    /// Asks to pick up ball `id` (C2S `0x20`); `time` is the ball's last update time.
    pub fn pickup_ball(&mut self, id: u8, time: u32) {
        self.send(&c2s::PickupBall { ball_id: id, time }, true);
    }

    /// Shoots the ball you carry (C2S `0x1F`): pixels and pixels per 10 s.
    pub fn shoot_ball(&mut self, now_ms: u64, id: u8, pos: (i16, i16), vel: (i16, i16)) {
        let pid = self.pid().unwrap_or(-1);
        let time = self.server_ticks(now_ms);
        self.send(
            &c2s::ShootBall {
                ball_id: id,
                x: pos.0,
                y: pos.1,
                x_speed: vel.0,
                y_speed: vel.1,
                pid,
                time,
            },
            true,
        );
    }

    /// Reports ball `id` entering a goal at tile (x, y) (C2S `0x21`).
    pub fn goal(&mut self, id: u8, x: i16, y: i16) {
        self.send(&c2s::Goal { ball_id: id, x, y }, true);
    }

    /// Reports that your crown timer ran out (C2S `0x1E`).
    pub fn crown_expired(&mut self) {
        self.send(&c2s::KothExpired {}, true);
    }

    /// Asks for player `pid`'s extra data while spectating them (C2S `0x08`); -1
    /// stops.
    pub fn spectate(&mut self, pid: i16) {
        self.send(&c2s::SpecRequest { pid }, true);
    }

    /// Reports your death (C2S `0x05`).
    pub fn send_death(&mut self, killer: i16, bounty: u16) {
        self.send(
            &c2s::Death {
                killer,
                bounty: bounty.min(i16::MAX as u16) as i16,
            },
            true,
        );
    }

    pub fn set_ship(&mut self, ship: u8) {
        self.send(
            &SetShip {
                ship: ship.min(SPECTATOR),
            },
            true,
        );
    }

    pub fn set_freq(&mut self, freq: i16) {
        self.send(&SetFreq { freq }, true);
    }
}

/// Extends a 16-bit server timestamp to the full tick count nearest `now`.
fn extend_ticks(now: u32, low: u16) -> u32 {
    let candidate = (now & !0xFFFF) | low as u32;
    let diff = candidate.wrapping_sub(now) as i32;
    if diff > 0x8000 {
        candidate.wrapping_sub(0x1_0000)
    } else if diff < -0x8000 {
        candidate.wrapping_add(0x1_0000)
    } else {
        candidate
    }
}

/// Where an S2C `0x3B` points: the zone's address and the arena to enter there
/// (`None` for any public arena). `None` when there is no address to go to.
///
/// **The address is the four bytes reversed**, `d c b a` on the wire for
/// `a.b.c.d`: the dotted address read as a big-endian number and written
/// little-endian. That is what Subspace Server .NET sends
/// (`Packets/Game/S2C_Redirect.cs` with `Modules/Redirect.cs` `TryGetIPv4`),
/// and what a live zone was seen to send, checked against the address the
/// directory servers list for the zone it pointed at. It is also the order the
/// login packet's own server address goes out in (see `udp.rs`). ASSS 1.4.4's
/// `core/redirect.c` writes `s_addr` instead, which is network order, the other
/// way round; the live zone settles it.
///
/// The arena follows the arena-entry packet's types: a name when one is sent, a
/// public arena's number when the type is one (0 and up), otherwise any public
/// arena.
fn redirect_target(r: &s2c::Redirect) -> Option<(std::net::SocketAddrV4, Option<String>)> {
    let [d, c, b, a] = r.ip;
    let ip = std::net::Ipv4Addr::new(a, b, c, d);
    if ip.is_unspecified() || r.port == 0 {
        return None;
    }
    let name = fixed_to_string(r.arena.as_bytes());
    let arena = if !name.is_empty() {
        Some(name)
    } else if r.arena_type >= 0 {
        Some(r.arena_type.to_string())
    } else {
        None
    };
    Some((std::net::SocketAddrV4::new(ip, r.port), arena))
}

fn fixed_to_string(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim_end().to_string()
}

/// How Continuum encodes a `?go` argument: nothing is any public arena, digits are
/// that public arena's number in the type field, anything else is a named arena
/// (nullspace `ChatController::HandleInputCommands`).
fn arena_target(arena: &str) -> (i16, FixedStr<16>) {
    if arena.is_empty() {
        (c2s::arena_type::ANY_PUBLIC, FixedStr::default())
    } else if let Ok(n) = arena.parse::<u16>() {
        (n.min(i16::MAX as u16) as i16, FixedStr::default())
    } else {
        (c2s::arena_type::NAMED, FixedStr::new(arena.as_bytes()))
    }
}

/// Human-readable login failure.
/// Whether a login form and the cipher it will be sent under disagree, and what to
/// say about it.
///
/// A zone picks the login form it expects from the **encryption**, not from the
/// packet's type byte — ASSS `core.c`'s `PLogin` is registered for both `0x09` and
/// `0x24` and checks the length against `p->type`, which `net.c` takes from the
/// encryption module. oss-server does the same: its transport classifies a connection
/// from the conn-init byte and `modules/core.rs` then holds a VIE client to
/// [`Login::VIE_LEN`] and a Continuum one to `Login::CONT_LEN`.
///
/// `None` means "cannot tell", which is not the same as agreement:
/// [`CipherKind`] is `#[non_exhaustive]` and [`CipherKind::Other`] is the `Cipher`
/// trait's default, so an external cipher that does not override `kind()` turns this
/// check off rather than failing it.
fn login_form_mismatch(cipher: Option<CipherKind>, continuum_form: bool) -> Option<&'static str> {
    match (cipher?, continuum_form) {
        // A VIE handshake that ended unencrypted is still the VIE module as far as the
        // zone is concerned, and an OpenSec client hello is a `00 01` packet, which
        // oss-server classifies as a VIE client. All three want the SubSpace form.
        (CipherKind::Vie | CipherKind::None | CipherKind::OpenSec, true) => {
            Some("Continuum login form on a connection that is not Continuum-encrypted")
        }
        (CipherKind::Continuum, false) => {
            Some("SubSpace login form on a Continuum-encrypted connection")
        }
        _ => None,
    }
}

/// Whether a login answer lets the player in. The three codes ASSS moves forward
/// on (`AUTH_IS_OK` in its `core.h`): a plain yes, a yes with no scores kept,
/// and a yes to watching only.
fn login_succeeded(code: u8) -> bool {
    matches!(
        code,
        login_code::OK | login_code::NO_SCORES | login_code::SPEC_ONLY
    )
}

/// Whether a login answer lets the player in and, if so, whether the zone
/// wants its registration form: `Some(form)` for in, `None` for refused.
///
/// The form is asked for by the reply's demographics byte. One code means two
/// things depending on the server, and this is where the two are told apart:
///
/// - **ASSS** sends `0x10` ([`login_code::NO_NAME`], `AUTH_NONAME` in `core.h`)
///   as a refusal, and from one place only: a login whose name is blank
///   (`core.c`, `PLogin`).
/// - **Subspace Server .NET** sends `0x10` (`AuthCode.AskDemographics`,
///   `IAuth.cs`) as a *success*, with the demographics byte set, when its
///   billing server wants the form (`BillingUdp.cs`). Its own login counts it
///   as OK.
///
/// So a `0x10` answering a login that carried a name cannot be ASSS's refusal,
/// and is read as .NET's yes-with-a-form. Read the other way, as it was, such
/// a .NET zone refused this client while letting every other one in. The byte
/// is not trusted to decide it: ASSS's refusal leaves it uninitialised
/// (`fail_login_with` fills only the code).
fn login_admits(code: u8, demo_data: u8, name_blank: bool) -> Option<bool> {
    if login_succeeded(code) {
        Some(demo_data != 0)
    } else if code == login_code::NO_NAME && !name_blank {
        Some(true)
    } else {
        None
    }
}

/// What a refusal code means, for the player, and whether trying again could
/// help.
///
/// Each one was checked against what ASSS means by it (its `core.h` names and
/// its `get_auth_code_msg`), and against what zones are seen to do with it. The
/// old labels were the codes' names in lower case -- "server busy" -- and that
/// one is what a billed zone answers when it has **locked a name out** after
/// repeated wrong passwords. A player told only "busy" tries again, which is
/// how a lockout of minutes became one of hours on 2026-11-17. So the wording
/// says what to do, and where trying again cannot help or can make it worse, it
/// says that first.
fn login_message(code: u8) -> &'static str {
    match code {
        // Reached only when the zone answers "unknown name" a second time, to a
        // login that already asked it to create the name: the first one is an
        // offer, `SessionEvent::NewUserRequired`.
        login_code::NEW_USER => {
            "The zone would not create an account with this name. Try another name, \
             or ask the zone's staff how to register one."
        }
        login_code::BAD_PASSWORD => {
            "Wrong password for this name. Check it before trying again: many zones \
             lock a name out after a few wrong tries."
        }
        login_code::FULL => "The zone is full. Try again later.",
        login_code::LOCKED => {
            "This name or this computer is banned from the zone. Trying again will not help."
        }
        login_code::NO_PERMISSION | login_code::NO_PERMISSION2 => {
            "The zone does not let this name in. Trying again will not help."
        }
        login_code::TOO_MANY_POINTS => {
            "This name has too many points for this zone. Trying again will not help."
        }
        login_code::TOO_SLOW => "The zone found the connection too slow. Try again later.",
        login_code::NO_NEW_CONNECTIONS => {
            "The zone is not accepting new players right now. Try again later."
        }
        login_code::BAD_NAME => {
            "The zone does not accept this name. Choose another; the same one will be \
             refused again."
        }
        login_code::OFFENSIVE_NAME => "The zone refused this name as offensive. Choose another.",
        login_code::SERVER_BUSY => {
            "The zone refused the login as busy. Billed zones also answer this way when \
             they have locked a name out after wrong passwords, so wait a while before \
             trying again: retrying can extend a lockout."
        }
        login_code::TOO_LOW_USAGE => "This name has not played long enough to enter this zone.",
        // Only for a blank name: with a name, `0x10` is a yes (`login_admits`).
        login_code::NO_NAME => "The zone received no name. Enter a name and try again.",
        login_code::TOO_MANY_DEMO | login_code::NO_DEMO => {
            "The zone does not admit this kind of client. Trying again will not help."
        }
        login_code::CUSTOM_TEXT => {
            "The zone refused the login and sent no reason. Wait before trying again."
        }
        _ => "The zone refused the login, with a code this client does not know.",
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::SessionConfig;

    /// This is the config handed to a [`Session`], so of all of them it is the one
    /// most likely to be logged while someone works out why a connection failed.
    #[test]
    fn the_session_config_cannot_print_its_password() {
        let cfg = SessionConfig {
            name: "Ace".into(),
            password: "hunter2-in-the-clear".into(),
            ..Default::default()
        };
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("hunter2"), "printed it: {shown}");
        assert!(shown.contains("<redacted>"), "and says so: {shown}");
        assert!(shown.contains("Ace"), "the rest is still readable: {shown}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_server_silence() {
        let mut s = Session::new(SessionConfig::default(), 1_000, 1);
        assert_eq!(s.ms_since_data(1_500), 500);
        s.handle_datagram(1_600, &[0, 0]);
        assert_eq!(s.ms_since_data(1_650), 50);
    }

    #[test]
    fn extends_small_timestamps() {
        assert_eq!(extend_ticks(0x0001_0005, 0x0003), 0x0001_0003);
        assert_eq!(extend_ticks(0x0001_0005, 0xFFF0), 0x0000_FFF0);
        assert_eq!(extend_ticks(0x0001_FFF0, 0x0005), 0x0002_0005);
    }

    #[test]
    fn names_trim_padding() {
        assert_eq!(fixed_to_string(b"Bob\0\0\0"), "Bob");
        assert_eq!(fixed_to_string(b"Alice   "), "Alice");
    }

    /// A weapon packet stamps the next tick when it shares a tick with the update
    /// before it, which leaves the last stamp ahead of the clock. The next regular
    /// update must still wait its turn instead of every call reporting "due".
    #[test]
    fn a_weapon_packet_does_not_start_a_position_flood() {
        let mut s = Session::new(SessionConfig::default(), 0, 1);
        s.state = State::Playing;
        s.players.insert(
            7,
            Player {
                pid: 7,
                name: "me".into(),
                squad: String::new(),
                ship: 0,
                freq: 0,
                kill_points: 0,
                flag_points: 0,
                wins: 0,
                losses: 0,
                flags: 0,
                position: None,
            },
        );
        s.pid = Some(7);
        let interval = s.position_interval();
        assert!(interval >= 5);
        let now = 10_000;
        let tick = s.server_ticks(now);
        // A routine update, then a weapon in the same tick: the weapon is stamped
        // one tick past the clock.
        s.send_position(now, &OwnPosition::default());
        s.send_position(now, &OwnPosition::default());
        assert_eq!(s.last_position_ticks, Some(tick.wrapping_add(1)));
        assert!(!s.position_due(now), "due again in the same tick");
        // Still not due a few ticks later, and due once the interval has passed.
        let ms = |ticks: u32| now + u64::from(ticks) * 10;
        assert!(!s.position_due(ms(interval - 1)));
        assert!(s.position_due(ms(interval + 2)));
    }

    /// A zone whose billing server is down answers every login with
    /// [`login_code::NO_SCORES`], which is a success code -- Continuum enters and
    /// reports it. Treating it as a failure made every billerless zone impossible
    /// to join, a subgame started without `Subbill` among them. Entering silently
    /// would be the other half of the bug: the player has to be told.
    #[test]
    fn no_scores_enters_the_arena_and_says_so() {
        let response = |code: u8| {
            let mut p = [0u8; 36];
            p[0] = s2c::kind::LOGIN_RESPONSE;
            p[1] = code;
            p
        };
        let run = |code: u8| {
            let mut s = Session::new(SessionConfig::default(), 0, 1);
            s.handle_packet(0, &response(code));
            s.events.into_iter().collect::<Vec<_>>()
        };

        // Both success codes log in; only NO_SCORES explains itself.
        let ok = run(login_code::OK);
        assert!(matches!(ok.first(), Some(SessionEvent::LoggedIn)));
        assert!(
            !ok.iter().any(|e| matches!(e, SessionEvent::Chat { .. })),
            "a plain OK login should not announce anything"
        );

        let no_scores = run(login_code::NO_SCORES);
        assert!(
            matches!(no_scores.first(), Some(SessionEvent::LoggedIn)),
            "NO_SCORES is a success code, not a login failure"
        );
        let notice = no_scores.iter().find_map(|e| match e {
            SessionEvent::Chat { kind, text, .. } if *kind == chat_type::ARENA => Some(text),
            _ => None,
        });
        assert_eq!(
            notice.map(String::as_str),
            Some("Scores are not available in this zone.")
        );

        // Spectator-only lets you in to watch, and says so.
        let spec = run(login_code::SPEC_ONLY);
        assert!(matches!(spec.first(), Some(SessionEvent::LoggedIn)));
        assert!(spec.iter().any(|e| matches!(
            e,
            SessionEvent::Chat { text, .. } if text.contains("watch only")
        )));

        // A code that really is a refusal still refuses.
        let bad = run(login_code::BAD_PASSWORD);
        assert!(matches!(
            bad.first(),
            Some(SessionEvent::LoginFailed {
                code: login_code::BAD_PASSWORD,
                ..
            })
        ));
    }

    /// `0x10` is ASSS's "no name" refusal and Subspace Server .NET's "yes, and
    /// fill in the registration form". A login that carried a name cannot have
    /// been refused for having none, so with a name it is the .NET yes: the
    /// player is let in and the form is asked for. With a blank name it is
    /// still ASSS's refusal, whatever byte 19 holds -- ASSS leaves it
    /// uninitialised on that path. And the byte alone asks for the form on any
    /// success code.
    #[test]
    fn code_0x10_is_a_yes_with_a_form_unless_the_name_was_blank() {
        let response = |code: u8, demo: u8| {
            let mut p = [0u8; 36];
            p[0] = s2c::kind::LOGIN_RESPONSE;
            p[1] = code;
            p[19] = demo;
            p
        };
        let run = |name: &str, code: u8, demo: u8| {
            let cfg = SessionConfig {
                name: name.into(),
                ..Default::default()
            };
            let mut s = Session::new(cfg, 0, 1);
            s.handle_packet(0, &response(code, demo));
            let wanted = s.registration_wanted();
            (s.events.into_iter().collect::<Vec<_>>(), wanted)
        };

        let (events, wanted) = run("Ace", login_code::NO_NAME, 1);
        assert!(
            matches!(events.first(), Some(SessionEvent::LoggedIn)),
            "{events:?}"
        );
        assert!(events.contains(&SessionEvent::RegistrationWanted));
        assert!(wanted);

        let (events, _) = run("", login_code::NO_NAME, 0xCC);
        assert!(matches!(
            events.first(),
            Some(SessionEvent::LoginFailed {
                code: login_code::NO_NAME,
                ..
            })
        ));

        let (events, wanted) = run("Ace", login_code::OK, 1);
        assert!(events.contains(&SessionEvent::RegistrationWanted) && wanted);
        let (events, wanted) = run("Ace", login_code::OK, 0);
        assert!(!events.contains(&SessionEvent::RegistrationWanted) && !wanted);
    }

    /// The arena list used to reach the game as "some other packet", which the
    /// game ignores, so `?arena` answered and nothing appeared. It is its own
    /// event now, with your own arena -- sent with its count negated -- said
    /// in words rather than in a sign.
    #[test]
    fn the_arena_list_arrives_as_its_own_event() {
        let list = s2c::ArenaList {
            arenas: vec![
                s2c::ArenaEntry {
                    name: b"duel".to_vec(),
                    population: 2,
                },
                s2c::ArenaEntry {
                    name: b"0".to_vec(),
                    population: -9,
                },
                s2c::ArenaEntry {
                    name: b"empty".to_vec(),
                    population: 0,
                },
            ],
        };
        let mut data = Vec::new();
        list.encode(&mut data);
        let mut s = Session::new(SessionConfig::default(), 0, 1);
        s.handle_packet(0, &data);
        let got: Vec<_> = s.events.into_iter().collect();
        let listed = |name: &str, players, yours| ArenaListed {
            name: name.into(),
            players,
            yours,
        };
        assert_eq!(
            got,
            vec![SessionEvent::ArenaList(vec![
                listed("duel", 2, false),
                listed("0", 9, true),
                listed("empty", 0, false),
            ])]
        );
    }

    /// A redirect used to reach the game as "some other packet", which it
    /// ignored, so a zone that answers the arena request with one left the
    /// player waiting for an arena that never came. It is its own event now,
    /// with the address read the way the live servers send it (the bytes
    /// reversed) and the arena by its entry type.
    #[test]
    fn a_redirect_says_where_to_go() {
        let redirect = |ip: [u8; 4], port: u16, arena_type: i16, arena: &str| {
            let mut data = Vec::new();
            s2c::Redirect {
                ip,
                port,
                arena_type,
                arena: FixedStr::new(arena.as_bytes()),
                login_id: 7,
            }
            .encode(&mut data);
            assert_eq!(data.len(), 29, "the packet as the servers send it");
            data
        };
        let run = |data: &[u8]| {
            let mut s = Session::new(SessionConfig::default(), 0, 1);
            s.handle_packet(0, data);
            s.events.into_iter().collect::<Vec<_>>()
        };
        let went = |server: &str, arena: Option<&str>| {
            vec![SessionEvent::Redirected {
                server: server.into(),
                arena: arena.map(str::to_string),
                login_id: 7,
            }]
        };

        // 192.0.2.10 (a documentation address), written d c b a.
        let ip = [10, 2, 0, 192];
        assert_eq!(
            run(&redirect(ip, 5000, -1, "")),
            went("192.0.2.10:5000", None)
        );
        assert_eq!(
            run(&redirect(ip, 5000, -3, "duel")),
            went("192.0.2.10:5000", Some("duel"))
        );
        assert_eq!(
            run(&redirect(ip, 5001, 2, "")),
            went("192.0.2.10:5001", Some("2"))
        );

        // Nowhere to go: no event, and the session is not hung up.
        assert!(run(&redirect([0; 4], 5000, -1, "")).is_empty());
        assert!(run(&redirect(ip, 0, -1, "")).is_empty());
    }

    /// The login id a redirect hands over goes out in the next login.
    #[test]
    fn the_redirect_id_goes_out_in_the_login() {
        let cfg = SessionConfig {
            redirect_id: 0x0102_0304,
            ..Default::default()
        };
        let mut s = Session::new(cfg, 0, 1);
        s.send_login();
        let sent = s.buf.clone();
        let login = c2s::Login::decode(&sent).expect("a login went out");
        assert_eq!(login.redirect_id, 0x0102_0304);
    }

    /// Hyperspace, 2026-11-17: the zone said exactly what was wrong in a login
    /// text, then refused with the "busy" code, and the client showed only
    /// "server busy". The player retried, and the lockout grew. The zone's own
    /// words now come with the refusal.
    #[test]
    fn a_refusal_carries_the_zones_own_words() {
        let text = s2c::LoginText {
            text: b"You have tried too many wrong passwords. Try again in a few hours".to_vec(),
        };
        let mut data = Vec::new();
        text.encode(&mut data);
        let mut response = [0u8; 36];
        response[0] = s2c::kind::LOGIN_RESPONSE;
        response[1] = login_code::SERVER_BUSY;

        let mut s = Session::new(SessionConfig::default(), 0, 1);
        s.handle_packet(0, &data);
        s.handle_packet(0, &response);
        let failed = s.events.iter().find_map(|e| match e {
            SessionEvent::LoginFailed {
                code,
                zone_text,
                message,
            } => Some((*code, zone_text.clone(), *message)),
            _ => None,
        });
        let (code, zone_text, message) = failed.expect("refused");
        assert_eq!(code, login_code::SERVER_BUSY);
        assert_eq!(
            zone_text.as_deref(),
            Some("You have tried too many wrong passwords. Try again in a few hours")
        );
        assert!(message.contains("wait"), "busy says to wait: {message}");

        // Without a text first, there is none, and the code speaks alone.
        let mut s = Session::new(SessionConfig::default(), 0, 1);
        s.handle_packet(0, &response);
        assert!(s.events.iter().any(|e| matches!(
            e,
            SessionEvent::LoginFailed {
                zone_text: None,
                ..
            }
        )));
    }

    /// Every refusal code has words of its own, and the ones where trying again
    /// cannot help, or can make it worse, say so.
    #[test]
    fn every_refusal_says_what_to_do() {
        let refusals = (0u8..=0x13).filter(|c| !login_succeeded(*c));
        for code in refusals {
            let m = login_message(code);
            assert!(
                !m.contains("code this client does not know"),
                "{code:#04x}: {m}"
            );
        }
        for code in [
            login_code::LOCKED,
            login_code::NO_PERMISSION,
            login_code::NO_PERMISSION2,
            login_code::TOO_MANY_POINTS,
            login_code::TOO_MANY_DEMO,
            login_code::NO_DEMO,
        ] {
            assert!(
                login_message(code).contains("will not help"),
                "{code:#04x}: {}",
                login_message(code)
            );
        }
        for code in [login_code::BAD_PASSWORD, login_code::SERVER_BUSY] {
            assert!(login_message(code).contains("lock"), "{code:#04x}");
        }
        assert!(login_succeeded(login_code::OK));
        assert!(login_succeeded(login_code::NO_SCORES));
        assert!(login_succeeded(login_code::SPEC_ONLY));
    }

    /// The optional-graphics byte, by connection. Off a Continuum-encrypted
    /// connection the byte is 1, the only way to be offered graphics at all; a
    /// bot that wants none sends the short packet with no byte. (The Continuum
    /// case, where the byte is 0, needs Continuum's encryption, which this
    /// repository does not build; it is one `matches!` in `optional_graphics`.)
    #[test]
    fn optional_graphics_are_asked_for_by_connection() {
        let wanting = Session::new(
            SessionConfig {
                want_lvz: true,
                ..Default::default()
            },
            0,
            1,
        );
        assert_eq!(wanting.optional_graphics(), Some(1));
        let bot = Session::new(SessionConfig::default(), 0, 1);
        assert_eq!(bot.optional_graphics(), None);
    }

    #[test]
    fn go_arena_targets_follow_continuum() {
        let (t, n) = arena_target("");
        assert_eq!((t, n.to_str()), (c2s::arena_type::ANY_PUBLIC, Some("")));
        // Digits name a public arena by number, in the type field itself.
        let (t, n) = arena_target("4");
        assert_eq!((t, n.to_str()), (4, Some("")));
        let (t, n) = arena_target("duel");
        assert_eq!((t, n.to_str()), (c2s::arena_type::NAMED, Some("duel")));
        // A number too large for the field is clamped, not wrapped negative.
        assert_eq!(arena_target("65535").0, i16::MAX);
    }

    /// The pairing the session cannot otherwise catch: the profile picks the login
    /// form, the handshake picks the cipher, and a zone that disagrees drops the
    /// login silently. Every arm is pinned, including the two that must stay quiet.
    #[test]
    fn a_login_form_must_match_the_cipher_it_is_sent_under() {
        use CipherKind::*;

        // The three that mean "the zone expects the SubSpace form".
        for k in [Vie, None, OpenSec] {
            assert!(
                login_form_mismatch(Some(k), true).is_some(),
                "{k:?} + Continuum form is a mismatch"
            );
            assert_eq!(
                login_form_mismatch(Some(k), false),
                Option::None,
                "{k:?} + SubSpace form is right"
            );
        }

        assert!(login_form_mismatch(Some(Continuum), false).is_some());
        assert_eq!(login_form_mismatch(Some(Continuum), true), Option::None);

        // `Other` is the `Cipher` trait's default, so an external cipher that does not
        // override `kind()` lands here. It must read as "cannot tell" in **both**
        // directions -- never as a mismatch, which would cry wolf at every private
        // implementation, and never as agreement either.
        assert_eq!(login_form_mismatch(Some(Other), true), Option::None);
        assert_eq!(login_form_mismatch(Some(Other), false), Option::None);

        // Before the cipher is known there is nothing to compare. In practice this
        // cannot happen -- `Connection::establish` sets the cipher before it pushes
        // `Connected`, and `send_login` runs from that arm -- but the signature allows
        // it and silence is the only safe answer.
        assert_eq!(login_form_mismatch(Option::None, true), Option::None);
        assert_eq!(login_form_mismatch(Option::None, false), Option::None);
    }
}
