//! B6: a soak test for `.ossr` recording and playback.
//!
//! `.ossr` is this project's own format, so there is no Continuum or ASSS behaviour
//! to match and no vector to write. What is unverified is that the recorder and the
//! player hold up over a long, busy, messy session, so this is a soak test — and a
//! soak test that only does the easy things proves nothing. What it covers:
//!
//! 1. **Length and volume.** The format has no index, no chunk table and no record
//!    group: the body is a flat run of `kind/time/length/data` records appended as
//!    the game runs, and nothing in it is rebuilt or re-indexed as it grows. The
//!    only boundaries that exist are the 8 KiB `BufWriter` behind
//!    [`Recorder::create`] (crossed in the first second of any real session), the
//!    `u32` record length (unreachable: a game packet is a few hundred bytes at
//!    most) and the `u32` millisecond timestamp (49.7 days, saturating). So there
//!    is no boundary that only a long recording crosses — `records_cross_many_write_buffers`
//!    and `the_clock_saturates_instead_of_wrapping` pin the two that do exist — and
//!    the soak still runs for minutes, so "nothing accumulates" is a measurement
//!    rather than a claim.
//! 2. **A busy arena.** Thirteen `oss-session` clients against `oss-server`, ten of
//!    them flying a real `oss-sim` ship and firing, so the recorder's hot path is
//!    position and weapon packets at the rate a real arena produces them.
//! 3. **Every event class**, each at least once and while the arena is busy:
//!    entry and departure, ship and freq changes, kills, flags, balls, turrets,
//!    an arena change by the recorder itself, and nine of the ten chat types.
//!    [`REQUIRED`] is the list, and the run fails if any of it is missing rather
//!    than quietly covering less. The tenth chat type, `CHANNEL`, is delivered by
//!    the SSC biller, which a zone on its own has none of; see [`REQUIRED_CHAT`].
//! 4. **Playback compared mechanically against ground truth.** Three comparisons,
//!    each over a whole sequence: the ordered stream of arena observations the live
//!    session made, a roster/score/chat-log snapshot every ten seconds, and every
//!    position the recorder sent against every own-ship position playback rebuilds.
//!    The server is what makes the traffic; what the live session saw, and what this
//!    harness itself sent, is the ground truth playback is held to.
//! 5. **A truncated recording.** One client's recorder is killed mid-session the way
//!    a crash kills it — the session is leaked with data still in the write buffer,
//!    so the file ends wherever the last spill left it — and its file must still
//!    play. The full recording is then cut at every write-buffer boundary, at every
//!    offset in its last few hundred bytes and at a spread of random offsets, and
//!    every prefix must parse to a prefix of the whole and play without panicking.
//! 6. **A reconnect is not covered, because `oss-session` cannot reconnect.** There
//!    is no API for it, `Session::disconnect` is terminal, and a recorder belongs to
//!    one `Session`, so no recording can span one today. The nearest thing the
//!    session layer *can* do — an arena change, which clears the roster and the
//!    player id in the middle of a recording — is in the script instead.
//! 7. **Pause, speed and seeking are missing, not unverified.** `replay::Player`
//!    hands out records by time and does nothing else; `Replay::seek` exists in
//!    `oss-formats` but no caller uses it, and the client has no control for any of
//!    the three.
//!
//! Running it:
//!
//! ```text
//! cargo build --release -p oss-server            # in the sibling oss-server checkout
//! OSS_SOAK_SERVER_BIN=../oss-server/target/release/oss-server \
//!   cargo test -p oss-session --test replay_soak -- --ignored --nocapture
//! ```
//!
//! `OSS_SOAK_SECS` sets the length (default 200; the script needs at least 180) and
//! `OSS_SOAK_CLIENTS` how many fly (default 14, minimum 6). The server is started on
//! a free port with a copy of its own bundled zone in a temporary directory, and is
//! killed by handle when the test ends, pass or fail.
//!
//! The tests that need no server run in a normal `cargo test`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oss_formats::replay::{Record, RecordKind, Replay};
use oss_proto::Packet as _;
use oss_session::replay::{Player as ReplayPlayer, Recorder};
use oss_session::sim::{arena_world, own_position, weapon_for};
use oss_session::udp::UdpDriver;
use oss_session::{OwnPosition, SPECTATOR, Session, SessionConfig, SessionEvent, State, chat_type};
use oss_sim::fixed::Rng;
use oss_sim::{ShipInput, World};

// ---------------------------------------------------------------------------
// What playback has to reproduce
// ---------------------------------------------------------------------------

/// One thing the session saw happen in the arena.
///
/// Only packet-derived observations belong here. `Connected` comes from the
/// transport rather than from a packet, and `MapNeeded`/`MapReady`/`FilesListed`/
/// `Playing` depend on *when* the map was answered — live by a download that takes
/// a moment, in playback from the file the replay carries — so those fire at
/// different points in the stream by design. Everything below is a packet the
/// server sent, or (for the recorder's own movement) one this client sent, so live
/// and playback must produce it in the same order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Obs {
    EnteredArena(i16),
    PlayerEntered {
        pid: i16,
        name: String,
    },
    PlayerLeft(i16),
    /// Someone's position packet. Only the player, deliberately: see [`observe`].
    Moved(i16),
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
    },
    Chat {
        kind: u8,
        pid: i16,
        name: String,
        text: String,
    },
    FlagLocation {
        id: u16,
        x: u16,
        y: u16,
        freq: u16,
    },
    FlagPickup {
        id: u16,
        pid: i16,
    },
    FlagsDropped(i16),
    FlagVictory {
        freq: i16,
        points: i32,
    },
    TurfFlags(usize),
    FlagReward(Vec<(i16, i16)>),
    Ball {
        id: u8,
        x: i16,
        y: i16,
        pid: i16,
        time: u32,
    },
    Goal {
        freq: i16,
        points: i32,
    },
    Attached {
        pid: i16,
        host: i16,
    },
    TurretsDetached(i16),
    Bricks(usize),
    GreenTaken {
        pid: i16,
        prize: i16,
    },
    PrizeReceived {
        count: i16,
        prize: i16,
    },
    WarpTo {
        x: i16,
        y: i16,
    },
    ShipReset,
    Crown {
        on: bool,
        pid: i16,
    },
    CrownTimer {
        ticks: u32,
        add: bool,
    },
    Objects(usize),
    Other(String),
}

impl Obs {
    /// The class name, for the coverage tally.
    fn class(&self) -> &'static str {
        match self {
            Obs::EnteredArena(_) => "entered-arena",
            Obs::PlayerEntered { .. } => "player-entered",
            Obs::PlayerLeft(_) => "player-left",
            Obs::Moved(_) => "position",
            Obs::ShipChanged { .. } => "ship-change",
            Obs::FreqChanged { .. } => "freq-change",
            Obs::Kill { .. } => "kill",
            Obs::Chat { .. } => "chat",
            Obs::FlagLocation { .. } => "flag-location",
            Obs::FlagPickup { .. } => "flag-pickup",
            Obs::FlagsDropped(_) => "flag-drop",
            Obs::FlagVictory { .. } => "flag-victory",
            Obs::TurfFlags(_) => "turf-flags",
            Obs::FlagReward(_) => "flag-reward",
            Obs::Ball { .. } => "ball",
            Obs::Goal { .. } => "goal",
            Obs::Attached { .. } => "turret-attach",
            Obs::TurretsDetached(_) => "turret-detach",
            Obs::Bricks(_) => "brick",
            Obs::GreenTaken { .. } => "green",
            Obs::PrizeReceived { .. } => "prize",
            Obs::WarpTo { .. } => "warp",
            Obs::ShipReset => "ship-reset",
            Obs::Crown { .. } => "crown",
            Obs::CrownTimer { .. } => "crown-timer",
            Obs::Objects(_) => "lvz-objects",
            Obs::Other(_) => "other",
        }
    }
}

/// Classes the run must produce. A soak that quietly covered less than this would
/// prove less than it claims, so a missing one fails the run rather than being left
/// for a reader to notice.
const REQUIRED: &[&str] = &[
    "entered-arena",
    "player-entered",
    "player-left",
    "position",
    "ship-change",
    "freq-change",
    "kill",
    "chat",
    "flag-location",
    "flag-pickup",
    "flag-drop",
    "ball",
    "turret-attach",
    "ship-reset",
    "prize",
    "warp",
];

/// Chat types the zone itself can produce, all nine of them.
///
/// `chat_type::CHANNEL` (9) is left out on purpose: chat channels are delivered by
/// the SSC biller, and `oss-server`'s `chat.rs` only forwards a `CHANNEL` message to
/// one — with no biller the message goes nowhere and nothing comes back. It is also
/// the chat type the recorder is least able to get wrong on its own, since a chat
/// record is the server's packet byte for byte and the kind is a field inside it.
const REQUIRED_CHAT: &[u8] = &[
    chat_type::ARENA,
    chat_type::PUBLIC_MACRO,
    chat_type::PUBLIC,
    chat_type::FREQ,
    chat_type::ENEMY_FREQ,
    chat_type::PRIVATE,
    chat_type::WARNING,
    chat_type::REMOTE_PRIVATE,
    chat_type::SERVER_ERROR,
];

/// One player as the roster shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    pid: i16,
    name: String,
    squad: String,
    ship: u8,
    freq: i16,
    kill_points: i32,
    flag_points: i32,
    wins: i16,
    losses: i16,
    flags: i16,
    /// The last position seen for them. Left out for the recorder itself: live it is
    /// never set (a client is not told its own position back), and in playback it
    /// comes from the client packets, which `Observed::own` checks exactly.
    position: Option<(i16, i16, u8, u16)>,
}

/// The arena at one checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    /// Milliseconds into the recording.
    t_ms: u32,
    pid: Option<i16>,
    roster: Vec<Row>,
    /// Every chat line so far: kind and text.
    chat: Vec<(u8, String)>,
    /// Where the recorder's ship was: live, the last position it sent; in playback,
    /// the one rebuilt from the client packets.
    own: Option<(i16, i16, u8, u16)>,
}

/// Everything one side of the comparison produced.
struct Observed {
    obs: Vec<Obs>,
    snapshots: Vec<Snapshot>,
    /// Every own-ship position, in order.
    own: Vec<(i16, i16, u8, u16)>,
}

fn roster(session: &Session) -> Vec<Row> {
    let me = session.pid();
    session
        .players()
        .map(|p| Row {
            pid: p.pid,
            name: p.name.clone(),
            squad: p.squad.clone(),
            ship: p.ship,
            freq: p.freq,
            kill_points: p.kill_points,
            flag_points: p.flag_points,
            wins: p.wins,
            losses: p.losses,
            flags: p.flags,
            position: (Some(p.pid) != me)
                .then(|| p.position.map(|q| (q.x, q.y, q.rotation, q.bounty)))
                .flatten(),
        })
        .collect()
}

/// The session event as an observation, or `None` for the ones playback is not
/// expected to produce in the same place (see [`Obs`]).
///
/// An observation may only carry what the event itself carries. A `SessionEvent` is
/// a notification, not a value: `PlayerEntered` and `PlayerMoved` name a player and
/// nothing else, so an observer has to look the rest up — and by then it is reading
/// the latest state, not the state at that event. The two sides drain at different
/// granularities (a live session handles every packet in a datagram, and every
/// datagram waiting on the socket, before anything is polled; a playback session is
/// fed one record at a time), so a looked-up field reads one packet later on the
/// live side whenever two updates for the same player land in one pump. What each
/// position packet *said* is compared at the checkpoints and, for the recorder's own
/// ship, exactly.
fn observe(session: &Session, ev: &SessionEvent) -> Option<Obs> {
    Some(match ev {
        SessionEvent::EnteredArena { pid } => Obs::EnteredArena(*pid),
        SessionEvent::PlayerEntered(pid) => Obs::PlayerEntered {
            pid: *pid,
            // A name is fixed for the visit; ship and freq are not, and reading
            // them here would read them late (see below).
            name: session.player(*pid)?.name.clone(),
        },
        SessionEvent::PlayerLeft(pid) => Obs::PlayerLeft(*pid),
        // A client's own movement reaches a playback session from the recorded
        // client packets and reaches a live one from nowhere at all, so it is
        // compared on its own (`Observed::own`), not in this stream.
        SessionEvent::PlayerMoved(pid) if Some(*pid) == session.pid() => return None,
        SessionEvent::PlayerMoved(pid) => Obs::Moved(*pid),
        SessionEvent::ShipChanged { pid, ship, freq } => Obs::ShipChanged {
            pid: *pid,
            ship: *ship,
            freq: *freq,
        },
        SessionEvent::FreqChanged { pid, freq } => Obs::FreqChanged {
            pid: *pid,
            freq: *freq,
        },
        SessionEvent::Kill {
            killer,
            killed,
            bounty,
            ..
        } => Obs::Kill {
            killer: *killer,
            killed: *killed,
            bounty: *bounty,
        },
        SessionEvent::Chat {
            kind,
            pid,
            name,
            text,
            ..
        } => Obs::Chat {
            kind: *kind,
            pid: *pid,
            name: name.clone().unwrap_or_default(),
            text: text.clone(),
        },
        SessionEvent::FlagLocation { id, x, y, freq } => Obs::FlagLocation {
            id: *id,
            x: *x,
            y: *y,
            freq: *freq,
        },
        SessionEvent::FlagPickup { id, pid } => Obs::FlagPickup { id: *id, pid: *pid },
        SessionEvent::FlagsDropped { pid } => Obs::FlagsDropped(*pid),
        SessionEvent::FlagVictory { freq, points } => Obs::FlagVictory {
            freq: *freq,
            points: *points,
        },
        SessionEvent::TurfFlags(o) => Obs::TurfFlags(o.len()),
        SessionEvent::FlagReward(r) => Obs::FlagReward(r.clone()),
        SessionEvent::Ball(b) => Obs::Ball {
            id: b.ball_id,
            x: b.x,
            y: b.y,
            pid: b.pid,
            time: b.time,
        },
        SessionEvent::Goal { freq, points } => Obs::Goal {
            freq: *freq,
            points: *points,
        },
        SessionEvent::Attached { pid, host } => Obs::Attached {
            pid: *pid,
            host: *host,
        },
        SessionEvent::TurretsDetached { pid } => Obs::TurretsDetached(*pid),
        SessionEvent::Bricks(b) => Obs::Bricks(b.len()),
        SessionEvent::GreenTaken { pid, prize, .. } => Obs::GreenTaken {
            pid: *pid,
            prize: *prize,
        },
        SessionEvent::PrizeReceived { count, prize } => Obs::PrizeReceived {
            count: *count,
            prize: *prize,
        },
        SessionEvent::WarpTo { x, y } => Obs::WarpTo { x: *x, y: *y },
        SessionEvent::ShipReset => Obs::ShipReset,
        SessionEvent::Crown { on, pid, .. } => Obs::Crown { on: *on, pid: *pid },
        SessionEvent::CrownTimer { ticks, add } => Obs::CrownTimer {
            ticks: *ticks,
            add: *add,
        },
        SessionEvent::ObjectsToggled(v) => Obs::Objects(v.len()),
        SessionEvent::ObjectsChanged(v) => Obs::Objects(v.len()),
        SessionEvent::Other(p) => Obs::Other(format!("{:?}", std::mem::discriminant(&**p))),
        _ => return None,
    })
}

/// Where two runs of checkpoints first disagree, down to the field. A whole
/// `Snapshot` is a roster of twenty players and the chat log so far, and printing
/// two of them says almost nothing about what changed.
fn snapshot_difference(live: &[Snapshot], play: &[Snapshot]) -> Option<String> {
    if live.len() != play.len() {
        return Some(format!(
            "checkpoints: live took {} and playback {}",
            live.len(),
            play.len()
        ));
    }
    for (i, (a, b)) in live.iter().zip(play).enumerate() {
        let at = format!("checkpoint {i} at {} ms", a.t_ms);
        if a.t_ms != b.t_ms {
            return Some(format!("{at}: playback is at {} ms", b.t_ms));
        }
        if a.pid != b.pid {
            return Some(format!("{at}: own pid {:?} vs {:?}", a.pid, b.pid));
        }
        if a.chat != b.chat {
            let n = a
                .chat
                .iter()
                .zip(&b.chat)
                .take_while(|(x, y)| x == y)
                .count();
            return Some(format!(
                "{at}: chat line {n} of {}/{}
  live     {:?}
  playback {:?}",
                a.chat.len(),
                b.chat.len(),
                a.chat.get(n),
                b.chat.get(n)
            ));
        }
        if a.own != b.own {
            return Some(format!("{at}: own position {:?} vs {:?}", a.own, b.own));
        }
        if a.roster.len() != b.roster.len() {
            return Some(format!(
                "{at}: roster of {} vs {}",
                a.roster.len(),
                b.roster.len()
            ));
        }
        for (x, y) in a.roster.iter().zip(&b.roster) {
            if x != y {
                return Some(format!(
                    "{at}: player {} ({})
  live     {x:?}
  playback {y:?}",
                    x.pid, x.name
                ));
            }
        }
    }
    None
}

/// The first index where two sequences differ, with a little context, so a failure
/// names the record rather than dumping a hundred thousand of them.
fn first_difference<T: PartialEq + std::fmt::Debug>(
    what: &str,
    a: &[T],
    b: &[T],
) -> Option<String> {
    let n = a.len().min(b.len());
    let i = (0..n).find(|&i| a[i] != b[i]);
    match (i, a.len() == b.len()) {
        (None, true) => None,
        (None, false) => {
            let extra = if a.len() > n { &a[n] } else { &b[n] };
            Some(format!(
                "{what}: live has {} and playback {}; the shorter is a prefix of the \
                 longer, and the next one is {extra:?}",
                a.len(),
                b.len(),
            ))
        }
        (Some(i), _) => Some(format!(
            "{what}: differ at {i} of {}/{}\n  live     {:?}\n  playback {:?}\n  before   {:?}",
            a.len(),
            b.len(),
            a[i],
            b[i],
            &a[i.saturating_sub(2)..i]
        )),
    }
}

// ---------------------------------------------------------------------------
// The zone under test
// ---------------------------------------------------------------------------

/// Group password written into the temporary zone's `staff.conf`, so two clients can
/// `?grplogin mod` and produce the two staff-only chat types. A group password needs
/// no account and changes nothing else about the zone.
const GROUP_PASSWORD: &str = "soak-only-not-a-secret";

/// An `oss-server` running its own bundled zone on a free port, from a copy in a
/// temporary directory with a flag game and a group password added.
struct Zone {
    child: Child,
    addr: String,
    dir: PathBuf,
}

impl Zone {
    fn start(bin: &Path, dir: &Path) -> Result<Self, String> {
        let out = Command::new(bin)
            .arg("--init-zone")
            .arg(dir)
            .output()
            .map_err(|e| format!("{}: {e}", bin.display()))?;
        if !out.status.success() {
            return Err(format!(
                "--init-zone failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        // Staff, for the two server-only chat types.
        let staff = dir.join("conf").join("staff.conf");
        let mut text = std::fs::read_to_string(&staff).map_err(|e| e.to_string())?;
        text.push_str(&format!("\n[GroupPasswords]\nmod = {GROUP_PASSWORD}\n"));
        std::fs::write(&staff, text).map_err(|e| e.to_string())?;
        // A flag game, so flags are placed and can be picked up. The bundled zone
        // attaches none ("Add fg_wz or fg_turf for a flag game", says its own
        // comment), and without one flagcore puts nothing on the map. It has to be
        // *loaded* as well as attached, or arenaman logs "can't attach: module
        // 'fg_wz' is not loaded" and carries on with no flags.
        let modules = dir.join("conf").join("modules.conf");
        let mut text = std::fs::read_to_string(&modules).map_err(|e| e.to_string())?;
        text.push_str(
            "
fg_wz
",
        );
        std::fs::write(&modules, text).map_err(|e| e.to_string())?;
        let arena = dir.join("arenas").join("(default)").join("arena.conf");
        let mut text = std::fs::read_to_string(&arena)
            .map_err(|e| e.to_string())?
            .replace(
                "AttachModules = points_kill points_flag points_goal",
                "AttachModules = points_kill points_flag points_goal fg_wz",
            );
        // And lagaction out of the way. It is not being indulged: against a zone
        // running the stock limits, every honest client is specced for "excessive
        // S2C packetloss (99.6 > 15.00)" within about five seconds of a *busy*
        // arena — reproducible with nothing but
        // `oss-bot --count 12 --fly`. The client reports how many datagrams it has
        // received only in its five-second time-sync packet, while the server's
        // per-connection `datagrams_sent` has meanwhile carried the map download
        // and the arena's relayed traffic, and `lagdata::ploss` compares the two as
        // soon as `s_sent` passes 200. A quiet arena never reaches 200 before the
        // client's second sync, which is why only a busy one shows it. That is an
        // oss-net/oss-server accounting question, not a replay one, and specced
        // ships would leave this soak with an empty arena to record.
        text.push_str(
            "
[Lag]
PingToSpec = 100000
SpikeToSpec = 1000000
             S2CLossToSpec = 1000
C2SLossToSpec = 1000
WeaponLossToSpec = 1000
             S2CLossToDisallowFlags = 1000
WeaponLossToDisallowFlags = 1000
             C2SLossToDisallowFlags = 1000
PingToDisallowFlags = 100000
             S2CLossToStartIgnoringWeapons = 1000
             WeaponLossToStartIgnoringWeapons = 1000
             S2CLossToIgnoreAllWeapons = 1000
WeaponLossToIgnoreAllWeapons = 1000
             PingToStartIgnoringWeapons = 100000
PingToIgnoreAllWeapons = 100000
",
        );
        std::fs::write(&arena, text).map_err(|e| e.to_string())?;

        let port = free_port()?;
        let addr = format!("127.0.0.1:{port}");
        let log = std::fs::File::create(dir.with_extension("log")).map_err(|e| e.to_string())?;
        let child = Command::new(bin)
            .arg("--zone")
            .arg(dir)
            .arg("--bind")
            .arg(&addr)
            .arg("--log")
            .arg("warn")
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .map_err(|e| format!("{}: {e}", bin.display()))?;
        Ok(Self {
            child,
            addr,
            dir: dir.to_path_buf(),
        })
    }
}

impl Drop for Zone {
    fn drop(&mut self) {
        // By handle, never by image name: nothing else on this machine is touched.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A port where both the game port and the ping port (port + 1) are free.
fn free_port() -> Result<u16, String> {
    for _ in 0..64 {
        let s = std::net::UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = s.local_addr().map_err(|e| e.to_string())?.port();
        drop(s);
        if port < u16::MAX && std::net::UdpSocket::bind(("127.0.0.1", port + 1)).is_ok() {
            return Ok(port);
        }
    }
    Err("no free port pair".into())
}

/// The `oss-server` binary: `OSS_SOAK_SERVER_BIN`, else the sibling checkout's
/// release then debug build.
fn server_binary() -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "oss-server.exe"
    } else {
        "oss-server"
    };
    if let Some(p) = std::env::var_os("OSS_SOAK_SERVER_BIN") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../oss-server/target");
    ["release", "debug"]
        .iter()
        .map(|p| root.join(p).join(exe))
        .find(|p| p.is_file())
}

// ---------------------------------------------------------------------------
// The clients
// ---------------------------------------------------------------------------

/// Clients with a job, on top of the flying ones.
const EXTRA: usize = 4;

const RECORDER: usize = 0;
const MODERATOR: usize = 1;
const FAR: usize = 2;
const CRASHER: usize = 3;

/// The arena everyone but [`FAR`] plays in.
const ARENA: &str = "soak";
/// Where the recorder goes when it changes arena: a different map, so the replay has
/// to carry two of them.
const OTHER_ARENA: &str = "scf";
/// Another arena, so a remote private message has somewhere to come from.
const FAR_ARENA: &str = "soakfar";

struct Client {
    name: String,
    session: Session,
    driver: UdpDriver,
    world: Option<World>,
    map: Option<Vec<u8>>,
    green_seed: u32,
    rng: Rng,
    last_step_ms: u64,
    /// The ship this client is flying now.
    ship: u8,
    /// The ship it asks for. Kept apart from `ship` because the server puts
    /// everyone in spectator mode as they enter and says so with a ship change,
    /// and a client that reads that as "what I wanted" never takes a ship at all.
    want: u8,
    dead: bool,
    /// Flag ids seen on the map, and a ball seen loose: what the script reaches for.
    flags: Vec<u16>,
    loose_ball: Option<(u8, u32)>,
}

impl Client {
    fn new(server: &str, name: &str, arena: &str, ship: u8, n: u32) -> Result<Self, String> {
        let driver = UdpDriver::connect(server).map_err(|e| format!("{server}: {e}"))?;
        let cfg = SessionConfig {
            name: name.to_owned(),
            arena: Some(arena.to_owned()),
            ship,
            machine_id: 0x0B07_0000 + n,
            ..Default::default()
        };
        let seed = (n + 1).wrapping_mul(2_654_435_761) | 1;
        let session = Session::new(cfg, driver.now_ms(), seed);
        Ok(Self {
            name: name.to_owned(),
            session,
            driver,
            world: None,
            map: None,
            green_seed: 0,
            rng: Rng::new(seed),
            last_step_ms: 0,
            ship,
            want: ship,
            dead: false,
            flags: Vec::new(),
            loose_ball: None,
        })
    }
}

fn start_flying(c: &mut Client) {
    let (Some(map), Some(settings)) = (c.map.as_ref(), c.session.settings()) else {
        return;
    };
    let (mut world, _) = arena_world(map, settings, c.green_seed);
    let pid = c.session.pid().unwrap_or(0) as u16;
    let freq = c.session.me().map_or(0, |p| p.freq) as u16;
    world.add_ship(pid, c.ship, freq);
    c.world = Some(world);
}

/// Where everyone fights. A random walk over a 16384-pixel map scatters the ships
/// so far apart that the server stops relaying their positions to each other
/// (`game.rs` only sends a position to players within their own screen size, or a
/// distance-weighted share within `Net:PositionExtraPixels`), and the arena looks
/// busy from the outside while the recorder sees almost nothing. Real players
/// fight over one place, so the ships here steer back towards one too.
const RALLY_RADIUS: i32 = 450;

/// Advances the simulated ship and sends positions and weapons, as `oss-bot` does,
/// but flying towards `rally` instead of wandering off. Returns the position sent.
fn fly(c: &mut Client, now: u64, rally: (i32, i32)) -> Option<OwnPosition> {
    let mut fired = None;
    {
        let world = c.world.as_mut()?;
        while c.last_step_ms + 10 <= now {
            c.last_step_ms += 10;
            let r = c.rng.next_u32();
            let ship = world.ships.first()?;
            let p = ship.pos.pixels();
            let (dx, dy) = (rally.0 - p.x, rally.1 - p.y);
            let far = dx * dx + dy * dy > RALLY_RADIUS * RALLY_RADIUS;
            // Ship rotation 0 is up and counts clockwise in fortieths of a turn.
            let want = (f64::from(dx).atan2(-f64::from(dy)) / std::f64::consts::TAU * 40.0)
                .round()
                .rem_euclid(40.0) as i32;
            let turn = (want - i32::from(ship.direction()) + 60).rem_euclid(40) - 20;
            let aimed = turn.abs() <= 4;
            let input = ShipInput {
                forward: if far { aimed } else { r.is_multiple_of(5) },
                left: if far { turn < 0 } else { r.is_multiple_of(7) },
                right: if far { turn > 0 } else { r % 7 == 1 },
                gun: r.is_multiple_of(25),
                bomb: r % 400 == 1,
                afterburner: far && aimed && r.is_multiple_of(3),
                ..Default::default()
            };
            for ev in world.step(&[input]) {
                if fired.is_none() {
                    fired = weapon_for(&ev, &world.ships[0]);
                }
            }
        }
    }
    if fired.is_none() && !c.session.position_due(now) {
        return None;
    }
    let pos = own_position(&c.world.as_ref()?.ships[0], fired);
    c.session.send_position(now, &pos);
    Some(pos)
}

// ---------------------------------------------------------------------------
// The script
// ---------------------------------------------------------------------------

/// Something a client does at a point in the run.
#[derive(Debug, Clone, Copy)]
enum Act {
    /// `?grplogin mod <password>`: also the first `ARENA` line, since the server
    /// answers it.
    GroupLogin,
    /// Public, public-macro or team chat.
    Say(u8, &'static str),
    /// Team chat aimed at another client's frequency.
    EnemyFreq(usize, &'static str),
    /// A private message to another client.
    Private(usize, &'static str),
    /// `?warn` on another client, privately: that client gets a `WARNING` line.
    Warn(usize, &'static str),
    /// `\text` from a moderator: everyone who can see mod chat gets `SERVER_ERROR`.
    ModChat(&'static str),
    /// A remote private message, from the client in the other arena.
    RemotePrivate(usize, &'static str),
    SetShip(u8),
    SetFreq(i16),
    /// Report a death at another client's hands.
    DieTo(usize),
    PickFlag,
    DropFlags,
    PickBall,
    ShootBall,
    /// Attach as a turret to another client.
    Attach(usize),
    Detach,
    Brick,
    /// `?shipreset` on another client, privately: that client's ship is reset.
    ResetShip(usize),
    /// `?prize` on another client, privately.
    Prize(usize, &'static str),
    /// `?warpto` on another client, privately.
    WarpTo(usize, &'static str),
    /// Ask for the zone's news: an `IncomingFile`, which arrives as a sized transfer
    /// and is recorded as the one reassembled packet.
    News,
    ChangeArena(&'static str),
    Leave,
    /// Kill this client's recorder the way a crash would: the session, with whatever
    /// is still in the write buffer, is leaked rather than dropped.
    CrashRecorder,
}

/// `(seconds into the run, which client, what it does)`. All of it happens while the
/// arena is busy: fourteen ships are flying and shooting throughout.
const SCRIPT: &[(u64, usize, Act)] = &[
    (12, RECORDER, Act::GroupLogin),
    (13, MODERATOR, Act::GroupLogin),
    (16, MODERATOR, Act::Say(chat_type::PUBLIC, "soak: public")),
    (
        18,
        MODERATOR,
        Act::Say(chat_type::PUBLIC_MACRO, "soak: macro"),
    ),
    (20, RECORDER, Act::SetFreq(3)),
    (21, 5, Act::SetFreq(3)),
    (22, MODERATOR, Act::SetFreq(7)),
    (24, 5, Act::Say(chat_type::FREQ, "soak: team")),
    (26, MODERATOR, Act::EnemyFreq(RECORDER, "soak: enemy team")),
    (28, MODERATOR, Act::Private(RECORDER, "soak: private")),
    (30, MODERATOR, Act::ModChat("soak: mod chat")),
    (32, MODERATOR, Act::Warn(RECORDER, "soak: warning")),
    (
        34,
        FAR,
        Act::RemotePrivate(RECORDER, "soak: remote private"),
    ),
    (38, RECORDER, Act::News),
    (42, CRASHER, Act::SetShip(3)),
    (46, RECORDER, Act::SetShip(1)),
    (50, RECORDER, Act::PickFlag),
    (54, 6, Act::PickFlag),
    (58, RECORDER, Act::DropFlags),
    (62, RECORDER, Act::DieTo(5)),
    (66, 5, Act::DieTo(RECORDER)),
    (70, 7, Act::DieTo(6)),
    (74, RECORDER, Act::PickBall),
    (78, RECORDER, Act::ShootBall),
    (82, 6, Act::PickBall),
    (86, RECORDER, Act::SetFreq(1)),
    (88, 8, Act::SetFreq(1)),
    (88, 9, Act::SetFreq(1)),
    // Attaching needs the host alive and on the same frequency, and a ship in the
    // middle of a respawn is neither, so more than one goes in.
    (90, 8, Act::Attach(RECORDER)),
    (92, 9, Act::Attach(RECORDER)),
    (94, 8, Act::Attach(RECORDER)),
    (96, 8, Act::Detach),
    (97, 9, Act::Detach),
    (100, RECORDER, Act::Brick),
    (102, MODERATOR, Act::Prize(RECORDER, "3 1")),
    (104, RECORDER, Act::DieTo(7)),
    (106, MODERATOR, Act::ResetShip(RECORDER)),
    (108, MODERATOR, Act::WarpTo(RECORDER, "500 500")),
    (110, CRASHER, Act::CrashRecorder),
    (118, 9, Act::Leave),
    (
        124,
        MODERATOR,
        Act::Say(chat_type::PUBLIC, "soak: still here"),
    ),
    (130, 6, Act::DropFlags),
    (136, RECORDER, Act::DieTo(5)),
    (
        142,
        MODERATOR,
        Act::Say(chat_type::PUBLIC, "soak: changing arena"),
    ),
    (146, RECORDER, Act::ChangeArena(OTHER_ARENA)),
    // Two of the fliers follow, so the new arena is busy too and a 140 KB map
    // downloads while the recording is running.
    (150, 6, Act::ChangeArena(OTHER_ARENA)),
    (152, 7, Act::ChangeArena(OTHER_ARENA)),
    (158, RECORDER, Act::SetShip(2)),
    (
        164,
        MODERATOR,
        Act::Say(chat_type::PUBLIC, "soak: nearly done"),
    ),
    (172, RECORDER, Act::Say(chat_type::PUBLIC, "soak: done")),
];

/// The last scripted second; a run must be this long plus a wind-down.
const SCRIPT_END: u64 = 172;
const CHECKPOINT_SECS: u64 = 5;

// ---------------------------------------------------------------------------
// The live run
// ---------------------------------------------------------------------------

struct LiveResult {
    observed: Observed,
    checkpoints: Vec<u32>,
    recording: PathBuf,
    crash_recording: PathBuf,
    chat_kinds: Vec<u8>,
}

#[allow(clippy::too_many_lines)]
fn run_live(server: &str, secs: u64, flying: usize, dir: &Path) -> Result<LiveResult, String> {
    let total = flying + EXTRA;
    let mut clients = Vec::new();
    for i in 0..total {
        let (name, arena, ship) = match i {
            RECORDER => ("SoakRec".to_owned(), ARENA, 0),
            MODERATOR => ("SoakMod".to_owned(), ARENA, 1),
            FAR => ("SoakFar".to_owned(), FAR_ARENA, SPECTATOR),
            CRASHER => ("SoakCrash".to_owned(), ARENA, 2),
            n => (format!("Soak{n}"), ARENA, (n % 8) as u8),
        };
        clients.push(Client::new(server, &name, arena, ship, i as u32)?);
    }

    // The recorder records from before it connects, so the replay holds the whole
    // visit; the crasher records too, to be killed in the middle of the run.
    let recording = dir.join("soak.ossr");
    let crash_recording = dir.join("soak-crash.ossr");
    // The instant the recorder's clock starts, which every record's timestamp is
    // relative to. Reading it again afterwards would be a few milliseconds late and
    // would put every checkpoint slightly behind the records it should include.
    let rec_start = clients[RECORDER].driver.now_ms();
    {
        let rec = Recorder::create(
            &recording,
            rec_start,
            &[
                ("zone", server),
                ("player", "SoakRec"),
                ("arena", ARENA),
                ("client", "replay_soak"),
            ],
        )
        .map_err(|e| format!("{}: {e}", recording.display()))?;
        clients[RECORDER].session.record_to(rec, rec_start);
    }
    {
        let now = clients[CRASHER].driver.now_ms();
        let rec = Recorder::create(&crash_recording, now, &[("player", "SoakCrash")])
            .map_err(|e| format!("{}: {e}", crash_recording.display()))?;
        clients[CRASHER].session.record_to(rec, now);
    }

    let mut obs = Vec::new();
    let mut chat: Vec<(u8, String)> = Vec::new();
    let mut chat_kinds: Vec<u8> = Vec::new();
    let mut snapshots = Vec::new();
    let mut checkpoints = Vec::new();
    let mut own: Vec<(i16, i16, u8, u16)> = Vec::new();
    let mut last_own: Option<(i16, i16, u8, u16)> = None;

    let mut next_step = 0usize;
    let mut next_checkpoint = CHECKPOINT_SECS;
    // Where the fight is; cleared when the recorder changes arena.
    let mut anchor: Option<(i32, i32)> = None;
    let start = Instant::now();
    let deadline = Duration::from_secs(secs);

    while start.elapsed() < deadline {
        let elapsed_s = start.elapsed().as_secs();
        // Everyone flies at the recorder, and the recorder holds one spot, so the
        // fight stays where the recording is instead of wandering out of the range
        // the server relays positions over.
        let here = clients[RECORDER]
            .world
            .as_ref()
            .and_then(|w| w.ships.first())
            .map(|s| {
                let p = s.pos.pixels();
                (p.x, p.y)
            });
        if anchor.is_none() {
            anchor = here;
        }
        let rally = here.or(anchor).unwrap_or((512 * 16, 512 * 16));
        let own_rally = anchor.unwrap_or(rally);

        for (i, c) in clients.iter_mut().enumerate().take(total) {
            if c.dead || c.session.state() == State::Closed {
                continue;
            }
            if let Err(e) = c.driver.pump(&mut c.session) {
                return Err(format!("{}: socket: {e}", c.name));
            }
            let is_recorder = i == RECORDER;
            while let Some(ev) = c.session.poll_event() {
                if is_recorder {
                    // A new arena: the fight moves with it.
                    if matches!(ev, SessionEvent::EnteredArena { .. }) {
                        anchor = None;
                    }
                    if let SessionEvent::Chat { kind, text, .. } = &ev {
                        chat.push((*kind, text.clone()));
                        if !chat_kinds.contains(kind) {
                            chat_kinds.push(*kind);
                        }
                    }
                    if let Some(o) = observe(&c.session, &ev) {
                        obs.push(o);
                    }
                }
                client_event(c, &ev);
            }
            let now = c.driver.now_ms();
            if c.world.is_none() {
                c.last_step_ms = now;
            }
            if c.session.state() == State::Playing {
                if c.world.is_some() {
                    let to = if is_recorder { own_rally } else { rally };
                    if let Some(p) = fly(c, now, to)
                        && is_recorder
                    {
                        let q = (p.x, p.y, p.rotation, p.bounty);
                        own.push(q);
                        last_own = Some(q);
                    }
                } else if c.session.position_due(now) {
                    // A spectator still reports in, so the server knows it is alive.
                    // These count as ground truth too: the recorder writes them like
                    // any other client packet and playback rebuilds them.
                    let pos = OwnPosition::default();
                    c.session.send_position(now, &pos);
                    if is_recorder {
                        let q = (pos.x, pos.y, pos.rotation, pos.bounty);
                        own.push(q);
                        last_own = Some(q);
                    }
                }
            }
            let _ = c.driver.send_pending(&mut c.session);
        }

        while let Some(&(at, who, act)) = SCRIPT.get(next_step) {
            if elapsed_s < at {
                break;
            }
            next_step += 1;
            assert!(who < total, "the script needs client {who}");
            do_act(&mut clients, who, act);
        }

        // A checkpoint, taken after the recorder's pump and its sends, so that every
        // record stamped at or before `t` is one the live session has already
        // handled. The wait afterwards keeps the next record's stamp past `t`.
        if elapsed_s >= next_checkpoint {
            next_checkpoint += CHECKPOINT_SECS;
            let r = &mut clients[RECORDER];
            let t = (r.driver.now_ms() - rec_start).min(u32::MAX as u64) as u32;
            snapshots.push(Snapshot {
                t_ms: t,
                pid: r.session.pid(),
                roster: roster(&r.session),
                chat: chat.clone(),
                own: last_own,
            });
            checkpoints.push(t);
            let until = rec_start + u64::from(t);
            while r.driver.now_ms() <= until {
                std::hint::spin_loop();
            }
        }

        std::thread::sleep(Duration::from_millis(2));
    }

    // Wind down: everyone leaves and the recording is closed.
    for c in clients.iter_mut().filter(|c| !c.dead) {
        c.session.disconnect();
        let _ = c.driver.send_pending(&mut c.session);
    }
    for _ in 0..100 {
        for c in clients.iter_mut().filter(|c| !c.dead) {
            let _ = c.driver.pump(&mut c.session);
            while c.session.poll_event().is_some() {}
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    clients[RECORDER].session.stop_recording();

    Ok(LiveResult {
        observed: Observed {
            obs,
            snapshots,
            own,
        },
        checkpoints,
        recording,
        crash_recording,
        chat_kinds,
    })
}

/// What a client does with its own events: answer the map, start flying, and keep
/// track of the flags and the ball the script may reach for.
fn client_event(c: &mut Client, ev: &SessionEvent) {
    match ev {
        SessionEvent::MapNeeded { .. } => c.session.download_map(),
        SessionEvent::MapReady { data, .. } => c.map = Some(data.clone()),
        SessionEvent::Seeds { green_seed, .. } => c.green_seed = *green_seed,
        SessionEvent::FilesListed(files) => {
            for f in files.clone() {
                c.session.download_file(&f.name);
            }
        }
        SessionEvent::Playing => {
            let current = c.session.me().map_or(SPECTATOR, |p| p.ship);
            if c.want != SPECTATOR && current != c.want {
                c.session.set_ship(c.want);
            } else if c.want != SPECTATOR && c.world.is_none() {
                start_flying(c);
            }
        }
        SessionEvent::ShipChanged { pid, ship, .. } if Some(*pid) == c.session.pid() => {
            c.ship = *ship;
            c.world = None;
            if *ship != SPECTATOR {
                start_flying(c);
            }
        }
        SessionEvent::ShipReset => {
            if let Some(w) = c.world.as_mut()
                && !w.ships.is_empty()
            {
                w.respawn(0);
            }
        }
        SessionEvent::FlagLocation { id, .. } => {
            if !c.flags.contains(id) {
                c.flags.push(*id);
            }
        }
        SessionEvent::Ball(b) => {
            c.loose_ball = (b.pid < 0 && b.time != 0).then_some((b.ball_id, b.time));
        }
        _ => {}
    }
}

/// A staff command aimed privately at another client, which is how a command with
/// a `privcmd_` capability is targeted at a player.
fn staff_command(clients: &mut [Client], who: usize, to: usize, text: &str) {
    if let Some(pid) = target_pid(clients, who, to) {
        clients[who]
            .session
            .send_chat(chat_type::PRIVATE, pid, text);
    }
}

/// The pid `who` should use for `to`, as `who`'s own session knows them.
fn target_pid(clients: &[Client], who: usize, to: usize) -> Option<i16> {
    let name = clients.get(to)?.name.clone();
    clients[who]
        .session
        .players()
        .find(|p| p.name.eq_ignore_ascii_case(&name))
        .map(|p| p.pid)
}

fn do_act(clients: &mut [Client], who: usize, act: Act) {
    let now = clients[who].driver.now_ms();
    match act {
        Act::GroupLogin => clients[who].session.send_chat(
            chat_type::PUBLIC,
            0,
            &format!("?grplogin mod {GROUP_PASSWORD}"),
        ),
        Act::Say(kind, text) => clients[who].session.send_chat(kind, 0, text),
        Act::EnemyFreq(to, text) => {
            if let Some(pid) = target_pid(clients, who, to) {
                clients[who]
                    .session
                    .send_chat(chat_type::ENEMY_FREQ, pid, text);
            }
        }
        Act::Private(to, text) => {
            if let Some(pid) = target_pid(clients, who, to) {
                clients[who]
                    .session
                    .send_chat(chat_type::PRIVATE, pid, text);
            }
        }
        Act::Warn(to, text) => {
            if let Some(pid) = target_pid(clients, who, to) {
                clients[who]
                    .session
                    .send_chat(chat_type::PRIVATE, pid, &format!("?warn {text}"));
            }
        }
        Act::ModChat(text) => {
            clients[who]
                .session
                .send_chat(chat_type::PUBLIC, 0, &format!("\\{text}"))
        }
        Act::RemotePrivate(to, text) => {
            let name = clients[to].name.clone();
            clients[who].session.send_chat(
                chat_type::REMOTE_PRIVATE,
                0,
                &format!(":{name}:{text}"),
            );
        }
        Act::SetShip(s) => {
            clients[who].want = s;
            clients[who].session.set_ship(s);
        }
        Act::SetFreq(f) => clients[who].session.set_freq(f),
        Act::DieTo(killer) => {
            if let Some(pid) = target_pid(clients, who, killer) {
                clients[who].session.send_death(pid, 15);
            }
        }
        Act::PickFlag => {
            if let Some(id) = clients[who].flags.first().copied() {
                clients[who].session.pickup_flag(id);
            }
        }
        Act::DropFlags => clients[who].session.drop_flags(),
        Act::PickBall => {
            if let Some((id, time)) = clients[who].loose_ball {
                clients[who].session.pickup_ball(id, time);
            }
        }
        Act::ShootBall => {
            let p = clients[who]
                .world
                .as_ref()
                .and_then(|w| w.ships.first())
                .map(|s| s.pos.pixels());
            let xy = p.map_or((8192, 8192), |p| (p.x as i16, p.y as i16));
            clients[who].session.shoot_ball(now, 0, xy, (300, -300));
        }
        Act::Attach(host) => {
            if let Some(pid) = target_pid(clients, who, host) {
                clients[who].session.attach(pid);
            }
        }
        Act::Detach => clients[who].session.attach(-1),
        Act::Brick => {
            let p = clients[who]
                .world
                .as_ref()
                .and_then(|w| w.ships.first())
                .map(|s| s.pos.pixels());
            if let Some(p) = p {
                clients[who]
                    .session
                    .send_brick((p.x / 16) as i16, (p.y / 16) as i16);
            }
        }
        Act::ResetShip(to) => staff_command(clients, who, to, "?shipreset"),
        Act::Prize(to, what) => staff_command(clients, who, to, &format!("?prize {what}")),
        Act::WarpTo(to, xy) => staff_command(clients, who, to, &format!("?warpto {xy}")),
        Act::News => clients[who].session.request_news(),
        Act::ChangeArena(a) => clients[who].session.change_arena(a),
        Act::Leave => {
            clients[who].session.disconnect();
            let _ = clients[who].driver.send_pending(&mut clients[who].session);
            clients[who].dead = true;
            return;
        }
        Act::CrashRecorder => {
            // What a crash leaves: nothing flushes or closes the file, so it ends
            // wherever the 8 KiB write buffer last spilled.
            if let Ok(mut dummy) = Client::new("127.0.0.1:1", "none", ARENA, SPECTATOR, 0) {
                dummy.dead = true;
                std::mem::forget(std::mem::replace(&mut clients[who], dummy));
            }
            return;
        }
    }
    let _ = clients[who].driver.send_pending(&mut clients[who].session);
}

// ---------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------

/// Plays the recording back the way the client does (`game/online.rs`): a playback
/// session fed the records due at each moment, answering the map and the LVZ files
/// from the replay itself.
fn play_back(bytes: &[u8], checkpoints: &[u32], end_ms: u32) -> Result<Observed, String> {
    let mut player = ReplayPlayer::from_bytes(bytes)?;
    let mut session = Session::playback(SessionConfig::default(), 0);
    let mut obs = Vec::new();
    let mut chat: Vec<(u8, String)> = Vec::new();
    let mut snapshots = Vec::new();
    let mut own: Vec<(i16, i16, u8, u16)> = Vec::new();
    let mut last_own = None;
    let mut next_checkpoint = 0usize;

    // Every 20 ms, and exactly on each checkpoint. Landing exactly on it matters:
    // the live snapshot was taken with every record stamped at or before `c`
    // handled and none after, so a playback that stepped past `c` would hold one
    // position packet more than the live session did and read as a difference.
    let mut stops: Vec<u32> = (0..=end_ms).step_by(20).collect();
    stops.extend_from_slice(checkpoints);
    stops.push(end_ms);
    stops.sort_unstable();
    stops.dedup();

    for t in stops {
        let due = player.due(t).to_vec();
        for r in &due {
            session.feed(u64::from(r.time_ms), r);
            while let Some(ev) = session.poll_event() {
                match &ev {
                    SessionEvent::MapNeeded { name, .. } => {
                        let data = player
                            .file(name)
                            .ok_or_else(|| format!("the replay has no map {name}"))?
                            .to_vec();
                        if !session.use_cached_map(&data) {
                            return Err(format!("the replay's {name} did not match"));
                        }
                    }
                    SessionEvent::FilesListed(files) => {
                        for f in files.clone() {
                            if let Some(d) = player.file(&f.name).map(<[u8]>::to_vec) {
                                session.use_cached_file(&f.name, &d);
                            }
                        }
                    }
                    SessionEvent::Chat { kind, text, .. } => chat.push((*kind, text.clone())),
                    SessionEvent::PlayerMoved(pid) if Some(*pid) == session.pid() => {
                        if let Some(q) = session.me().and_then(|p| p.position) {
                            let q = (q.x, q.y, q.rotation, q.bounty);
                            own.push(q);
                            last_own = Some(q);
                        }
                    }
                    _ => {}
                }
                if let Some(o) = observe(&session, &ev) {
                    obs.push(o);
                }
            }
        }
        if checkpoints.get(next_checkpoint) == Some(&t) {
            next_checkpoint += 1;
            snapshots.push(Snapshot {
                t_ms: t,
                pid: session.pid(),
                roster: roster(&session),
                chat: chat.clone(),
                own: last_own,
            });
        }
    }
    Ok(Observed {
        obs,
        snapshots,
        own,
    })
}

// ---------------------------------------------------------------------------
// The soak
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs an oss-server binary and minutes of wall clock; see the module header"]
#[allow(clippy::too_many_lines)]
fn replay_soak() {
    let secs: u64 = std::env::var("OSS_SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    assert!(
        secs >= SCRIPT_END + 8,
        "OSS_SOAK_SECS must be at least {} so the script finishes",
        SCRIPT_END + 8
    );
    let flying: usize = std::env::var("OSS_SOAK_CLIENTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(14);
    assert!(flying >= 6, "OSS_SOAK_CLIENTS must be at least 6");

    let bin = server_binary().unwrap_or_else(|| {
        panic!(
            "no oss-server binary: build one in the sibling checkout \
             (cargo build --release -p oss-server) or set OSS_SOAK_SERVER_BIN"
        )
    });
    let dir = std::env::temp_dir().join(format!("oss-replay-soak-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let zone = Zone::start(&bin, &dir.join("zone")).expect("start oss-server");
    // A moment to bind before the first client knocks.
    std::thread::sleep(Duration::from_millis(750));

    let live = run_live(&zone.addr, secs, flying, &dir).expect("the live run");
    drop(zone);

    let bytes = std::fs::read(&live.recording).expect("read the recording");
    let replay = Replay::parse(&bytes).expect("the recording parses");
    assert!(!replay.truncated, "the recording was cut short");

    // 1. Length and volume: a real session, not a thirty-second capture.
    let server_packets = replay
        .records
        .iter()
        .filter(|r| r.kind == RecordKind::ServerPacket)
        .count();
    let client_packets = replay
        .records
        .iter()
        .filter(|r| r.kind == RecordKind::ClientPacket)
        .count();
    // The observation stream deliberately does not carry what a position packet
    // said, so the recording is what says whether the arena was shooting. Counting
    // S2C 0x05 packets is not enough: `oss-server` sends that shape for an ordinary
    // position too whenever the bounty or the player id needs more than a byte, so
    // the weapon field has to be read.
    let weapon_packets = replay
        .records
        .iter()
        .filter(|r| r.kind == RecordKind::ServerPacket)
        .filter(|r| {
            r.data.first() == Some(&oss_proto::s2c::kind::WEAPON)
                && oss_proto::position::S2CWeapon::decode(&r.data)
                    .is_ok_and(|w| w.weapon.kind() != 0)
        })
        .count();
    let names: Vec<String> = replay
        .files()
        .map(|(n, _)| String::from_utf8_lossy(n).into_owned())
        .collect();
    println!(
        "recording: {} bytes, {} records ({server_packets} from the server, \
         {weapon_packets} of them shots, {client_packets} from us), \
         files {names:?}, {:.1} s",
        bytes.len(),
        replay.records.len(),
        f64::from(replay.duration_ms()) / 1000.0,
    );
    let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
    for o in &live.observed.obs {
        *classes.entry(o.class()).or_default() += 1;
    }
    println!("classes seen: {classes:?}");
    println!("chat types seen: {:?}", live.chat_kinds);

    assert!(
        replay.duration_ms() >= SCRIPT_END as u32 * 1000,
        "the recording stopped before the script finished ({} ms)",
        replay.duration_ms()
    );
    assert!(
        server_packets > 10_000,
        "only {server_packets} server packets: the arena was not busy"
    );
    assert!(
        weapon_packets > 300,
        "only {weapon_packets} shots fired: the arena was not shooting"
    );
    assert!(
        bytes.len() > 8192 * 20,
        "only {} bytes: fewer than twenty write buffers",
        bytes.len()
    );
    assert!(
        names.len() >= 2,
        "the replay carries only {names:?}; the arena change needs both maps"
    );
    // Continuum sends a position every `SendPositionDelay` ticks plus one per
    // weapon. Many times that is a flood, which is what `position_due` reading its
    // difference unsigned used to cause once a weapon shared a tick with an update.
    let per_second = client_packets as f64 / f64::from(replay.duration_ms()) * 1000.0;
    println!("the recorder sent {per_second:.1} packets a second");
    assert!(
        per_second < 50.0,
        "the recorder sent {per_second:.0} packets a second: a position flood"
    );

    // 3. Coverage.
    let missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|c| !classes.contains_key(c))
        .collect();
    assert!(
        missing.is_empty(),
        "event classes that never happened: {missing:?}"
    );
    let missing_chat: Vec<u8> = REQUIRED_CHAT
        .iter()
        .copied()
        .filter(|k| !live.chat_kinds.contains(k))
        .collect();
    assert!(
        missing_chat.is_empty(),
        "chat types never seen: {missing_chat:?}"
    );

    // 4. Playback against ground truth.
    let played = play_back(&bytes, &live.checkpoints, replay.duration_ms()).expect("playback");
    if let Some(d) = first_difference("observations", &live.observed.obs, &played.obs) {
        panic!("{d}");
    }
    if let Some(d) = snapshot_difference(&live.observed.snapshots, &played.snapshots) {
        panic!("{d}");
    }
    if let Some(d) = first_difference("own positions", &live.observed.own, &played.own) {
        panic!("{d}");
    }
    assert_eq!(
        played.snapshots.len(),
        live.observed.snapshots.len(),
        "playback produced a different number of checkpoints"
    );
    assert!(
        live.observed.own.len() > 500,
        "the recorder only sent {} positions",
        live.observed.own.len()
    );
    println!(
        "{} observations, {} checkpoints and {} own positions matched",
        played.obs.len(),
        played.snapshots.len(),
        played.own.len()
    );

    // 5a. The killed recorder's file.
    let crash = std::fs::read(&live.crash_recording).expect("read the killed recording");
    let partial = Replay::parse(&crash).expect("a killed recording still parses");
    println!(
        "killed recording: {} bytes, {} records, {} s, truncated {}",
        crash.len(),
        partial.records.len(),
        partial.duration_ms() / 1000,
        partial.truncated
    );
    assert!(
        !partial.records.is_empty(),
        "the killed recording holds nothing"
    );
    let mut crashed = Session::playback(SessionConfig::default(), 0);
    for r in &partial.records {
        crashed.feed(u64::from(r.time_ms), r);
        while crashed.poll_event().is_some() {}
    }
    assert!(
        crashed.pid().is_some(),
        "the killed recording did not play far enough to enter the arena"
    );
    assert!(
        partial.duration_ms() >= 60_000,
        "the killed recording stops at {} ms, long before the kill",
        partial.duration_ms()
    );

    // 5b. Every prefix of the real recording.
    check_every_prefix(&bytes, &replay);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Cuts a recording short where a crash can leave it and checks that every prefix
/// parses to a prefix of the whole and plays.
fn check_every_prefix(bytes: &[u8], whole: &Replay) {
    let mut offsets: Vec<usize> = Vec::new();
    // Where a crash actually cuts: the write buffer's boundaries.
    offsets.extend((8192..bytes.len()).step_by(8192));
    // Every byte of the tail, where a record header is most likely to be split.
    offsets.extend(bytes.len().saturating_sub(400)..=bytes.len());
    // And a spread through the middle.
    let mut rng = Rng::new(0x5EED_1234);
    for _ in 0..400 {
        offsets.push(rng.next_u32() as usize % bytes.len());
    }
    offsets.sort_unstable();
    offsets.dedup();

    for &cut in &offsets {
        let part = &bytes[..cut];
        if cut < 12 {
            assert!(
                Replay::parse(part).is_err(),
                "a {cut}-byte file is not a replay"
            );
            continue;
        }
        let p = Replay::parse(part).unwrap_or_else(|e| panic!("cut at {cut}: {e}"));
        assert!(
            p.records.len() <= whole.records.len() && p.records == whole.records[..p.records.len()],
            "cut at {cut} is not a prefix of the whole recording \
             ({} records of {})",
            p.records.len(),
            whole.records.len()
        );
        // And it plays: cut-short file data must never panic the client.
        let mut s = Session::playback(SessionConfig::default(), 0);
        for r in &p.records {
            s.feed(u64::from(r.time_ms), r);
            while s.poll_event().is_some() {}
        }
    }
    println!("{} truncation points checked", offsets.len());
}

// ---------------------------------------------------------------------------
// What can be checked without a server
// ---------------------------------------------------------------------------

/// A `Write` that keeps the bytes, so a recorder can be driven without a file.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Sink {
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut v = self
            .0
            .lock()
            .map_err(|_| std::io::Error::other("sink poisoned"))?;
        v.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The format has no chunk, group or index that a long recording crosses: the body
/// is a flat run of records and nothing is rebuilt as it grows. The one boundary
/// that does exist is the write buffer, and a recording many times its size comes
/// back byte for byte, in order, once each.
#[test]
fn records_cross_many_write_buffers() {
    let sink = Sink::default();
    let mut rec = Recorder::new(
        Box::new(std::io::BufWriter::new(sink.clone())),
        0,
        &[("player", "soak")],
    )
    .expect("recorder");
    let mut expected: Vec<Record> = vec![Record::meta(0, "player", "soak")];
    let mut rng = Rng::new(0xB01D_FACE);
    for i in 0..20_000u32 {
        let now = u64::from(i) * 7;
        let len = 4 + rng.next_u32() as usize % 60;
        let data: Vec<u8> = (0..len).map(|k| (i as u8).wrapping_add(k as u8)).collect();
        let kind = if i.is_multiple_of(3) {
            rec.server_packet(now, &data);
            RecordKind::ServerPacket
        } else {
            rec.client_packet(now, &data);
            RecordKind::ClientPacket
        };
        expected.push(Record {
            kind,
            time_ms: now as u32,
            data,
        });
    }
    rec.flush();
    drop(rec);

    let bytes = sink.bytes();
    assert!(
        bytes.len() > 8192 * 50,
        "{} bytes is not enough write buffers to prove anything",
        bytes.len()
    );
    let replay = Replay::parse(&bytes).expect("parses");
    assert!(!replay.truncated);
    assert!(replay.records == expected, "the records came back changed");
    assert_eq!(replay.duration_ms(), 19_999 * 7);

    // A recording this long still hands its records out in order and once each.
    let mut player = ReplayPlayer::from_bytes(&bytes).expect("opens");
    let mut seen = 0usize;
    let mut t = 0u32;
    while !player.finished() {
        let due = player.due(t);
        for (k, r) in due.iter().enumerate() {
            assert!(
                *r == expected[seen + k],
                "record {} came out wrong",
                seen + k
            );
        }
        seen += due.len();
        t = t.saturating_add(997);
        assert!(t < u32::MAX, "the player never finished");
    }
    assert_eq!(seen, expected.len());
}

/// A recording cut short anywhere still parses to a prefix of itself and plays. This
/// is the cheap exhaustive version of what the soak does to a real file.
#[test]
fn a_cut_recording_is_a_prefix_of_itself() {
    let sink = Sink::default();
    let mut rec =
        Recorder::new(Box::new(sink.clone()), 0, &[("player", "soak")]).expect("recorder");
    rec.file(0, "map.lvl", &[7; 300]);
    for i in 0..120u32 {
        rec.server_packet(u64::from(i) * 11, &[i as u8; 24]);
        if i.is_multiple_of(5) {
            rec.marker(u64::from(i) * 11, "mark");
        }
    }
    rec.flush();
    drop(rec);
    let bytes = sink.bytes();
    let whole = Replay::parse(&bytes).expect("parses");
    let mut cut_mid_record = 0;

    for cut in 0..=bytes.len() {
        let part = &bytes[..cut];
        if cut < 12 {
            assert!(Replay::parse(part).is_err(), "{cut} bytes is not a replay");
            continue;
        }
        let p = Replay::parse(part).unwrap_or_else(|e| panic!("cut at {cut}: {e}"));
        assert!(
            p.records.len() <= whole.records.len() && p.records == whole.records[..p.records.len()],
            "cut at {cut} is not a prefix"
        );
        if p.truncated {
            cut_mid_record += 1;
            assert!(p.records.len() < whole.records.len(), "cut at {cut}");
        }
        if cut.is_multiple_of(29) {
            let mut s = Session::playback(SessionConfig::default(), 0);
            for r in &p.records {
                s.feed(u64::from(r.time_ms), r);
                while s.poll_event().is_some() {}
            }
        }
    }
    // Most cuts land inside a record rather than on a boundary; if none did, the
    // loop above was not testing truncation at all.
    assert!(
        cut_mid_record > bytes.len() / 2,
        "{cut_mid_record} of {}",
        bytes.len()
    );
}

/// The recorder's clock saturates rather than wrapping, so a recording running past
/// 49.7 days keeps its records in order instead of rewinding to the start.
#[test]
fn the_clock_saturates_instead_of_wrapping() {
    let sink = Sink::default();
    let mut rec = Recorder::new(Box::new(sink.clone()), 0, &[]).expect("recorder");
    rec.server_packet(u64::from(u32::MAX) - 1, &[1, 2, 3]);
    rec.server_packet(u64::from(u32::MAX) + 1000, &[4, 5, 6]);
    rec.server_packet(u64::from(u32::MAX) * 3, &[7, 8, 9]);
    rec.flush();
    drop(rec);
    let replay = Replay::parse(&sink.bytes()).expect("parses");
    let times: Vec<u32> = replay.records.iter().map(|r| r.time_ms).collect();
    assert_eq!(times, vec![u32::MAX - 1, u32::MAX, u32::MAX]);
    assert!(
        times.windows(2).all(|w| w[0] <= w[1]),
        "the times went backwards"
    );
}

/// A check on the harness's own comparison: it has to notice a difference, not only
/// report success.
#[test]
fn the_comparison_catches_a_difference() {
    let a = vec![Obs::ShipReset, Obs::PlayerLeft(3), Obs::ShipReset];
    let mut b = a.clone();
    assert!(first_difference("x", &a, &b).is_none());
    b[1] = Obs::PlayerLeft(4);
    let msg = first_difference("x", &a, &b).expect("a difference");
    assert!(msg.contains("differ at 1"), "{msg}");
    let short = a[..2].to_vec();
    let msg = first_difference("x", &a, &short).expect("a length difference");
    assert!(msg.contains("live has 3 and playback 2"), "{msg}");
}
