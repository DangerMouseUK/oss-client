//! A minimal relay "zone" for local testing of clients and bots: logs players in
//! (any name/password), puts everyone in one arena, serves one map and settings,
//! relays positions, weapons, ship changes and chat, and sends security seeds.
//!
//! It is **not** a game server (no scoring, no kills, no validation) — `oss-server`
//! is. It exists so the client and `oss-bot` can be exercised over real UDP before
//! the real server's network transport is wired up.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use oss_net::{Config, Event, Server, VieAcceptor};
use oss_proto::c2s::GoArena;
use oss_proto::common::PlayerData;
use oss_proto::s2c::{
    ChatBody, EnteringArena, Kill, LoginResponse, MapData, MapFilename, PlayerEntering, PlayerId,
    PlayerLeaving, S2CChat, Security, ShipChange,
};
use oss_proto::{
    C2SPacket, C2SPosition, ClientSettings, FixedStr, Packet, PositionTail, S2CPosition, S2CWeapon,
    checksum,
};

use crate::SPECTATOR;

struct ZonePlayer {
    pid: i16,
    name: String,
    ship: u8,
    freq: i16,
    in_arena: bool,
}

pub struct TestZone {
    socket: UdpSocket,
    net: Server<SocketAddr>,
    players: BTreeMap<SocketAddr, ZonePlayer>,
    next_pid: i16,
    settings: Vec<u8>,
    map_name: String,
    map_crc: u32,
    map_packet: Vec<u8>,
    start: Instant,
    seed: u32,
    buf: Vec<u8>,
}

impl TestZone {
    /// Binds `addr` and serves `map_lvl` (an uncompressed `.lvl`) as `map_name`.
    pub fn bind(
        addr: &str,
        map_name: &str,
        map_lvl: &[u8],
        settings: &ClientSettings,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        let mut net = Server::new(Config::server());
        net.add_acceptor(Box::new(VieAcceptor::default()));
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(map_lvl)?;
        let mut map_packet = Vec::new();
        MapData {
            name: FixedStr::new(map_name.as_bytes()),
            data: z.finish()?,
        }
        .encode(&mut map_packet);
        Ok(Self {
            socket,
            net,
            players: BTreeMap::new(),
            next_pid: 0,
            settings: settings.to_bytes(),
            map_name: map_name.to_string(),
            map_crc: checksum::crc32(map_lvl),
            map_packet,
            start: Instant::now(),
            seed: 0x5EED_0001,
            buf: vec![0; 2048],
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn ticks(&self) -> u32 {
        oss_net::ticks(self.now())
    }

    /// Handles everything waiting and sends replies. Never blocks.
    pub fn pump(&mut self) {
        loop {
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, from)) => {
                    let now = self.now();
                    let data = self.buf[..n].to_vec();
                    self.net.handle_datagram(now, from, &data);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused
                    ) =>
                {
                    continue;
                }
                Err(e) => {
                    log::warn!("test zone socket: {e}");
                    break;
                }
            }
        }
        let now = self.now();
        self.net.tick(now);
        while let Some((from, ev)) = self.net.poll_event() {
            match ev {
                Event::Packet(p) => self.handle(from, &p),
                Event::Disconnected(_) => self.leave(from),
                _ => {}
            }
        }
        while let Some((to, d)) = self.net.poll_transmit() {
            let _ = self.socket.send_to(&d, to);
        }
    }

    fn send(&mut self, to: SocketAddr, p: &impl Packet, reliable: bool) {
        self.buf.clear();
        p.encode(&mut self.buf);
        let data = std::mem::take(&mut self.buf);
        self.net.send(&to, &data, reliable);
        self.buf = data;
        self.buf.resize(2048, 0);
    }

    fn send_raw(&mut self, to: SocketAddr, data: &[u8]) {
        self.net.send(&to, data, true);
    }

    fn others(&self, me: SocketAddr) -> Vec<SocketAddr> {
        self.players
            .iter()
            .filter(|(a, p)| **a != me && p.in_arena)
            .map(|(a, _)| *a)
            .collect()
    }

    fn player_data(p: &ZonePlayer) -> PlayerData {
        PlayerData {
            pid: p.pid,
            name: FixedStr::new(p.name.as_bytes()),
            ship: p.ship as i8,
            freq: p.freq,
            attached_to: -1,
            ..Default::default()
        }
    }

    fn handle(&mut self, from: SocketAddr, pkt: &[u8]) {
        let Ok(packet) = C2SPacket::decode(pkt) else {
            return;
        };
        match packet {
            C2SPacket::Login(l) => {
                let pid = self.next_pid;
                self.next_pid += 1;
                let name = l.name.to_str().unwrap_or("player").trim().to_string();
                log::info!("test zone: {name} logged in from {from} as pid {pid}");
                // Two public teams, alternating, so players can fight.
                let freq = pid % 2;
                self.players.insert(
                    from,
                    ZonePlayer {
                        pid,
                        name,
                        ship: SPECTATOR,
                        freq,
                        in_arena: false,
                    },
                );
                self.send(
                    from,
                    &LoginResponse {
                        code: 0,
                        server_version: 134,
                        ..Default::default()
                    },
                    true,
                );
            }
            C2SPacket::GoArena(go) => self.enter(from, &go),
            C2SPacket::MapRequest(_) => {
                let p = self.map_packet.clone();
                if let Some(c) = self.net.connection_mut(&from) {
                    c.send_sized(p);
                }
            }
            C2SPacket::Position(p) => self.relay_position(from, &p),
            C2SPacket::Chat(c) => {
                let Some(me) = self.players.get(&from) else {
                    return;
                };
                let out = S2CChat(ChatBody {
                    kind: c.0.kind,
                    sound: c.0.sound,
                    pid: me.pid,
                    text: c.0.text,
                });
                for to in self.others(from) {
                    self.send(to, &out, true);
                }
            }
            C2SPacket::Death(d) => {
                let Some(me) = self.players.get(&from) else {
                    return;
                };
                log::info!(
                    "test zone: {} (pid {}) killed by pid {}",
                    me.name,
                    me.pid,
                    d.killer
                );
                let out = Kill {
                    green: 0,
                    killer: d.killer,
                    killed: me.pid,
                    bounty: d.bounty,
                    flags: 0,
                };
                let all: Vec<SocketAddr> = self
                    .players
                    .iter()
                    .filter(|(_, p)| p.in_arena)
                    .map(|(a, _)| *a)
                    .collect();
                for to in all {
                    self.send(to, &out, true);
                }
            }
            C2SPacket::SetShip(s) => {
                let Some(me) = self.players.get_mut(&from) else {
                    return;
                };
                me.ship = s.ship.min(SPECTATOR);
                let out = ShipChange {
                    ship: me.ship as i8,
                    pid: me.pid,
                    freq: me.freq,
                };
                let all: Vec<SocketAddr> = self
                    .players
                    .iter()
                    .filter(|(_, p)| p.in_arena)
                    .map(|(a, _)| *a)
                    .collect();
                for to in all {
                    self.send(to, &out, true);
                }
            }
            _ => {}
        }
    }

    fn enter(&mut self, from: SocketAddr, go: &GoArena) {
        let Some(me) = self.players.get_mut(&from) else {
            return;
        };
        me.ship = go.ship.min(SPECTATOR);
        me.in_arena = true;
        let pid = me.pid;
        let me_data = Self::player_data(me);
        let everyone: Vec<PlayerData> = self
            .players
            .values()
            .filter(|p| p.in_arena)
            .map(Self::player_data)
            .collect();
        self.send(from, &PlayerId { pid, arena: None }, true);
        let settings = self.settings.clone();
        self.send_raw(from, &settings);
        self.send(from, &PlayerEntering { players: everyone }, true);
        let filename = MapFilename::Single {
            name: FixedStr::new(self.map_name.as_bytes()),
            checksum: self.map_crc,
        };
        self.send(from, &filename, true);
        self.send(from, &EnteringArena {}, true);
        self.seed = self.seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let sec = Security {
            green_seed: self.seed,
            door_seed: self.seed >> 3,
            timestamp: self.ticks(),
            key: self.seed | 1,
        };
        self.send(from, &sec, true);
        for to in self.others(from) {
            self.send(
                to,
                &PlayerEntering {
                    players: vec![me_data.clone()],
                },
                true,
            );
        }
    }

    fn relay_position(&mut self, from: SocketAddr, p: &C2SPosition) {
        let Some(me) = self.players.get(&from) else {
            return;
        };
        let pid = me.pid;
        let time = p.time as u16;
        let small = p.weapon.kind() == 0 && (0..256).contains(&pid) && p.bounty < 256;
        let targets = self.others(from);
        if small {
            let out = S2CPosition {
                rotation: p.rotation,
                time,
                x: p.x,
                c2s_latency: 0,
                bounty: p.bounty as u8,
                player_id: pid as u8,
                status: p.status,
                y_speed: p.y_speed,
                y: p.y,
                x_speed: p.x_speed,
                tail: PositionTail::Energy(p.energy.max(0) as u16),
            };
            for to in targets {
                self.send(to, &out, false);
            }
        } else {
            let out = S2CWeapon {
                rotation: p.rotation,
                time,
                x: p.x,
                y_speed: p.y_speed,
                player_id: pid as u16,
                x_speed: p.x_speed,
                checksum: 0,
                status: p.status,
                c2s_latency: 0,
                y: p.y,
                bounty: p.bounty,
                weapon: p.weapon,
                tail: PositionTail::None,
            };
            for to in targets {
                self.send(to, &out, p.weapon.kind() != 0);
            }
        }
    }

    fn leave(&mut self, from: SocketAddr) {
        if let Some(p) = self.players.remove(&from) {
            log::info!("test zone: {} left", p.name);
            if p.in_arena {
                for to in self.others(from) {
                    self.send(to, &PlayerLeaving { pid: p.pid }, true);
                }
            }
        }
    }
}
