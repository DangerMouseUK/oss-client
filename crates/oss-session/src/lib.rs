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

pub mod directory;
pub mod map;
pub mod replay;
pub mod security;
pub mod sim;
pub mod test_zone;
pub mod udp;

use std::collections::{BTreeMap, VecDeque};

use oss_net::{
    Config, Connection, DisconnectReason, Event as NetEvent, Handshake, VieClientHandshake,
};
use oss_proto::c2s::{self, GoArena, Login, MapRequest, SecurityResponse, SetFreq, SetShip};
use oss_proto::s2c::ChatBody;
use oss_proto::s2c::{self, MapFilename, login_code};
use oss_proto::{C2SPosition, ClientSettings, FixedStr, Packet, S2CPacket, WeaponInfo, checksum};

pub use oss_net::DisconnectReason as Reason;
pub use oss_proto::common::chat_type;
pub use oss_proto::common::{Brick, ObjectChange, object_change};
pub use oss_proto::s2c::BallPosition;

/// Spectator "ship".
pub const SPECTATOR: u8 = 8;

/// Who to log in as and where to go.
#[derive(Debug, Clone)]
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
    /// Ask for the arena's LVZ files (graphics; bots usually don't need them).
    pub want_lvz: bool,
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
    LoginFailed {
        code: u8,
        message: &'static str,
    },
    LoggedIn,
    /// Your player id in the arena.
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
    MapProgress {
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
    /// Any packet the session doesn't interpret itself.
    Other(Box<S2CPacket>),
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
    downloading: bool,
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
}

impl Session {
    /// A session using VIE encryption (what SubSpace servers expect).
    pub fn new(cfg: SessionConfig, now_ms: u64, seed: u32) -> Self {
        Self::with_handshake(cfg, now_ms, Box::new(VieClientHandshake::new(seed)))
    }

    /// A session with any client handshake (e.g. unencrypted, or a private cipher).
    pub fn with_handshake(cfg: SessionConfig, now_ms: u64, handshake: Box<dyn Handshake>) -> Self {
        Self {
            conn: Connection::client(Config::client(), handshake, now_ms),
            cfg,
            state: State::Connecting,
            pid: None,
            players: BTreeMap::new(),
            settings: None,
            settings_raw: Vec::new(),
            map: None,
            files: Vec::new(),
            downloads: VecDeque::new(),
            turf_flags: Default::default(),
            downloading: false,
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

    pub fn poll_event(&mut self) -> Option<SessionEvent> {
        self.events.pop_front()
    }

    pub fn disconnect(&mut self) {
        self.conn.disconnect();
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
                NetEvent::SizedProgress { received, total } => self
                    .events
                    .push_back(SessionEvent::MapProgress { received, total }),
                NetEvent::Disconnected(reason) => {
                    self.state = State::Closed;
                    self.events.push_back(SessionEvent::Closed(reason));
                }
                _ => {}
            }
        }
    }

    fn send_login(&mut self) {
        let login = Login {
            kind: c2s::kind::LOGIN,
            flags: 0,
            name: FixedStr::new(self.cfg.name.as_bytes()),
            password: FixedStr::new(self.cfg.password.as_bytes()),
            machine_id: self.cfg.machine_id,
            connect_type: 4,
            timezone_bias: self.cfg.timezone_bias,
            unknown1: 0,
            client_version: 134,
            field444: 444,
            field555: 555,
            permission_id: self.cfg.machine_id,
            reserved: [0; 12],
            cont_id: None,
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
            optional_graphics: self.cfg.want_lvz.then_some(1),
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
            S2CPacket::LoginResponse(r) => {
                if r.code == login_code::OK {
                    self.events.push_back(SessionEvent::LoggedIn);
                    self.go_arena();
                } else {
                    self.events.push_back(SessionEvent::LoginFailed {
                        code: r.code,
                        message: login_message(r.code),
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
                self.downloading = false;
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
        self.downloading || !self.downloads.is_empty()
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

    fn next_download(&mut self) {
        if self.downloading {
            return;
        }
        if let Some(index) = self.downloads.pop_front() {
            self.downloading = true;
            let index = (index != 0).then_some(index);
            self.send(&MapRequest { index }, true);
        }
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
        let map_checksum = self
            .map
            .as_ref()
            .and_then(|m| m.tiles.as_ref())
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
        let ping = (self.conn.rtt_ms() / 10).min(u16::MAX as u64) as u16;
        let stats = self.conn.stats();
        let resp = SecurityResponse {
            weapon_count: self.weapons_received,
            settings_checksum,
            exe_checksum: checksum::vie_exe_checksum(key),
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
            continuum: None,
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
        self.last_position_ticks
            .is_none_or(|t| now.wrapping_sub(t) >= self.position_interval())
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

fn fixed_to_string(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim_end().to_string()
}

/// Human-readable login failure.
pub fn login_message(code: u8) -> &'static str {
    match code {
        login_code::NEW_USER => "unknown name (new user)",
        login_code::BAD_PASSWORD => "wrong password",
        login_code::FULL => "arena full",
        login_code::LOCKED => "locked out",
        login_code::NO_PERMISSION | login_code::NO_PERMISSION2 => "no permission",
        login_code::SPEC_ONLY => "spectator only",
        login_code::TOO_MANY_POINTS => "too many points",
        login_code::TOO_SLOW => "connection too slow",
        login_code::NO_NEW_CONNECTIONS => "server not accepting new connections",
        login_code::BAD_NAME => "bad name",
        login_code::OFFENSIVE_NAME => "offensive name",
        login_code::NO_SCORES => "no scores",
        login_code::SERVER_BUSY => "server busy",
        login_code::TOO_LOW_USAGE => "too low usage",
        login_code::NO_NAME => "no name",
        login_code::TOO_MANY_DEMO | login_code::NO_DEMO => "demo version not allowed",
        _ => "login refused",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
