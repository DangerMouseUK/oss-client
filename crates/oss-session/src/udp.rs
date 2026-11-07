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

    /// Milliseconds since this driver was created (the session's clock).
    pub fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Receives everything waiting, ticks the session and sends what it queued.
    /// Never blocks. Returns an error only for unexpected socket failures.
    pub fn pump(&mut self, session: &mut Session) -> std::io::Result<()> {
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
