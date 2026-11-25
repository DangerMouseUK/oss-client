//! The test zone's `keep_first_handshake`, over real UDP: a zone that keeps a
//! connection's first handshake until it is closed. A second handshake from the
//! same address is answered but changes nothing, so a login sent under it goes
//! unread; after a disconnect, a new handshake from that address works.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use oss_net::{Config, Connection, VieClientHandshake, vie};
use oss_proto::c2s::Login;
use oss_proto::{ClientSettings, FixedStr};
use oss_session::test_zone::TestZone;

/// A tiny map: one wall tile.
fn map() -> Vec<u8> {
    (10u32 | (10 << 12) | (1 << 24)).to_le_bytes().to_vec()
}

struct Rig {
    zone: TestZone,
    socket: UdpSocket,
    start: Instant,
}

impl Rig {
    fn new(keep_first: bool) -> Self {
        let mut zone = TestZone::bind(
            "127.0.0.1:0",
            "tiny.lvl",
            &map(),
            &ClientSettings::default(),
        )
        .expect("bind the zone");
        zone.keep_first_handshake(keep_first);
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind the client");
        socket
            .connect(zone.local_addr().expect("zone address"))
            .expect("connect");
        socket.set_nonblocking(true).expect("nonblocking");
        Self {
            zone,
            socket,
            start: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Sends a bare connection request with `key` and reads the answer's key.
    fn request(&mut self, key: u32) -> Option<u32> {
        let mut req = vec![0, 1];
        req.extend_from_slice(&key.to_le_bytes());
        req.extend_from_slice(&[1, 0]);
        self.socket.send(&req).expect("send");
        let mut buf = [0u8; 600];
        for _ in 0..200 {
            self.zone.pump();
            if let Ok(n) = self.socket.recv(&mut buf)
                && n == 6
                && buf[..2] == [0, 2]
            {
                return Some(u32::from_le_bytes([buf[2], buf[3], buf[4], buf[5]]));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }

    /// Runs `conn` against the zone for `ms` milliseconds of real time.
    fn run(&mut self, conn: &mut Connection, ms: u64) {
        let until = self.now() + ms;
        let mut buf = [0u8; 600];
        while self.now() < until {
            let now = self.now();
            conn.tick(now);
            while let Some(d) = conn.poll_transmit() {
                let _ = self.socket.send(&d);
            }
            self.zone.pump();
            while let Ok(n) = self.socket.recv(&mut buf) {
                conn.handle_datagram(self.now(), &buf[..n]);
            }
            while conn.poll_event().is_some() {}
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A VIE connection with `key` from this rig's address, sending a login
    /// once it is up. Returns the connection, still open.
    fn connect_and_log_in(&mut self, key: u32, name: &str) -> Connection {
        let hs = VieClientHandshake::with_key(key, 0x01);
        let mut conn = Connection::client(Config::client(), Box::new(hs), self.now());
        for _ in 0..50 {
            self.run(&mut conn, 20);
            if conn.cipher_kind().is_some() {
                break;
            }
        }
        assert!(
            conn.cipher_kind().is_some(),
            "the zone answered the handshake"
        );
        let mut login = Vec::new();
        Login {
            name: FixedStr::new(name.as_bytes()),
            client_version: 134,
            ..Login::default()
        }
        .encode(&mut login);
        conn.send(&login, true);
        self.run(&mut conn, 400);
        conn
    }
}

const FIRST: u32 = 0x8000_1234;
const SECOND: u32 = 0x8000_5678;
const THIRD: u32 = 0x8000_9ABC;

/// The flag on: the second handshake is answered as a new one would be, and
/// the login sent under it never reaches the zone. Once the client
/// disconnects, a third handshake from the same address gets in.
#[test]
fn a_second_handshake_does_not_replace_the_first() {
    let mut rig = Rig::new(true);
    assert_eq!(rig.request(FIRST), Some(vie::server_key_for(FIRST)));

    let mut conn = rig.connect_and_log_in(SECOND, "talkspast");
    assert_eq!(
        rig.zone.player_count(),
        0,
        "the login was read with the first handshake's keys and dropped"
    );

    conn.disconnect();
    rig.run(&mut conn, 300);
    let _conn = rig.connect_and_log_in(THIRD, "fresh");
    assert_eq!(
        rig.zone.player_count(),
        1,
        "a new handshake after a close works"
    );
}

/// The flag off (the default): the second handshake opens a new connection in
/// place of the first, as `oss-server` does, and the same sequence logs in.
#[test]
fn by_default_a_second_handshake_replaces_the_first() {
    let mut rig = Rig::new(false);
    assert_eq!(rig.request(FIRST), Some(vie::server_key_for(FIRST)));
    let _conn = rig.connect_and_log_in(SECOND, "replaced");
    assert_eq!(rig.zone.player_count(), 1);
}

/// A first rung the zone answers with something it cannot use: it sends a
/// connection request and ignores the reply, so the zone holds a connection
/// for it and the step down comes on the clock.
struct IgnoresItsAnswer {
    last_sent: Option<u64>,
}

impl oss_net::Handshake for IgnoresItsAnswer {
    fn on_datagram(&mut self, _: u64, _: &[u8], _: &mut Vec<Vec<u8>>) -> oss_net::Step {
        oss_net::Step::Pending
    }

    fn poll(&mut self, now: u64, out: &mut Vec<Vec<u8>>) -> oss_net::Step {
        if self.last_sent.is_none_or(|t| now.saturating_sub(t) >= 1000) {
            let mut req = vec![0, 1];
            req.extend_from_slice(&FIRST.to_le_bytes());
            req.extend_from_slice(&[1, 0]);
            out.push(req);
            self.last_sent = Some(now);
        }
        oss_net::Step::Pending
    }
}

/// The session over `UdpDriver` steps down from a rung the zone answered, and
/// the driver moves to a fresh local port for the next one, so the zone (which
/// keeps the first handshake) meets it as a new client and the login is read.
#[test]
fn the_driver_steps_down_from_a_fresh_port_and_is_heard() {
    use oss_session::{Session, SessionConfig, SessionEvent, udp::UdpDriver};

    let mut zone = TestZone::bind(
        "127.0.0.1:0",
        "tiny.lvl",
        &map(),
        &ClientSettings::default(),
    )
    .expect("bind the zone");
    zone.keep_first_handshake(true);
    let addr = zone.local_addr().expect("zone address").to_string();
    let mut driver = UdpDriver::connect(&addr).expect("socket");
    let first_port = driver.bound_port();
    let hs = oss_net::Fallback::new(
        Box::new(IgnoresItsAnswer { last_sent: None }),
        Box::new(VieClientHandshake::with_key(SECOND, 0x01)),
        300,
    );
    let cfg = SessionConfig {
        name: "stepper".into(),
        ..Default::default()
    };
    let mut session = Session::with_handshake(cfg, driver.now_ms(), Box::new(hs));
    let start = Instant::now();
    let mut logged_in = false;
    while start.elapsed() < Duration::from_secs(5) && !logged_in {
        driver.pump(&mut session).expect("pump");
        zone.pump();
        while let Some(ev) = session.poll_event() {
            logged_in |= ev == SessionEvent::LoggedIn;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(logged_in, "the zone read the login");
    assert_ne!(driver.bound_port(), first_port, "from a new local port");
    assert_eq!(zone.player_count(), 1);
}

/// The client's own automatic order, OpenSec then SubSpace, against a lenient
/// zone that keeps its first handshake: the zone takes the OpenSec hello for a
/// SubSpace connection request and answers it. The OpenSec rung recognises
/// that answer, steps down at once and closes it, and the driver moves to a
/// fresh port, so the SubSpace attempt is heard.
#[test]
fn the_automatic_order_gets_past_a_lenient_zone_that_keeps_its_first_handshake() {
    use oss_session::security::{MechanismRegistry, SecurityMode, client_handshake, context};
    use oss_session::{Session, SessionConfig, SessionEvent, udp::UdpDriver};

    let mut zone = TestZone::bind(
        "127.0.0.1:0",
        "tiny.lvl",
        &map(),
        &ClientSettings::default(),
    )
    .expect("bind the zone");
    zone.keep_first_handshake(true);
    zone.lenient_requests(true);
    let addr = zone.local_addr().expect("zone address").to_string();
    let mut driver = UdpDriver::connect(&addr).expect("socket");
    let first_port = driver.bound_port();
    let hs = client_handshake(
        &MechanismRegistry::builtin(),
        &SecurityMode::Auto,
        &context(0x1234_5678, None, None),
    )
    .expect("the built-ins build");
    let cfg = SessionConfig {
        name: "automatic".into(),
        ..Default::default()
    };
    let mut session = Session::with_negotiation(cfg, driver.now_ms(), hs);
    let start = Instant::now();
    let mut logged_in = false;
    while start.elapsed() < Duration::from_secs(5) && !logged_in {
        driver.pump(&mut session).expect("pump");
        zone.pump();
        while let Some(ev) = session.poll_event() {
            logged_in |= ev == SessionEvent::LoggedIn;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(logged_in, "the zone read the login");
    assert_ne!(driver.bound_port(), first_port, "from a new local port");
    assert!(
        start.elapsed() < Duration::from_millis(1400),
        "stepped down on the answer, not on the clock: {:?}",
        start.elapsed()
    );
}
