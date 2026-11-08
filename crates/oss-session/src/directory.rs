//! The zone list from a directory server, and zone pings for population and latency.
//!
//! Clients reach a directory over the normal transport (VIE handshake, reliable
//! delivery) on its port 4990, send a zone list request (type `0x01`, minimum
//! players) and get back the list, usually as big/sized data. Each zone also answers a
//! 4-byte ping on its game port + 1 with its population. See oss-core's
//! `docs/protocol/directory.md`.
//!
//! Only directory servers the player configures are ever contacted.

use std::io::ErrorKind;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Instant;

use oss_net::{Config, Connection, Event as NetEvent, VieClientHandshake};
use oss_proto::directory::{ZoneList, ZoneListRequest};

/// One zone, as the browser shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneInfo {
    pub name: String,
    pub description: String,
    /// "a.b.c.d:port" of the game server.
    pub address: String,
    /// Players, as the directory last heard (pings refresh it).
    pub players: u32,
    /// Round trip of the last ping, if answered.
    pub ping_ms: Option<u32>,
}

fn zone_info(e: &oss_proto::directory::ZoneEntry) -> ZoneInfo {
    let text = |b: &[u8]| {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).trim().to_string()
    };
    let [a, b, c, d] = e.ip;
    ZoneInfo {
        name: text(e.name.as_bytes()),
        description: text(&e.description),
        address: format!("{a}.{b}.{c}.{d}:{}", e.port),
        players: e.players as u32,
        ping_ms: None,
    }
}

/// Sans-IO zone list request over one transport connection.
pub struct ZoneListFetch {
    conn: Connection,
    min_players: u32,
    asked: bool,
    result: Option<Result<Vec<ZoneInfo>, String>>,
}

impl ZoneListFetch {
    pub fn new(now_ms: u64, seed: u32, min_players: u32) -> Self {
        Self {
            conn: Connection::client(
                Config::client(),
                Box::new(VieClientHandshake::new(seed)),
                now_ms,
            ),
            min_players,
            asked: false,
            result: None,
        }
    }

    pub fn handle_datagram(&mut self, now_ms: u64, data: &[u8]) {
        self.conn.handle_datagram(now_ms, data);
        self.drain();
    }

    pub fn tick(&mut self, now_ms: u64) {
        self.conn.tick(now_ms);
        self.drain();
        self.conn.flush(now_ms);
    }

    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        self.conn.poll_transmit()
    }

    /// The zones, or why there are none; `None` while still waiting.
    pub fn result(&self) -> Option<&Result<Vec<ZoneInfo>, String>> {
        self.result.as_ref()
    }

    fn drain(&mut self) {
        while let Some(ev) = self.conn.poll_event() {
            match ev {
                NetEvent::Connected if !self.asked => {
                    let mut p = Vec::new();
                    ZoneListRequest {
                        min_players: self.min_players,
                    }
                    .encode(&mut p);
                    self.conn.send(&p, true);
                    self.asked = true;
                }
                NetEvent::Packet(data) if self.result.is_none() => {
                    if data.first() == Some(&ZoneList::TYPE) {
                        self.result = Some(
                            ZoneList::decode(&data)
                                .map(|l| l.zones.iter().map(zone_info).collect())
                                .map_err(|e| format!("bad zone list: {e}")),
                        );
                        self.conn.disconnect();
                    }
                }
                NetEvent::Disconnected(reason) if self.result.is_none() => {
                    self.result = Some(Err(format!("directory closed ({reason:?})")));
                }
                _ => {}
            }
        }
    }
}

/// Fetches the zone list from a directory over UDP, without blocking: call
/// [`DirectoryClient::pump`] every frame until [`DirectoryClient::result`] is ready.
pub struct DirectoryClient {
    fetch: ZoneListFetch,
    socket: UdpSocket,
    start: Instant,
    buf: Vec<u8>,
}

impl DirectoryClient {
    /// Starts asking `server` ("host:port", normally port 4990).
    pub fn connect(server: &str, min_players: u32) -> std::io::Result<Self> {
        let addr = server.to_socket_addrs()?.next().ok_or_else(|| {
            std::io::Error::new(ErrorKind::NotFound, "directory address did not resolve")
        })?;
        let socket = UdpSocket::bind(any_for(addr))?;
        socket.connect(addr)?;
        socket.set_nonblocking(true)?;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.subsec_nanos() | 1);
        Ok(Self {
            fetch: ZoneListFetch::new(0, seed, min_players),
            socket,
            start: Instant::now(),
            buf: vec![0; 2048],
        })
    }

    pub fn pump(&mut self) -> std::io::Result<()> {
        loop {
            match self.socket.recv(&mut self.buf) {
                Ok(n) => {
                    let now = self.start.elapsed().as_millis() as u64;
                    self.fetch.handle_datagram(now, &self.buf[..n]);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => break,
                Err(e) => return Err(e),
            }
        }
        self.fetch.tick(self.start.elapsed().as_millis() as u64);
        while let Some(d) = self.fetch.poll_transmit() {
            match self.socket.send(&d) {
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn result(&self) -> Option<&Result<Vec<ZoneInfo>, String>> {
        self.fetch.result()
    }
}

fn any_for(addr: SocketAddr) -> SocketAddr {
    if addr.is_ipv4() {
        SocketAddr::from(([0, 0, 0, 0], 0))
    } else {
        SocketAddr::from(([0u16; 8], 0))
    }
}

/// Pings zones on their game port + 1 (population and latency), without blocking.
pub struct ZonePinger {
    socket: UdpSocket,
    start: Instant,
    /// Ping port per zone (index into the list given to [`ZonePinger::ping`]).
    targets: Vec<Option<SocketAddr>>,
}

impl ZonePinger {
    pub fn new() -> std::io::Result<Self> {
        let socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0)))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            start: Instant::now(),
            targets: Vec::new(),
        })
    }

    /// Sends a ping to every zone (their game port + 1).
    pub fn ping(&mut self, zones: &[ZoneInfo]) {
        self.targets = zones
            .iter()
            .map(|z| {
                let addr: SocketAddr = z.address.parse().ok()?;
                Some(SocketAddr::new(addr.ip(), addr.port().checked_add(1)?))
            })
            .collect();
        let stamp = self.start.elapsed().as_millis() as u32;
        for t in self.targets.iter().flatten() {
            let _ = self.socket.send_to(&stamp.to_le_bytes(), t);
        }
    }

    /// Reads pongs and updates population and ping in `zones`.
    pub fn update(&mut self, zones: &mut [ZoneInfo]) {
        let mut buf = [0u8; 64];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((8, from)) => {
                    let players = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                    let sent = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
                    let now = self.start.elapsed().as_millis() as u32;
                    if let Some(i) = self.targets.iter().position(|t| *t == Some(from))
                        && let Some(z) = zones.get_mut(i)
                    {
                        z.players = players;
                        z.ping_ms = Some(now.wrapping_sub(sent));
                    }
                }
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }
}

/// A list of zones as a directory would send it (for test directories).
pub fn encode_zone_list(zones: &[(String, String, [u8; 4], u16, u16)]) -> Vec<u8> {
    let list = ZoneList {
        zones: zones
            .iter()
            .map(
                |(name, desc, ip, port, players)| oss_proto::directory::ZoneEntry {
                    ip: *ip,
                    port: *port,
                    players: *players,
                    score_keeping: 1,
                    hop_count: 0,
                    version: 134,
                    name: oss_proto::FixedStr::new(name.as_bytes()),
                    description: desc.as_bytes().to_vec(),
                },
            )
            .collect(),
    };
    let mut out = Vec::new();
    list.encode(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oss_net::{Server, VieAcceptor};

    #[test]
    fn fetches_a_big_zone_list_through_the_transport() {
        let mut server: Server<u32> = Server::new(Config::server());
        server.add_acceptor(Box::new(VieAcceptor::default()));
        // Enough zones that the list is bigger than one datagram.
        let zones: Vec<_> = (0..40)
            .map(|i| {
                (
                    format!("Zone {i}"),
                    format!("A test zone with a description, number {i}"),
                    [127, 0, 0, 1],
                    5000 + i as u16,
                    i as u16,
                )
            })
            .collect();
        let list = encode_zone_list(&zones);
        assert!(list.len() > 1000);
        let mut fetch = ZoneListFetch::new(0, 0x1234, 0);
        let mut now = 0;
        while fetch.result().is_none() && now < 20_000 {
            now += 10;
            fetch.tick(now);
            while let Some(d) = fetch.poll_transmit() {
                server.handle_datagram(now, 1, &d);
            }
            server.tick(now);
            while let Some((from, ev)) = server.poll_event() {
                if let oss_net::Event::Packet(p) = ev {
                    assert_eq!(p, [1, 0, 0, 0, 0], "zone list request, min 0");
                    if let Some(c) = server.connection_mut(&from) {
                        c.send_sized(list.clone());
                    }
                }
            }
            while let Some((_, d)) = server.poll_transmit() {
                fetch.handle_datagram(now, &d);
            }
        }
        let got = fetch.result().expect("an answer").as_ref().expect("a list");
        assert_eq!(got.len(), 40);
        assert_eq!(got[7].name, "Zone 7");
        assert_eq!(got[7].address, "127.0.0.1:5007");
        assert_eq!(got[7].players, 7);
        assert!(got[39].description.ends_with("number 39"));
    }
}
