//! Choosing the connection's encryption: OpenSec first, VIE as a fallback.
//!
//! OpenSec (oss-core `docs/protocol/opensec.md`) authenticates the server when the
//! client knows its key. Policy here, following the spec's downgrade rule:
//!
//! - **auto** (default): with no key on record for the server, try OpenSec and fall
//!   back to VIE if the server doesn't answer the OpenSec hello in time. With a key
//!   on record, require OpenSec (never fall back: that would be a downgrade).
//! - **opensec**: OpenSec only.
//! - **vie**: VIE only.
//!
//! After an OpenSec connection to a server with no key on record, the server's key
//! is remembered (trust on first use) in a `known_servers.txt` file.
//!
//! The client also offers an identity key (OpenSec `CLIENT_AUTH`), which zones may
//! use for bans. Following the spec's linkability advice (section 4.6) each zone
//! gets its own key, made on first use and kept in `identities.txt`, so zones can't
//! link a player's sessions with each other. A zone that doesn't take identities
//! ignores it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use oss_net::opensec::{IdentityKey, OpenSecClientHandshake, os_entropy};
use oss_net::{Fallback, Handshake, VieClientHandshake};

/// How long an OpenSec hello may go unanswered before falling back to VIE.
pub const FALLBACK_AFTER_MS: u64 = 1500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityMode {
    #[default]
    Auto,
    OpenSec,
    Vie,
}

impl SecurityMode {
    pub fn name(self) -> &'static str {
        match self {
            SecurityMode::Auto => "auto",
            SecurityMode::OpenSec => "opensec",
            SecurityMode::Vie => "vie",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        [SecurityMode::Auto, SecurityMode::OpenSec, SecurityMode::Vie]
            .into_iter()
            .find(|m| m.name().eq_ignore_ascii_case(s))
    }
}

/// The client handshake for `mode`, with the server's key if one is on record and
/// the identity key to offer, if any.
pub fn client_handshake(
    mode: SecurityMode,
    pinned: Option<[u8; 32]>,
    identity: Option<IdentityKey>,
    seed: u32,
) -> Box<dyn Handshake> {
    let entropy = os_entropy();
    let opensec = |pin| {
        let hs = OpenSecClientHandshake::new(&entropy, pin);
        match identity.clone() {
            Some(id) => hs.with_identity(id),
            None => hs,
        }
    };
    match (mode, pinned) {
        (SecurityMode::Vie, _) => Box::new(VieClientHandshake::new(seed)),
        (SecurityMode::OpenSec, pin) | (SecurityMode::Auto, pin @ Some(_)) => {
            Box::new(opensec(pin))
        }
        (SecurityMode::Auto, None) => Box::new(Fallback::new(
            Box::new(opensec(None)),
            Box::new(VieClientHandshake::new(seed)),
            FALLBACK_AFTER_MS,
        )),
    }
}

/// A file of 32-byte keys by "host:port", one `host:port hexkey` per line.
#[derive(Debug, Clone, Default)]
struct KeyFile {
    path: Option<PathBuf>,
    keys: BTreeMap<String, [u8; 32]>,
}

impl KeyFile {
    /// A missing or unreadable file is an empty list; bad lines are skipped.
    fn load(path: &Path) -> Self {
        let mut keys = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
                if let Some((server, key)) = line.split_once(char::is_whitespace)
                    && let Some(k) = unhex(key.trim())
                {
                    keys.insert(server.to_ascii_lowercase(), k);
                }
            }
        }
        Self {
            path: Some(path.to_path_buf()),
            keys,
        }
    }

    fn get(&self, server: &str) -> Option<[u8; 32]> {
        self.keys.get(&server.to_ascii_lowercase()).copied()
    }

    /// Adds or replaces a key and saves the file under `header`.
    fn insert(&mut self, server: &str, key: [u8; 32], header: &str) -> std::io::Result<()> {
        self.keys.insert(server.to_ascii_lowercase(), key);
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut text = String::from(header);
        for (s, k) in &self.keys {
            text += &format!("{s} {}\n", hex(k));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)
    }
}

/// Server keys on record, by "host:port" (trust on first use).
#[derive(Debug, Clone, Default)]
pub struct KnownServers(KeyFile);

fn hex(k: &[u8; 32]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

impl KnownServers {
    /// Reads `path` (lines of `host:port hexkey`); a missing file is an empty list.
    pub fn load(path: &Path) -> Self {
        Self(KeyFile::load(path))
    }

    pub fn get(&self, server: &str) -> Option<[u8; 32]> {
        self.0.get(server)
    }

    /// Records a server's key and saves the file.
    pub fn remember(&mut self, server: &str, key: [u8; 32]) -> std::io::Result<()> {
        self.0.insert(
            server,
            key,
            "# OpenSec server keys (trust on first use): host:port key\n",
        )
    }
}

/// Your OpenSec identity keys, one per zone ("host:port"). The file holds secret
/// keys: anyone with a copy can use your identity at those zones.
#[derive(Debug, Clone, Default)]
pub struct Identities(KeyFile);

impl Identities {
    /// Reads `path` (lines of `host:port hexsecret`); a missing file is empty.
    pub fn load(path: &Path) -> Self {
        Self(KeyFile::load(path))
    }

    /// The identity for `server`, made and saved on first use.
    pub fn for_server(&mut self, server: &str) -> std::io::Result<IdentityKey> {
        if let Some(secret) = self.0.get(server) {
            return Ok(IdentityKey::from_secret(secret));
        }
        let key = IdentityKey::generate(&os_entropy());
        self.0.insert(
            server,
            key.secret_bytes(),
            "# OpenSec identity keys, one per zone: host:port SECRET key.\n\
             # Keep this file private: anyone with a copy can use your identity.\n",
        )?;
        Ok(key)
    }
}

/// A short fingerprint of a public key to show or quote (its first 8 bytes in hex).
pub fn fingerprint(public: &[u8; 32]) -> String {
    hex(public)[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oss_net::opensec::{OpenSecAcceptor, ServerKey};
    use oss_net::{Config, Connection, Event, Server, VieAcceptor};

    /// Connects a client with `handshake` to a server with `acceptors`; returns
    /// the client's cipher name and server key once connected, or None.
    fn connect(
        handshake: Box<dyn Handshake>,
        acceptors: Vec<Box<dyn oss_net::Acceptor>>,
    ) -> Option<(&'static str, Option<[u8; 32]>)> {
        let mut server: Server<u32> = Server::new(Config::server());
        for a in acceptors {
            server.add_acceptor(a);
        }
        let mut client = Connection::client(Config::client(), handshake, 0);
        let mut now = 0;
        while now < 15_000 {
            now += 10;
            client.tick(now);
            while let Some(d) = client.poll_transmit() {
                server.handle_datagram(now, 1, &d);
            }
            server.tick(now);
            while server.poll_event().is_some() {}
            while let Some((_, d)) = server.poll_transmit() {
                client.handle_datagram(now, &d);
            }
            while let Some(ev) = client.poll_event() {
                match ev {
                    Event::Connected => return Some((client.cipher_name()?, client.peer_key())),
                    Event::Disconnected(_) => return None,
                    _ => {}
                }
            }
        }
        None
    }

    fn opensec_server(key: &ServerKey) -> Box<dyn oss_net::Acceptor> {
        Box::new(OpenSecAcceptor::new(Some(key.clone()), os_entropy()))
    }

    #[test]
    fn auto_prefers_opensec_and_learns_the_key() {
        let key = ServerKey::generate(&os_entropy());
        let (cipher, peer) = connect(
            client_handshake(SecurityMode::Auto, None, None, 7),
            vec![opensec_server(&key), Box::new(VieAcceptor::default())],
        )
        .expect("connects");
        assert_eq!(cipher, "opensec");
        assert_eq!(peer, Some(key.public()));
    }

    #[test]
    fn auto_falls_back_to_vie_when_opensec_is_ignored() {
        let (cipher, _) = connect(
            client_handshake(SecurityMode::Auto, None, None, 7),
            vec![Box::new(VieAcceptor::default())],
        )
        .expect("connects over VIE");
        assert_eq!(cipher, "vie");
    }

    #[test]
    fn a_pinned_key_is_required_and_never_downgraded() {
        let key = ServerKey::generate(&os_entropy());
        let other = ServerKey::generate(&os_entropy());
        // Right key: OpenSec.
        let ok = connect(
            client_handshake(SecurityMode::Auto, Some(key.public()), None, 7),
            vec![opensec_server(&key), Box::new(VieAcceptor::default())],
        );
        assert_eq!(ok.map(|c| c.0), Some("opensec"));
        // Wrong key: refused, and no VIE fallback even though the server offers it.
        let bad = connect(
            client_handshake(SecurityMode::Auto, Some(other.public()), None, 7),
            vec![opensec_server(&key), Box::new(VieAcceptor::default())],
        );
        assert_eq!(bad, None);
        // A VIE-only server with a key on record: no downgrade either.
        let vie_only = connect(
            client_handshake(SecurityMode::Auto, Some(key.public()), None, 7),
            vec![Box::new(VieAcceptor::default())],
        );
        assert_eq!(vie_only, None);
    }

    /// Connects with `handshake`, sends one packet, and returns the identity the
    /// server then reports for the client.
    fn identity_seen_by(
        handshake: Box<dyn Handshake>,
        acceptor: Box<dyn oss_net::Acceptor>,
    ) -> Option<[u8; 32]> {
        let mut server: Server<u32> = Server::new(Config::server());
        server.add_acceptor(acceptor);
        let mut client = Connection::client(Config::client(), handshake, 0);
        let mut sent = false;
        for step in 1..1500u64 {
            let now = step * 10;
            client.tick(now);
            while let Some(d) = client.poll_transmit() {
                server.handle_datagram(now, 1, &d);
            }
            server.tick(now);
            while server.poll_event().is_some() {}
            while let Some((_, d)) = server.poll_transmit() {
                client.handle_datagram(now, &d);
            }
            while let Some(ev) = client.poll_event() {
                if matches!(ev, Event::Connected) && !sent {
                    client.send(&[0x09, 1, 2, 3], true);
                    sent = true;
                }
            }
            if let Some(id) = server.connection(&1).and_then(|c| c.peer_key()) {
                return Some(id);
            }
        }
        None
    }

    #[test]
    fn the_zone_learns_the_identity_offered() {
        let key = ServerKey::generate(&os_entropy());
        let me = IdentityKey::generate(&os_entropy());
        let accepting = || -> Box<dyn oss_net::Acceptor> {
            Box::new(OpenSecAcceptor::new(Some(key.clone()), os_entropy()).with_client_identities())
        };
        // Offered in auto mode (inside the VIE fallback) and in OpenSec mode.
        for mode in [SecurityMode::Auto, SecurityMode::OpenSec] {
            let hs = client_handshake(mode, None, Some(me.clone()), 7);
            assert_eq!(identity_seen_by(hs, accepting()), Some(me.public()));
        }
        // Without one the zone sees none; a zone that doesn't take identities
        // still connects.
        let hs = client_handshake(SecurityMode::OpenSec, None, None, 7);
        assert_eq!(identity_seen_by(hs, accepting()), None);
        let plain = connect(
            client_handshake(SecurityMode::Auto, None, Some(me.clone()), 7),
            vec![opensec_server(&key)],
        );
        assert_eq!(plain.map(|c| c.0), Some("opensec"));
    }

    #[test]
    fn identities_are_per_zone_and_kept() {
        let dir = std::env::temp_dir().join(format!("oss-ident-{}", std::process::id()));
        let path = dir.join("identities.txt");
        let _ = std::fs::remove_dir_all(&dir);
        let mut ids = Identities::load(&path);
        let a = ids.for_server("A.example:5000").unwrap();
        let b = ids.for_server("b.example:5000").unwrap();
        assert_ne!(a.public(), b.public());
        let mut again = Identities::load(&path);
        let a2 = again.for_server("a.example:5000").unwrap();
        assert_eq!(a2.public(), a.public());
        assert_eq!(fingerprint(&[0xab; 32]), "abababababababab");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_servers_round_trip() {
        let dir = std::env::temp_dir().join(format!("oss-known-{}", std::process::id()));
        let path = dir.join("known_servers.txt");
        let _ = std::fs::remove_dir_all(&dir);
        let mut k = KnownServers::load(&path);
        assert_eq!(k.get("Zone.Example:5000"), None);
        k.remember("Zone.Example:5000", [7; 32]).unwrap();
        let back = KnownServers::load(&path);
        assert_eq!(back.get("zone.example:5000"), Some([7; 32]));
        assert_eq!(
            SecurityMode::from_name("OpenSec"),
            Some(SecurityMode::OpenSec)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
