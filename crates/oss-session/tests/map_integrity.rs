//! The map a session answers security checks with is the map the zone announced.
//!
//! Every security answer carries a checksum of the arena's map, so a session that
//! plays on a map whose CRC is not the zone's answers with a checksum the zone
//! cannot accept, and a zone that checks it ends the visit. These tests run a
//! session through the real transport against a scripted zone, as `login_flow.rs`
//! does, with a synthetic map and a synthetic stand-in for an LVZ file, and pin
//! the three ways that went wrong or could:
//!
//! 1. **An unnamed slot in the file list is never asked for.** A subgame zone
//!    lists an optional file it is not offering as an entry with no name, no
//!    checksum and no size, keeping the indices after it. Asking for it anyway
//!    gets the file behind it, under a name the list never gave.
//! 2. **A file the arena does not list never replaces the map.** Such a file used
//!    to be taken for the map, and the next security answer carried that file's
//!    bytes read as tiles.
//! 3. **A damaged map is never used.** One damaged copy is asked for again; a
//!    second ends the visit with [`SessionEvent::MapRejected`], and a security
//!    check held for the map is never answered.
//!
//! Playback of a recording that holds a damaged map neither refuses nor hangs up:
//! there is no zone to ask again.

use std::io::Write;

use oss_net::{Config, Event, Server, TransferId, VieAcceptor};
use oss_proto::common::PlayerData;
use oss_proto::s2c::{
    EnteringArena, LoginResponse, MapData, MapFile, MapFilename, PlayerEntering, PlayerId, Security,
};
use oss_proto::{C2SPacket, ClientSettings, FixedStr, Packet, checksum};
use oss_session::replay::{Player as ReplayPlayer, Recorder};
use oss_session::{Session, SessionConfig, SessionEvent, State};

const ADDR: u32 = 1;
const KEY: u32 = 0x0F00_2345;
const MAP_NAME: &str = "tiny.lvl";
const LVZ_NAME: &str = "art.lvz";
/// The file behind the unnamed slot: listed by nobody, sent all the same.
const HIDDEN_NAME: &str = "hidden.lvz";

/// Where [`checksum::map_checksum`] starts sampling for [`KEY`].
fn sample_origin() -> (u32, u32) {
    (KEY % 31, KEY % 32)
}

fn tile(x: u32, y: u32, t: u32) -> [u8; 4] {
    (x | (y << 12) | (t << 24)).to_le_bytes()
}

/// The arena's map: a tile list with walls on squares the checksum samples.
fn map_bytes() -> Vec<u8> {
    let (x, y) = sample_origin();
    [tile(x, y, 1), tile(x + 31, y + 32, 5), tile(400, 400, 7)].concat()
}

/// A map the zone did not announce: the same with one more wall.
fn other_map_bytes() -> Vec<u8> {
    let (x, y) = sample_origin();
    [map_bytes(), tile(x + 62, y, 9).to_vec()].concat()
}

/// Stand-in for an LVZ. Only its first four bytes are like one; the rest is a
/// tile list, so that read as a map it puts walls where the checksum looks.
fn hidden_bytes() -> Vec<u8> {
    let (x, y) = sample_origin();
    [
        b"CONT".to_vec(),
        tile(x, y + 64, 3).to_vec(),
        tile(x + 93, y + 96, 2).to_vec(),
    ]
    .concat()
}

const LVZ: &[u8] = b"CONT listed lvz bytes";

/// The security map checksum of a level's bytes, as the session computes it.
fn map_checksum_of(level: &[u8]) -> u32 {
    let grid = oss_formats::Level::parse(level).unwrap().tile_grid();
    checksum::map_checksum(KEY, |x, y| grid.get(x as u32, y as u32))
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn packet(p: &impl Packet) -> Vec<u8> {
    let mut v = Vec::new();
    p.encode(&mut v);
    v
}

fn map_data(name: &str, data: Vec<u8>) -> Vec<u8> {
    packet(&MapData {
        name: FixedStr::new(name.as_bytes()),
        data,
    })
}

fn security() -> Vec<u8> {
    packet(&Security {
        green_seed: 7,
        door_seed: 9,
        timestamp: 0,
        key: KEY,
    })
}

/// The arena's file list: the map, an unnamed slot, then one real LVZ (index 2).
fn file_list() -> MapFilename {
    MapFilename::Files(vec![
        MapFile {
            name: FixedStr::new(MAP_NAME.as_bytes()),
            checksum: checksum::crc32(&map_bytes()),
            size: 0,
        },
        MapFile {
            name: FixedStr::default(),
            checksum: 0,
            size: 0,
        },
        MapFile {
            name: FixedStr::new(LVZ_NAME.as_bytes()),
            checksum: checksum::crc32(LVZ),
            size: LVZ.len() as u32 + 17,
        },
    ])
}

#[derive(Default)]
struct Zone {
    /// Damaged copies of the map to send before a good one.
    damaged_maps: u32,
    /// Send the security check with the arena, before the map is asked for.
    challenge_on_entry: bool,
    /// Push [`HIDDEN_NAME`] unasked once the map has gone, then send the check.
    push_hidden: bool,
    settings: Vec<u8>,
    /// File requests received, in order (0 = the map).
    requests: Vec<u16>,
    /// Every security answer received.
    answers: Vec<oss_proto::c2s::SecurityResponse>,
    /// The transfer carrying the pushed file, while it is under way.
    hidden_transfer: Option<TransferId>,
    /// Steps left before the check goes out after the pushed file.
    challenge_in: Option<u32>,
    disconnected: bool,
}

impl Zone {
    fn handle(&mut self, server: &mut Server<u32>, pkt: &[u8]) {
        let decoded = match C2SPacket::decode(pkt).expect("valid C2S") {
            C2SPacket::ContLogin(c) => C2SPacket::Login(c.0),
            other => other,
        };
        match decoded {
            C2SPacket::Login(_) => {
                let reply = LoginResponse {
                    code: 0,
                    server_version: 134,
                    ..Default::default()
                };
                server.send(&ADDR, &packet(&reply), true);
            }
            C2SPacket::GoArena(_) => {
                self.settings = ClientSettings::default().to_bytes();
                server.send(
                    &ADDR,
                    &packet(&PlayerId {
                        pid: 5,
                        arena: None,
                    }),
                    true,
                );
                server.send(&ADDR, &self.settings, true);
                let me = PlayerData {
                    pid: 5,
                    name: FixedStr::new(b"tester"),
                    ship: 8,
                    ..Default::default()
                };
                server.send(&ADDR, &packet(&PlayerEntering { players: vec![me] }), true);
                server.send(&ADDR, &packet(&file_list()), true);
                server.send(&ADDR, &packet(&EnteringArena {}), true);
                if self.challenge_on_entry {
                    server.send(&ADDR, &security(), true);
                }
            }
            C2SPacket::MapRequest(r) => {
                let index = r.index.unwrap_or(0);
                self.requests.push(index);
                let p = match index {
                    0 if self.damaged_maps > 0 => {
                        self.damaged_maps -= 1;
                        map_data(MAP_NAME, zlib(&other_map_bytes()))
                    }
                    0 => map_data(MAP_NAME, zlib(&map_bytes())),
                    2 => map_data(LVZ_NAME, LVZ.to_vec()),
                    // The unnamed slot, answered as a subgame zone answers it.
                    _ => map_data(HIDDEN_NAME, hidden_bytes()),
                };
                let good_map = index == 0 && p == map_data(MAP_NAME, zlib(&map_bytes()));
                let conn = server.connection_mut(&ADDR).unwrap();
                conn.send_sized(p);
                if good_map && self.push_hidden {
                    self.hidden_transfer =
                        Some(conn.send_sized(map_data(HIDDEN_NAME, hidden_bytes())));
                }
            }
            C2SPacket::SecurityResponse(r) => self.answers.push(r),
            _ => {}
        }
    }
}

/// Runs a session against `zone` for up to `steps` 10 ms steps, answering the map
/// and the files the way the graphical client does. Stops early once `done`.
fn run(
    zone: &mut Zone,
    steps: usize,
    recording: Option<Shared>,
    done: impl Fn(&Zone, &Session) -> bool,
) -> (Session, Vec<SessionEvent>) {
    let mut now = 1_000u64;
    let mut server = Server::new(Config::server());
    server.add_acceptor(Box::new(VieAcceptor::default()));
    let cfg = SessionConfig {
        name: "tester".into(),
        password: "pw".into(),
        want_lvz: true,
        ..Default::default()
    };
    let mut client = Session::new(cfg, now, 0xBEEF);
    if let Some(r) = recording {
        client.record_to(Recorder::new(Box::new(r), now, &[]).unwrap(), now);
    }
    let mut events = Vec::new();
    for _ in 0..steps {
        now += 10;
        client.tick(now);
        while let Some(d) = client.poll_transmit() {
            server.handle_datagram(now, ADDR, &d);
        }
        server.tick(now);
        while let Some((_, ev)) = server.poll_event() {
            match ev {
                Event::Packet(p) => zone.handle(&mut server, &p),
                Event::SizedSent(id) if zone.hidden_transfer == Some(id) => {
                    zone.hidden_transfer = None;
                    zone.challenge_in = Some(50);
                }
                Event::Disconnected(_) => zone.disconnected = true,
                _ => {}
            }
        }
        if let Some(n) = zone.challenge_in.as_mut() {
            if *n == 0 {
                zone.challenge_in = None;
                server.send(&ADDR, &security(), true);
            } else {
                *n -= 1;
            }
        }
        while let Some((_, d)) = server.poll_transmit() {
            client.handle_datagram(now, &d);
        }
        while let Some(ev) = client.poll_event() {
            match &ev {
                SessionEvent::MapNeeded { .. } => client.download_map(),
                SessionEvent::FilesListed(files) => {
                    for f in files {
                        client.download_file(&f.name);
                    }
                }
                _ => {}
            }
            events.push(ev);
        }
        if done(zone, &client) {
            break;
        }
    }
    (client, events)
}

fn maps_ready(events: &[SessionEvent]) -> Vec<&[u8]> {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::MapReady { data, .. } => Some(&data[..]),
            _ => None,
        })
        .collect()
}

#[test]
fn the_test_files_would_give_different_answers() {
    // Without these, the tests below could pass for the wrong reason.
    let real = map_checksum_of(&map_bytes());
    assert_ne!(real, map_checksum_of(&hidden_bytes()));
    assert_ne!(real, map_checksum_of(&other_map_bytes()));
    assert_ne!(
        checksum::crc32(&map_bytes()),
        checksum::crc32(&other_map_bytes())
    );
}

#[test]
fn an_unnamed_slot_in_the_file_list_is_never_asked_for() {
    let mut zone = Zone {
        challenge_on_entry: true,
        ..Default::default()
    };
    let (client, events) = run(&mut zone, 2000, None, |z, c| {
        !z.answers.is_empty() && !c.downloads_pending() && z.requests.len() >= 2
    });

    let listed = events.iter().find_map(|e| match e {
        SessionEvent::FilesListed(f) => Some(f.clone()),
        _ => None,
    });
    let listed = listed.expect("files listed");
    assert_eq!(listed.len(), 1, "the unnamed slot is not offered");
    // The listed file keeps its own index; the slot before it is not renumbered away.
    assert_eq!((listed[0].index, listed[0].name.as_str()), (2, LVZ_NAME));
    assert_eq!(zone.requests, vec![0, 2]);
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::FileReady { name, data, downloaded: true } if name == LVZ_NAME && data == LVZ
    )));
    assert_eq!(maps_ready(&events), vec![&map_bytes()[..]]);
    assert_eq!(zone.answers.len(), 1);
    assert_eq!(zone.answers[0].map_checksum, map_checksum_of(&map_bytes()));
    assert_eq!(client.state(), State::Playing);
}

#[test]
fn a_file_the_arena_does_not_list_never_replaces_the_map() {
    let mut zone = Zone {
        push_hidden: true,
        ..Default::default()
    };
    let (client, events) = run(&mut zone, 3000, None, |z, _| !z.answers.is_empty());

    assert!(
        zone.hidden_transfer.is_none(),
        "the unlisted file was delivered before the check"
    );
    assert_eq!(zone.answers.len(), 1, "the check was answered");
    assert_eq!(
        zone.answers[0].map_checksum,
        map_checksum_of(&map_bytes()),
        "answered with the announced map, not the unlisted file"
    );
    assert_eq!(
        zone.answers[0].settings_checksum,
        checksum::settings_checksum(KEY, &zone.settings)
    );
    assert_eq!(maps_ready(&events), vec![&map_bytes()[..]]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::FileReady { name, .. } if name == HIDDEN_NAME)),
        "an unlisted file is not handed on either"
    );
    assert!(!client.downloads_pending());
    assert_eq!(client.state(), State::Playing);
}

#[test]
fn a_damaged_map_is_asked_for_again_and_the_check_waits_for_it() {
    let mut zone = Zone {
        damaged_maps: 1,
        challenge_on_entry: true,
        ..Default::default()
    };
    let (client, events) = run(&mut zone, 3000, None, |z, c| {
        !z.answers.is_empty() && !c.downloads_pending()
    });

    assert_eq!(zone.requests, vec![0, 0, 2], "the map was asked for twice");
    assert_eq!(maps_ready(&events), vec![&map_bytes()[..]]);
    assert_eq!(zone.answers.len(), 1);
    assert_eq!(zone.answers[0].map_checksum, map_checksum_of(&map_bytes()));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::MapRejected { .. }))
    );
    assert_eq!(client.state(), State::Playing);
}

#[test]
fn a_map_that_stays_damaged_ends_the_visit_unanswered() {
    let mut zone = Zone {
        damaged_maps: u32::MAX,
        challenge_on_entry: true,
        ..Default::default()
    };
    let recording = Shared::default();
    let (client, events) = run(&mut zone, 3000, Some(recording.clone()), |z, c| {
        z.disconnected && c.state() == State::Closed
    });

    assert_eq!(zone.requests, vec![0, 0], "asked once more, then no more");
    assert!(maps_ready(&events).is_empty(), "no damaged map is used");
    assert!(
        zone.answers.is_empty(),
        "the check held for the map is never answered"
    );
    let rejected = events
        .iter()
        .position(|e| matches!(e, SessionEvent::MapRejected { name, .. } if name == MAP_NAME))
        .expect("the refusal is reported");
    let closed = events
        .iter()
        .position(|e| matches!(e, SessionEvent::Closed(_)))
        .expect("and the session hangs up");
    assert!(rejected < closed, "the reason comes before the hang-up");
    assert!(zone.disconnected, "the zone saw the client leave");
    assert_eq!(client.state(), State::Closed);
    drop(client);

    // The same visit played back: the damaged map is not used, and playback,
    // which has no zone to ask again, neither refuses nor hangs up.
    let bytes = recording.0.lock().unwrap().clone();
    let mut player = ReplayPlayer::from_bytes(&bytes).unwrap();
    let mut replayed = Session::playback(SessionConfig::default(), 0);
    let mut seen = Vec::new();
    let mut t = 0u32;
    while !player.finished() {
        t += 10;
        let due: Vec<_> = player.due(t).to_vec();
        for r in &due {
            replayed.feed(t as u64, r);
        }
        seen.extend(std::iter::from_fn(|| replayed.poll_event()));
    }
    assert!(
        seen.iter()
            .any(|e| matches!(e, SessionEvent::MapNeeded { .. }))
    );
    assert!(!seen.iter().any(|e| matches!(
        e,
        SessionEvent::MapReady { .. } | SessionEvent::MapRejected { .. } | SessionEvent::Closed(_)
    )));
    assert_ne!(replayed.state(), State::Closed);
}

/// A `Write` the test can read back after the session is gone.
#[derive(Clone, Default)]
struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
