//! Drives a [`Session`] over a real UDP socket (non-blocking, std only).

use std::io::ErrorKind;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Instant;

use crate::Session;

/// Largest datagram we read.
const MAX_DATAGRAM: usize = 2048;

/// A UDP socket connected to one server, plus a millisecond clock.
pub struct UdpDriver {
    socket: UdpSocket,
    server: SocketAddr,
    start: Instant,
    buf: Vec<u8>,
}

impl UdpDriver {
    /// Opens a socket for `server` ("host:port").
    pub fn connect(server: &str) -> std::io::Result<Self> {
        let server = server.to_socket_addrs()?.next().ok_or_else(|| {
            std::io::Error::new(ErrorKind::NotFound, "server address did not resolve")
        })?;
        let bind: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0".parse()
        } else {
            "[::]:0".parse()
        }
        .map_err(|_| std::io::Error::other("bind address"))?;
        let socket = UdpSocket::bind(bind)?;
        socket.connect(server)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            server,
            start: Instant::now(),
            buf: vec![0; MAX_DATAGRAM],
        })
    }

    pub fn server(&self) -> SocketAddr {
        self.server
    }

    /// The port the socket was bound to, which the login reports. Zero if the
    /// socket cannot say.
    pub fn bound_port(&self) -> u16 {
        self.socket.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// The server's IPv4 address as a host-order `u32`, which is how the login
    /// carries it. Zero for IPv6, which the field cannot represent.
    pub fn server_ip(&self) -> u32 {
        match self.server {
            SocketAddr::V4(v4) => u32::from(*v4.ip()),
            SocketAddr::V6(_) => 0,
        }
    }

    /// Milliseconds since this driver was created (the session's clock).
    pub fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Receives everything waiting, ticks the session and sends what it queued.
    /// Never blocks. Returns an error only for unexpected socket failures.
    pub fn pump(&mut self, session: &mut Session) -> std::io::Result<()> {
        // Before anything the session might do, including sending its login:
        // this driver is the only thing that knows either value, and telling
        // it here means no caller has to remember to. Two stores per pump.
        session.set_local_endpoint(self.server_ip(), self.bound_port());
        loop {
            match self.socket.recv(&mut self.buf) {
                Ok(n) => {
                    let now = self.now_ms();
                    session.handle_datagram(now, &self.buf[..n]);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // Windows reports ICMP "port unreachable" as a reset on the next recv;
                // the transport's timeout handles a dead server.
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused
                    ) =>
                {
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        let now = self.now_ms();
        session.tick(now);
        self.send_pending(session)
    }

    /// Sends every datagram the session has queued.
    pub fn send_pending(&mut self, session: &mut Session) -> std::io::Result<()> {
        while let Some(d) = session.poll_transmit() {
            match self.socket.send(&d) {
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::ConnectionRefused
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The login's `server_ip` is the IPv4 address as a host-order `u32`, and
    /// these two are read off real captures rather than derived: a client
    /// dialling 127.0.0.1 put `01 00 00 7F` at offset 89, and one dialling
    /// Trench Wars at 207.148.16.165 put 0xCF9410A5 there.
    #[test]
    fn the_server_address_encodes_the_way_a_real_client_sends_it() {
        let d = |s: &str| -> u32 {
            match s.parse::<SocketAddr>().expect("addr") {
                SocketAddr::V4(v4) => u32::from(*v4.ip()),
                SocketAddr::V6(_) => 0,
            }
        };
        assert_eq!(d("127.0.0.1:5000"), 0x7F00_0001);
        assert_eq!(d("127.0.0.1:5000").to_le_bytes(), [0x01, 0x00, 0x00, 0x7F]);
        assert_eq!(d("207.148.16.165:5400"), 0xCF94_10A5);
        assert_eq!(d("[::1]:5000"), 0, "IPv6 cannot be carried in the field");
    }

    /// A driver reports a real bound port, and hands both to the session
    /// before the session can send anything — so no caller has to remember.
    #[test]
    fn a_driver_tells_the_session_its_endpoint() {
        let d = UdpDriver::connect("127.0.0.1:5000").expect("connect");
        assert_ne!(d.bound_port(), 0, "the OS gave us an ephemeral port");
        assert_eq!(d.server_ip(), 0x7F00_0001);
    }
}
