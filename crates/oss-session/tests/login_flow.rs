//! End-to-end: a session logs in to a scripted zone through the real transport (VIE
//! encryption, reliable delivery, sized map transfer), enters an arena, downloads the
//! map, answers the security check, sends positions and chats.

use std::io::Write;

use oss_net::{Config, Event, Server, VieAcceptor};
use oss_proto::common::PlayerData;
use oss_proto::s2c::{
    ChatBody, EnteringArena, Green, LoginResponse, MapData, MapFile, MapFilename, PlayerEntering,
    PlayerId, S2CChat, Security,
};
use oss_proto::{C2SPacket, ClientSettings, FixedStr, Packet, checksum};
use oss_session::replay::{Player as ReplayPlayer, Recorder};
use oss_session::{OwnPosition, Session, SessionConfig, SessionEvent, State, chat_type};

const ADDR: u32 = 1;
const KEY: u32 = 0x1234_5678;

/// A tiny map: a few walls, no tileset.
fn map_bytes() -> Vec<u8> {
    let mut v = Vec::new();
    for (x, y, t) in [(10u32, 10u32, 1u32), (11, 10, 1), (512, 500, 5)] {
        v.extend_from_slice(&(x | (y << 12) | (t << 24)).to_le_bytes());
    }
    v
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn player(pid: i16, name: &str, ship: i8) -> PlayerData {
    PlayerData {
        pid,
        name: FixedStr::new(name.as_bytes()),
        ship,
        freq: 0,
        ..Default::default()
    }
}

#[derive(Default)]
struct Zone {
    settings: Vec<u8>,
    got_login: Option<String>,
    security: Option<oss_proto::c2s::SecurityResponse>,
    positions: Vec<oss_proto::C2SPosition>,
    chats: Vec<String>,
    greens: Vec<(i16, i16, i16)>,
    /// Files requested, in order (0 = the map).
    requests: Vec<u16>,
}

/// Stand-in LVZ contents (the session doesn't parse them).
const LVZ: &[u8] = b"CONT fake lvz bytes";

impl Zone {
    fn handle(&mut self, server: &mut Server<u32>, pkt: &[u8]) {
        let mut out = Vec::new();
        let mut send = |server: &mut Server<u32>, p: &dyn Fn(&mut Vec<u8>)| {
            out.clear();
            p(&mut out);
            server.send(&ADDR, &out, true);
        };
        match C2SPacket::decode(pkt).expect("valid C2S") {
            C2SPacket::Login(l) => {
                self.got_login = l.name.to_str().map(str::to_string);
                send(server, &|o| {
                    LoginResponse {
                        code: 0,
                        server_version: 134,
                        ..Default::default()
                    }
                    .encode(o)
                });
            }
            C2SPacket::GoArena(go) => {
                assert_eq!(go.optional_graphics, Some(1), "asks for LVZ files");
                let settings = ClientSettings::default();
                self.settings = settings.to_bytes();
                let s = self.settings.clone();
                let map = map_bytes();
                send(server, &|o| {
                    PlayerId {
                        pid: 5,
                        extra: None,
                    }
                    .encode(o)
                });
                send(server, &|o| o.extend_from_slice(&s));
                send(server, &|o| {
                    PlayerEntering {
                        players: vec![player(5, "tester", 8), player(9, "other", 2)],
                    }
                    .encode(o)
                });
                send(server, &|o| {
                    MapFilename::Files(vec![
                        MapFile {
                            name: FixedStr::new(b"tiny.lvl"),
                            checksum: checksum::crc32(&map),
                            size: 0,
                        },
                        MapFile {
                            name: FixedStr::new(b"art.lvz"),
                            checksum: checksum::crc32(LVZ),
                            size: LVZ.len() as u32 + 17,
                        },
                    ])
                    .encode(o)
                });
                send(server, &|o| EnteringArena {}.encode(o));
                send(server, &|o| {
                    Security {
                        green_seed: 7,
                        door_seed: 9,
                        timestamp: 0,
                        key: KEY,
                    }
                    .encode(o)
                });
            }
            C2SPacket::MapRequest(r) => {
                let index = r.index.unwrap_or(0);
                self.requests.push(index);
                let mut p = Vec::new();
                match index {
                    0 => MapData {
                        name: FixedStr::new(b"tiny.lvl"),
                        data: zlib(&map_bytes()),
                    },
                    _ => MapData {
                        name: FixedStr::new(b"art.lvz"),
                        data: LVZ.to_vec(),
                    },
                }
                .encode(&mut p);
                server.connection_mut(&ADDR).unwrap().send_sized(p);
            }
            C2SPacket::SecurityResponse(r) => self.security = Some(r),
            C2SPacket::Position(p) => self.positions.push(p),
            C2SPacket::TakeGreen(g) => {
                self.greens.push((g.x, g.y, g.prize));
                // Tell the client someone else took one too.
                let other = Green {
                    time: g.time,
                    x: 40,
                    y: 41,
                    prize: 3,
                    pid: 9,
                };
                send(server, &|o| other.encode(o));
            }
            C2SPacket::Chat(c) => {
                let text = String::from_utf8_lossy(&c.0.text).into_owned();
                self.chats.push(text.clone());
                let reply = S2CChat(ChatBody {
                    kind: chat_type::PUBLIC,
                    sound: 0,
                    pid: 9,
                    text: b"hi tester".to_vec(),
                });
                send(server, &|o| reply.encode(o));
            }
            _ => {}
        }
    }
}

#[test]
fn logs_in_enters_arena_downloads_map_and_plays() {
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
    // Record the whole visit into memory, to play it back afterwards.
    let recording = Shared::default();
    client.record_to(
        Recorder::new(Box::new(recording.clone()), now, &[("player", "tester")]).unwrap(),
        now,
    );
    let mut zone = Zone::default();
    let mut events = Vec::new();
    let mut map_ready = false;
    let mut sent_chat = false;

    for _ in 0..2000 {
        now += 10;
        client.tick(now);
        while let Some(d) = client.poll_transmit() {
            server.handle_datagram(now, ADDR, &d);
        }
        server.tick(now);
        while let Some((_, ev)) = server.poll_event() {
            if let Event::Packet(p) = ev {
                zone.handle(&mut server, &p);
            }
        }
        while let Some((_, d)) = server.poll_transmit() {
            client.handle_datagram(now, &d);
        }
        while let Some(ev) = client.poll_event() {
            match &ev {
                SessionEvent::MapNeeded { name, .. } => {
                    assert_eq!(name, "tiny.lvl");
                    assert!(
                        !client.use_cached_map(b"stale"),
                        "wrong CRC must be rejected"
                    );
                    client.download_map();
                }
                SessionEvent::FilesListed(files) => {
                    assert_eq!(files.len(), 1);
                    assert_eq!((files[0].index, files[0].name.as_str()), (1, "art.lvz"));
                    assert!(!client.use_cached_file("art.lvz", b"stale"));
                    client.download_file("art.lvz");
                }
                SessionEvent::MapReady {
                    data, downloaded, ..
                } => {
                    assert!(downloaded);
                    assert_eq!(data, &map_bytes());
                    map_ready = true;
                }
                _ => {}
            }
            events.push(ev);
        }
        if client.state() == State::Playing && client.position_due(now) {
            let pos = OwnPosition {
                x: 8200,
                y: 8100,
                x_speed: 50,
                rotation: 10,
                energy: 1000,
                ..Default::default()
            };
            client.send_position(now, &pos);
        }
        if client.state() == State::Playing && !sent_chat {
            client.send_chat(chat_type::PUBLIC, 0, "hello zone");
            client.send_take_green(now, 512, 513, -7);
            sent_chat = true;
        }
        if zone.positions.len() >= 3
            && events
                .iter()
                .any(|e| matches!(e, SessionEvent::Chat { .. }))
            && events
                .iter()
                .any(|e| matches!(e, SessionEvent::GreenTaken { .. }))
        {
            break;
        }
    }

    assert_eq!(zone.got_login.as_deref(), Some("tester"));
    assert!(map_ready);
    assert_eq!(client.state(), State::Playing);
    assert_eq!(client.pid(), Some(5));
    assert_eq!(client.player(9).map(|p| p.name.as_str()), Some("other"));
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::Seeds {
            green_seed: 7,
            door_seed: 9,
            ..
        }
    )));

    let sec = zone.security.expect("security response");
    assert_eq!(
        sec.settings_checksum,
        checksum::settings_checksum(KEY, &zone.settings)
    );
    assert_eq!(sec.exe_checksum, checksum::vie_exe_checksum(KEY));
    let level = oss_formats::Level::parse(&map_bytes()).unwrap();
    let grid = level.tile_grid();
    assert_eq!(
        sec.map_checksum,
        checksum::map_checksum(KEY, |x, y| grid.get(x as u32, y as u32))
    );

    assert!(zone.positions.len() >= 3);
    let p = &zone.positions[0];
    assert_eq!((p.x, p.y, p.x_speed, p.rotation), (8200, 8100, 50, 10));
    // Timestamps strictly increase.
    assert!(
        zone.positions
            .windows(2)
            .all(|w| (w[1].time.wrapping_sub(w[0].time) as i32) > 0)
    );
    assert_eq!(zone.chats, vec!["hello zone".to_string()]);
    // The map first, then the LVZ, one transfer at a time.
    assert_eq!(zone.requests, vec![0, 1]);
    assert!(!client.downloads_pending());
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::FileReady { name, data, downloaded: true } if name == "art.lvz" && data == LVZ
    )));
    assert_eq!(zone.greens, vec![(512, 513, -7)]);
    assert!(events.contains(&SessionEvent::GreenTaken {
        pid: 9,
        x: 40,
        y: 41,
        prize: 3
    }));
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::Chat { pid: 9, name: Some(n), text, .. } if n == "other" && text == "hi tester"
    )));

    // Play the recording back: same arena, players, files and chat, and the
    // recorder's own movement shows up as position updates.
    client.flush(now);
    drop(client);
    let bytes = recording.0.lock().unwrap().clone();
    let mut player = ReplayPlayer::from_bytes(&bytes).unwrap();
    assert_eq!(player.replay.meta_value("player"), Some("tester"));
    assert_eq!(player.file("TINY.LVL"), Some(&map_bytes()[..]));
    assert_eq!(player.file("art.lvz"), Some(LVZ));
    let mut replayed = Session::playback(SessionConfig::default(), 0);
    let mut seen = Vec::new();
    let mut t = 0u32;
    while !player.finished() {
        t += 10;
        let due: Vec<_> = player.due(t).to_vec();
        for r in &due {
            replayed.feed(t as u64, r);
        }
        while let Some(ev) = replayed.poll_event() {
            if let SessionEvent::MapNeeded { name, .. } = &ev {
                assert!(replayed.use_cached_map(player.file(name).unwrap()));
            }
            if let SessionEvent::FilesListed(files) = &ev {
                for f in files {
                    assert!(replayed.use_cached_file(&f.name, player.file(&f.name).unwrap()));
                }
            }
            seen.push(ev);
        }
    }
    assert_eq!(replayed.state(), State::Playing);
    assert_eq!(replayed.pid(), Some(5));
    assert_eq!(replayed.player(9).map(|p| p.name.as_str()), Some("other"));
    assert!(
        seen.iter()
            .any(|e| matches!(e, SessionEvent::PlayerMoved(5)))
    );
    assert_eq!(
        replayed.me().and_then(|p| p.position).map(|p| (p.x, p.y)),
        Some((8200, 8100))
    );
    assert!(seen.iter().any(|e| matches!(
        e,
        SessionEvent::Chat { pid: 9, text, .. } if text == "hi tester"
    )));
    // The replay's clock follows the recorded server time.
    assert!(replayed.server_ticks(t as u64) > 0);
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

#[test]
fn rejected_login_is_reported() {
    let mut now = 0u64;
    let mut server = Server::new(Config::server());
    server.add_acceptor(Box::new(VieAcceptor::default()));
    let mut client = Session::new(SessionConfig::default(), now, 1);
    let mut failed = None;
    for _ in 0..500 {
        now += 10;
        client.tick(now);
        while let Some(d) = client.poll_transmit() {
            server.handle_datagram(now, ADDR, &d);
        }
        server.tick(now);
        while let Some((_, ev)) = server.poll_event() {
            if let Event::Packet(p) = ev
                && p[0] == 0x09
            {
                let mut out = Vec::new();
                LoginResponse {
                    code: 2,
                    ..Default::default()
                }
                .encode(&mut out);
                server.send(&ADDR, &out, true);
            }
        }
        while let Some((_, d)) = server.poll_transmit() {
            client.handle_datagram(now, &d);
        }
        while let Some(ev) = client.poll_event() {
            if let SessionEvent::LoginFailed { message, .. } = ev {
                failed = Some(message);
            }
        }
        if failed.is_some() {
            break;
        }
    }
    assert_eq!(failed, Some("wrong password"));
}
