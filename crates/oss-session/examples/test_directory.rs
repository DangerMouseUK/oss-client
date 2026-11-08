//! A minimal local directory server for testing the zone browser:
//!
//! ```text
//! cargo run -p oss-session --example test_directory -- [--bind 127.0.0.1:4990] \
//!     "Local Zone=127.0.0.1:5000" "Another=127.0.0.1:5077"
//! ```
//!
//! Answers zone list requests with the zones given on the command line (players are
//! filled in by the client's own pings). It only lists what you give it, binds to
//! localhost by default, and never contacts anything itself. Not a real directory:
//! there is no zone registration.

use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use oss_net::{Config, Event, Server, VieAcceptor};
use oss_session::directory::encode_zone_list;

fn main() {
    let mut bind = "127.0.0.1:4990".to_string();
    let mut zones = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--bind" {
            bind = args.next().unwrap_or(bind);
            continue;
        }
        let Some((name, addr)) = a.split_once('=') else {
            eprintln!("zones are \"Name=host:port\", got {a}");
            std::process::exit(2);
        };
        let Ok(addr) = addr.parse::<SocketAddr>() else {
            eprintln!("{addr}: not an IPv4 host:port");
            std::process::exit(2);
        };
        let SocketAddr::V4(v4) = addr else {
            eprintln!("{addr}: directories list IPv4 zones only");
            std::process::exit(2);
        };
        zones.push((
            name.to_string(),
            format!("{name} (local test directory)"),
            v4.ip().octets(),
            v4.port(),
            0u16,
        ));
    }
    let list = encode_zone_list(&zones);
    let socket = match UdpSocket::bind(&bind) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    let _ = socket.set_nonblocking(true);
    eprintln!("test directory on {bind} listing {} zone(s)", zones.len());
    let mut server: Server<SocketAddr> = Server::new(Config::server());
    server.add_acceptor(Box::new(VieAcceptor::default()));
    let start = Instant::now();
    let mut buf = vec![0u8; 2048];
    loop {
        let now = start.elapsed().as_millis() as u64;
        while let Ok((n, from)) = socket.recv_from(&mut buf) {
            server.handle_datagram(now, from, &buf[..n]);
        }
        server.tick(now);
        while let Some((from, ev)) = server.poll_event() {
            if let Event::Packet(p) = ev
                && p.first() == Some(&1)
                && let Some(c) = server.connection_mut(&from)
            {
                eprintln!("zone list for {from}");
                c.send_sized(list.clone());
            }
        }
        while let Some((to, d)) = server.poll_transmit() {
            let _ = socket.send_to(&d, to);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
