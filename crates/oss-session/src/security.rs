//! Choosing the connection's encryption from the mechanisms this build has.
//!
//! Every security mechanism a build can speak is described once, as an
//! [`oss_net::Mechanism`], and kept in a [`MechanismRegistry`]. This repository's
//! own are the built-in ones ([`MechanismRegistry::builtin`]): None, Subspace (the
//! SubSpace cipher, `vie`) and OpenSecurity (`opensec`, oss-core
//! `docs/protocol/opensec.md`). A build outside this repository adds its own by
//! registering them; nothing here names them, and everything below reads the
//! registry rather than a list of its own.
//!
//! What a person chooses is a [`SecurityMode`]:
//!
//! - **auto** (default): the automatic order, every mechanism that may be tried on
//!   its own, strongest first, stepping down to the next when a server does not
//!   answer within [`FALLBACK_AFTER_MS`]. With the built-ins that is OpenSec, then
//!   VIE. **With a server's key on record it is that key's mechanism alone**, never
//!   stepped down from: that would be a downgrade.
//! - **a mechanism by name**: that one and nothing else, whatever key is on record.
//!   A person asking is an explicit policy, and it takes precedence over the
//!   opportunistic one (RFC 7435).
//! - **none** is one of those names: no encryption, for development work. A person
//!   has to ask for it; it is not in the automatic order, and nothing a server
//!   sends can select it. The server has to have been set to allow it too, so both
//!   ends are a decision. Do not use it to reach a zone you do not run: the login
//!   packet carries your password.
//!
//! **VIE is the floor of the automatic path.** Each step down is triggered by an
//! attempt that failed, and failure is cheap to manufacture for anyone positioned to
//! disturb the traffic; a ladder whose bottom rung is cleartext hands that attacker
//! cleartext for the price of being a nuisance. VIE costs nothing to keep as the
//! bottom rung, because it is the path every legacy client uses anyway. The
//! registry enforces it: nothing weaker may be registered as automatic.
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
use std::fmt;
use std::path::{Path, PathBuf};

use oss_net::Handshake;
use oss_net::opensec::{IdentityKey, os_entropy};
pub use oss_net::{
    ClientContext, Era, LadderProgress, Mechanism, MechanismError, MechanismId, MechanismPolicy,
    MechanismRegistry, Selection,
};

/// How long a mechanism in the automatic order may go unanswered before the
/// next one is tried.
pub const FALLBACK_AFTER_MS: u64 = 1500;

/// The word for the automatic order. It is not a mechanism, so it is read
/// before any mechanism's name.
pub const AUTO: &str = "auto";

/// What a person asked for: the automatic order, or one mechanism by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SecurityMode {
    /// The automatic order, read from the registry by strength.
    #[default]
    Auto,
    /// One mechanism, and nothing else.
    Named(MechanismId),
}

impl SecurityMode {
    /// Reads a word a person typed: [`AUTO`] first, then the name of a
    /// mechanism in `registry` (a short name in any case, `name@domain`
    /// exactly). The error says which words this build accepts.
    pub fn parse(word: &str, registry: &MechanismRegistry) -> Result<Self, String> {
        let word = word.trim();
        if word.eq_ignore_ascii_case(AUTO) {
            return Ok(Self::Auto);
        }
        registry
            .find(word)
            .map(|m| Self::Named(m.id().clone()))
            .ok_or_else(|| format!("expected {}, got `{word}`", words(registry)))
    }

    /// The word for this choice, as [`SecurityMode::parse`] reads it.
    pub fn name(&self) -> &str {
        match self {
            Self::Auto => AUTO,
            Self::Named(id) => id.as_str(),
        }
    }
}

impl fmt::Display for SecurityMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Every word [`SecurityMode::parse`] accepts: [`AUTO`], then each mechanism's
/// name, weakest first, for example `auto, none, vie or opensec`.
pub fn words(registry: &MechanismRegistry) -> String {
    let mut all = vec![AUTO];
    all.extend(registry.ids().map(MechanismId::as_str));
    match all.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => all.join(""),
    }
}

/// The two lines a program logs at start, read from `registry`: what it can
/// speak, weakest first, and the order **auto** tries, for example
/// `Mechanisms supported: None, Subspace, OpenSecurity` and
/// `Automatic order: OpenSecurity, then Subspace`.
pub fn startup_lines(registry: &MechanismRegistry) -> [String; 2] {
    [
        registry.supported_line(),
        MechanismPolicy::automatic(registry).automatic_line(registry),
    ]
}

/// The help for a `--security <mode>` option, read from `registry` and wrapped
/// to `width` columns, every line after the first indented to `column` (the
/// first goes after the option's name). `notes` is said after the words, for
/// what only one program needs to say.
pub fn usage(registry: &MechanismRegistry, notes: &str, column: usize, width: usize) -> String {
    let order: Vec<&str> = MechanismPolicy::automatic(registry)
        .order()
        .iter()
        .filter_map(|id| registry.get(id))
        .map(Mechanism::display_name)
        .collect();
    let names: Vec<&str> = registry.ids().map(MechanismId::as_str).collect();
    let mut text = format!(
        "auto (the default) tries the automatic order, {}, stepping down when a \
         server does not answer. Or name one mechanism: {}.",
        order.join(", then "),
        match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            _ => names.join(""),
        }
    );
    if !notes.is_empty() {
        text.push(' ');
        text += notes;
    }
    if registry.find("none").is_some() {
        text += " Choosing none sends your password in the clear; it works only against \
                 a server set to allow it, and is never chosen for you.";
    }
    wrap(&text, column, width)
}

/// Greedy word wrap: the first line has no indent, the rest `column` spaces.
fn wrap(text: &str, column: usize, width: usize) -> String {
    let room = width.saturating_sub(column).max(1);
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > room {
            out += &line;
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', column));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line += word;
    }
    out + &line
}

/// A connection's context: its seed, the server's key if one is on record in
/// `known_servers.txt` (an OpenSec key: see [`KnownServers::mechanism`]), and
/// the identity to offer the zone.
pub fn context(
    seed: u32,
    known_key: Option<[u8; 32]>,
    identity: Option<IdentityKey>,
) -> ClientContext {
    let mut ctx = ClientContext::new(seed);
    if let Some(key) = known_key {
        ctx = ctx.pinned(KnownServers::mechanism(), key);
    }
    if let Some(identity) = identity {
        ctx = ctx.identity(identity);
    }
    ctx
}

/// A connection's handshake, and how to learn which mechanism it ended on.
pub struct Negotiation {
    /// The handshake to open the connection with.
    pub handshake: Box<dyn Handshake>,
    /// Which mechanism it uses, once it has settled.
    pub mechanism: MechanismInUse,
}

impl fmt::Debug for Negotiation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Negotiation")
            .field("mechanism", &self.mechanism)
            .finish_non_exhaustive()
    }
}

/// Which mechanism a connection uses.
#[derive(Debug, Clone)]
pub enum MechanismInUse {
    /// The automatic order: the ladder says which rung won, once one has.
    Automatic(LadderProgress),
    /// One mechanism asked for by name: the only one the connection can use.
    Named {
        /// The mechanism.
        id: MechanismId,
        /// What the zone should treat the connection as.
        era: Era,
    },
}

impl MechanismInUse {
    /// The mechanism the connection uses: for the automatic order, the one that
    /// won, and `None` until one has; for a mechanism by name, that one.
    pub fn id(&self) -> Option<&MechanismId> {
        match self {
            Self::Automatic(progress) => progress.established(),
            Self::Named { id, .. } => Some(id),
        }
    }

    /// The era of [`MechanismInUse::id`]: what the zone will treat the
    /// connection as, declared by the mechanism itself.
    pub fn era(&self) -> Option<Era> {
        match self {
            Self::Automatic(progress) => progress.established_era(),
            Self::Named { era, .. } => Some(*era),
        }
    }
}

/// The client handshake for `mode`, from the mechanisms in `registry`.
///
/// **auto** is a ladder down the automatic order, stepping down after
/// [`FALLBACK_AFTER_MS`] without an answer, or the one rung of the mechanism
/// whose key `ctx` carries. A mechanism **by name** is that mechanism's own
/// handshake, with no ladder around it, whatever key `ctx` carries. Refused
/// for a name the registry does not hold, a key recorded by a mechanism it
/// does not hold, or an automatic order with nothing in it.
pub fn client_handshake(
    registry: &MechanismRegistry,
    mode: &SecurityMode,
    ctx: &ClientContext,
) -> Result<Negotiation, MechanismError> {
    match mode {
        SecurityMode::Auto => {
            let ladder = MechanismPolicy::automatic(registry).client_handshake(
                registry,
                ctx,
                FALLBACK_AFTER_MS,
            )?;
            let progress = ladder.progress();
            Ok(Negotiation {
                handshake: Box::new(ladder),
                mechanism: MechanismInUse::Automatic(progress),
            })
        }
        SecurityMode::Named(id) => {
            let era = registry
                .get(id)
                .ok_or_else(|| MechanismError::Unknown(id.as_str().to_owned()))?
                .era();
            Ok(Negotiation {
                handshake: registry.client_handshake(id.as_str(), ctx)?,
                mechanism: MechanismInUse::Named {
                    id: id.clone(),
                    era,
                },
            })
        }
    }
}

/// A file of 32-byte keys by "host:port", one `host:port hexkey` per line.
#[derive(Debug, Clone, Default)]
struct KeyFile {
    path: Option<PathBuf>,
    keys: BTreeMap<String, [u8; 32]>,
    /// Lines that are neither comments nor keys, kept as they are and written
    /// back: a line this version cannot read is not this version's to delete.
    other_lines: Vec<String>,
    /// The file exists but could not be read: why. Its keys are unknown, so it
    /// is never written over.
    unreadable: Option<String>,
}

impl KeyFile {
    /// A missing file is an empty list. A file that exists but cannot be read
    /// is an empty list too, marked `unreadable`: it used to be taken for an
    /// empty one and written over at the next save, losing every key in it.
    /// Lines that are not keys are kept for writing back.
    fn load(path: &Path) -> Self {
        let mut keys = BTreeMap::new();
        let mut other_lines = Vec::new();
        let mut unreadable = None;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let lines = text.lines().map(str::trim);
                for line in lines.filter(|l| !l.is_empty() && !l.starts_with('#')) {
                    if let Some((server, key)) = line.split_once(char::is_whitespace)
                        && let Some(k) = unhex(key.trim())
                    {
                        keys.insert(server.to_ascii_lowercase(), k);
                    } else {
                        log::warn!(
                            "{}: a line that is not a key is kept as it is",
                            path.display()
                        );
                        other_lines.push(line.to_string());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                log::error!("{} could not be read: {e}", path.display());
                unreadable = Some(e.to_string());
            }
        }
        Self {
            path: Some(path.to_path_buf()),
            keys,
            other_lines,
            unreadable,
        }
    }

    fn get(&self, server: &str) -> Option<[u8; 32]> {
        self.keys.get(&server.to_ascii_lowercase()).copied()
    }

    /// Adds or replaces a key and saves the file under `header`.
    fn insert(&mut self, server: &str, key: [u8; 32], header: &str) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            self.keys.insert(server.to_ascii_lowercase(), key);
            return Ok(());
        };
        if let Some(why) = &self.unreadable {
            return Err(std::io::Error::other(format!(
                "{} could not be read ({why}), so it is not written over",
                path.display()
            )));
        }
        self.keys.insert(server.to_ascii_lowercase(), key);
        let mut text = String::from(header);
        for line in &self.other_lines {
            text += line;
            text.push('\n');
        }
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
    /// The mechanism whose keys this file holds: OpenSec, the only one that
    /// records a server key. A key from here is passed on as recorded by it
    /// ([`context`]), so **auto** uses OpenSec alone for that server.
    pub fn mechanism() -> MechanismId {
        MechanismId::new("opensec").expect("a reserved name")
    }

    /// Reads `path` (lines of `host:port hexkey`); a missing file is an empty list.
    pub fn load(path: &Path) -> Self {
        Self(KeyFile::load(path))
    }

    pub fn get(&self, server: &str) -> Option<[u8; 32]> {
        self.0.get(server)
    }

    /// Why the file could not be read, when it exists and could not. Its keys
    /// are then unknown, so a server whose key changed cannot be told from one
    /// met for the first time: a caller should not connect on trust-on-first-use
    /// until the file is fixed. [`KnownServers::remember`] refuses to write
    /// over it.
    pub fn unreadable(&self) -> Option<&str> {
        self.0.unreadable.as_deref()
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::test_zone::dummy_mechanism;
    use oss_net::opensec::{OpenSecAcceptor, ServerKey};
    use oss_net::{
        CipherKind, Config, Connection, Event, Ladder, NullCipher, Server, Step, VieAcceptor,
    };

    /// A fresh path in the system's temporary folder for one test.
    fn temp_file(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("oss-session-keyfile-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("keys.txt")
    }

    /// The handshake for a typed mode, with the built-in mechanisms: the shape
    /// the tests below were written against (a mode, the key on record, the
    /// identity and the seed), so that each still reads as it did.
    fn handshake(
        mode: &str,
        pinned: Option<[u8; 32]>,
        identity: Option<IdentityKey>,
        seed: u32,
    ) -> Box<dyn Handshake> {
        let registry = MechanismRegistry::builtin();
        let mode = SecurityMode::parse(mode, &registry).expect("a built-in mode");
        client_handshake(&registry, &mode, &context(seed, pinned, identity))
            .expect("the built-ins build")
            .handshake
    }

    /// A file that exists but cannot be read is not taken for an empty one:
    /// it says so, and nothing is written over it.
    #[test]
    fn an_unreadable_key_file_is_never_written_over() {
        let path = temp_file("unreadable");
        let bytes = [b'a', 0xFF, 0xFE, b'\n'];
        std::fs::write(&path, bytes).unwrap();
        let mut known = KnownServers::load(&path);
        assert!(known.unreadable().is_some());
        assert!(known.remember("192.0.2.1:5000", [7; 32]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "left as it was");

        let mut ids = Identities::load(&path);
        assert!(ids.for_server("192.0.2.1:5000").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "left as it was");

        let missing = KnownServers::load(&path.with_file_name("absent.txt"));
        assert!(
            missing.unreadable().is_none(),
            "a missing file is just empty"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A line that is not a key survives the next save.
    #[test]
    fn lines_that_are_not_keys_are_kept() {
        let path = temp_file("kept");
        let key = "ab".repeat(32);
        std::fs::write(
            &path,
            format!("# header\n192.0.2.1:5000 {key}\nsomething newer 1 2 3\n"),
        )
        .unwrap();
        let mut known = KnownServers::load(&path);
        assert!(known.unreadable().is_none());
        assert_eq!(known.get("192.0.2.1:5000"), Some([0xAB; 32]));
        known.remember("192.0.2.2:5000", [1; 32]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("something newer 1 2 3"), "{text}");
        assert!(text.contains(&key), "{text}");
        let again = KnownServers::load(&path);
        assert_eq!(again.get("192.0.2.2:5000"), Some([1; 32]));
        assert_eq!(again.get("192.0.2.1:5000"), Some([0xAB; 32]));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Connects a client with `handshake` to a server with `acceptors`; returns
    /// the client's cipher name and server key once connected, or None.
    fn connect(
        handshake: Box<dyn Handshake>,
        acceptors: Vec<Box<dyn oss_net::Acceptor>>,
    ) -> Option<(&'static str, Option<[u8; 32]>)> {
        connect_kind(handshake, acceptors).map(|(name, key, _)| (name, key))
    }

    /// What a client ended with: its cipher's name, the server's key, and the
    /// kind of cipher.
    type Ended = (&'static str, Option<[u8; 32]>, Option<CipherKind>);

    /// [`connect`], with the kind of cipher the client ended with.
    fn connect_kind(
        handshake: Box<dyn Handshake>,
        acceptors: Vec<Box<dyn oss_net::Acceptor>>,
    ) -> Option<Ended> {
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
                    Event::Connected => {
                        return Some((
                            client.cipher_name()?,
                            client.peer_key(),
                            client.cipher_kind(),
                        ));
                    }
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
            handshake("auto", None, None, 7),
            vec![opensec_server(&key), Box::new(VieAcceptor::default())],
        )
        .expect("connects");
        assert_eq!(cipher, "opensec");
        assert_eq!(peer, Some(key.public()));
    }

    #[test]
    fn auto_falls_back_to_vie_when_opensec_is_ignored() {
        let (cipher, _) = connect(
            handshake("auto", None, None, 7),
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
            handshake("auto", Some(key.public()), None, 7),
            vec![opensec_server(&key), Box::new(VieAcceptor::default())],
        );
        assert_eq!(ok.map(|c| c.0), Some("opensec"));
        // Wrong key: refused, and no VIE fallback even though the server offers it.
        let bad = connect(
            handshake("auto", Some(other.public()), None, 7),
            vec![opensec_server(&key), Box::new(VieAcceptor::default())],
        );
        assert_eq!(bad, None);
        // A VIE-only server with a key on record: no downgrade either.
        let vie_only = connect(
            handshake("auto", Some(key.public()), None, 7),
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
        for mode in ["auto", "opensec"] {
            let hs = handshake(mode, None, Some(me.clone()), 7);
            assert_eq!(identity_seen_by(hs, accepting()), Some(me.public()));
        }
        // Without one the zone sees none; a zone that doesn't take identities
        // still connects.
        let hs = handshake("opensec", None, None, 7);
        assert_eq!(identity_seen_by(hs, accepting()), None);
        let plain = connect(
            handshake("auto", None, Some(me.clone()), 7),
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

    /// The mechanism a build outside this repository supplies is registered,
    /// not swapped in for the built-ins: one `register` line, and **auto** tries
    /// it at its place by strength and connects over it, `--security` can name
    /// it, and every built-in is still there and still does what it did.
    #[test]
    fn a_registered_mechanism_is_used_and_the_built_ins_are_still_there() {
        let mut registry = MechanismRegistry::builtin();
        registry
            .register(dummy_mechanism(
                "dummy@example.org",
                "Dummy",
                Era::Modern,
                Mechanism::STRENGTH_OPENSEC + 1000,
            ))
            .unwrap();
        let ctx = || context(7, None, None);
        let dummy_zone = || -> Vec<Box<dyn oss_net::Acceptor>> {
            let only = registry.find("dummy@example.org").unwrap();
            vec![only.acceptor().unwrap(), Box::new(VieAcceptor::default())]
        };

        // Automatic: the strongest, so tried first, and it is what connects.
        let auto = client_handshake(&registry, &SecurityMode::Auto, &ctx()).unwrap();
        let used = auto.mechanism.clone();
        assert_eq!(
            connect(auto.handshake, dummy_zone()).map(|c| c.0),
            Some("dummy")
        );
        assert_eq!(
            used.id(),
            Some(&MechanismId::new("dummy@example.org").unwrap())
        );
        assert_eq!(used.era(), Some(Era::Modern));

        // By name, with nothing else tried.
        let named = SecurityMode::parse("dummy@example.org", &registry).unwrap();
        let by_name = client_handshake(&registry, &named, &ctx()).unwrap();
        assert_eq!(
            connect(by_name.handshake, dummy_zone()).map(|c| c.0),
            Some("dummy")
        );

        // The built-ins are all still there, by name and in the automatic
        // order behind it, and each still connects as itself.
        assert_eq!(
            startup_lines(&registry),
            [
                "Mechanisms supported: None, Subspace, OpenSecurity, Dummy",
                "Automatic order: Dummy, then OpenSecurity, then Subspace",
            ]
        );
        let key = ServerKey::generate(&os_entropy());
        for (word, cipher) in [("opensec", "opensec"), ("vie", "vie")] {
            let mode = SecurityMode::parse(word, &registry).unwrap();
            let hs = client_handshake(&registry, &mode, &ctx())
                .unwrap()
                .handshake;
            let zone = vec![opensec_server(&key), Box::new(VieAcceptor::default()) as _];
            assert_eq!(connect(hs, zone).map(|c| c.0), Some(cipher), "{word}");
        }
        let none = SecurityMode::parse("none", &registry).unwrap();
        let hs = client_handshake(&registry, &none, &ctx())
            .unwrap()
            .handshake;
        assert_eq!(
            connect(hs, vec![Box::new(oss_net::NullAcceptor)]).map(|c| c.0),
            Some("none")
        );

        // A zone without the dummy: automatic steps down past it to OpenSec.
        let auto = client_handshake(&registry, &SecurityMode::Auto, &ctx()).unwrap();
        let used = auto.mechanism.clone();
        let zone = vec![opensec_server(&key), Box::new(VieAcceptor::default()) as _];
        assert_eq!(connect(auto.handshake, zone).map(|c| c.0), Some("opensec"));
        assert_eq!(used.id(), Some(&MechanismId::new("opensec").unwrap()));
    }

    /// The registry with the built-ins is exactly this repository's modes as
    /// they were: auto prefers OpenSec, each name is that mechanism, and a key on
    /// record is never downgraded.
    #[test]
    fn the_built_in_registry_is_this_repositorys_modes() {
        let key = ServerKey::generate(&os_entropy());
        let both = || vec![opensec_server(&key), Box::new(VieAcceptor::default()) as _];
        assert_eq!(
            connect(handshake("auto", None, None, 7), both()).map(|c| c.0),
            Some("opensec"),
        );
        assert_eq!(
            connect(handshake("vie", None, None, 7), both()).map(|c| c.0),
            Some("vie"),
        );
        assert_eq!(
            connect(handshake("opensec", None, None, 7), both()).map(|c| c.0),
            Some("opensec"),
        );
        // A key on record is still never downgraded.
        let other = ServerKey::generate(&os_entropy());
        assert_eq!(
            connect(handshake("auto", Some(other.public()), None, 7), both()),
            None
        );
        assert_eq!(
            startup_lines(&MechanismRegistry::builtin()),
            [
                "Mechanisms supported: None, Subspace, OpenSecurity",
                "Automatic order: OpenSecurity, then Subspace",
            ]
        );
    }

    /// `none` is reached by asking for it and no other way. A server that will
    /// not encrypt gets a connection out of the mode that asked and out of
    /// nothing else, including `auto`, which is the mode that walks down a
    /// ladder: VIE is the bottom rung of that ladder and `none` is not on it.
    #[test]
    fn no_encryption_is_only_ever_asked_for() {
        let refusing =
            || -> Vec<Box<dyn oss_net::Acceptor>> { vec![Box::new(oss_net::NullAcceptor)] };

        let (cipher, peer) = connect(handshake("none", None, None, 7), refusing())
            .expect("the mode that asked connects");
        assert_eq!(cipher, "none");
        assert_eq!(peer, None);

        for mode in ["auto", "vie", "opensec"] {
            assert_eq!(
                connect(handshake(mode, None, None, 7), refusing()),
                None,
                "{mode} must not end unencrypted"
            );
        }

        // And asking for it is a person's decision, so it wins over a key on
        // record for the server -- an explicit policy over an opportunistic one.
        let key = ServerKey::generate(&os_entropy());
        let (cipher, _) = connect(handshake("none", Some(key.public()), None, 7), refusing())
            .expect("an explicit mode is not overridden by a remembered key");
        assert_eq!(cipher, "none");
    }

    /// Every word the registry gives round-trips through its name, `auto`
    /// first; the short names match in any case; an unknown word is refused
    /// with the list of words this build accepts; and of the built-ins only
    /// `none` does not encrypt, and it may only be asked for by name.
    #[test]
    fn every_mode_round_trips_through_its_name() {
        let registry = MechanismRegistry::builtin();
        assert_eq!(words(&registry), "auto, none, vie or opensec");
        for word in ["auto", "none", "vie", "opensec"] {
            let mode = SecurityMode::parse(word, &registry).unwrap();
            assert_eq!(mode.name(), word);
            assert_eq!(SecurityMode::parse(mode.name(), &registry), Ok(mode));
        }
        assert_eq!(
            SecurityMode::parse("AUTO", &registry),
            Ok(SecurityMode::Auto)
        );
        assert_eq!(
            SecurityMode::parse("NONE", &registry).unwrap().name(),
            "none"
        );
        assert_eq!(
            SecurityMode::parse("plaintext", &registry),
            Err("expected auto, none, vie or opensec, got `plaintext`".to_owned())
        );
        let by_name_only: Vec<&str> = registry
            .iter()
            .filter(|m| !m.is_automatic())
            .map(|m| m.id().as_str())
            .collect();
        assert_eq!(by_name_only, ["none"]);
    }

    /// A mechanism registered in one line becomes a word `--security` accepts,
    /// and the help text lists it, in its place by strength.
    #[test]
    fn a_registered_mechanism_is_a_word_and_is_in_the_help() {
        let mut registry = MechanismRegistry::builtin();
        registry
            .register(dummy_mechanism(
                "dummy@example.org",
                "Dummy",
                Era::Modern,
                2000,
            ))
            .unwrap();
        assert_eq!(
            words(&registry),
            "auto, none, vie, dummy@example.org or opensec"
        );
        assert_eq!(
            SecurityMode::parse("dummy@example.org", &registry)
                .unwrap()
                .name(),
            "dummy@example.org"
        );
        let help = usage(&registry, "", 4, 60);
        let flat = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains("the automatic order, OpenSecurity, then Dummy, then Subspace,"),
            "{help}"
        );
        assert!(
            flat.contains("Or name one mechanism: none, vie, dummy@example.org or opensec."),
            "{help}"
        );
        for line in help.lines() {
            assert!(line.len() <= 60, "{line:?} is too long");
        }
        for line in help.lines().skip(1) {
            assert!(
                line.starts_with("    ") && !line.starts_with("     "),
                "{line:?}"
            );
        }
    }

    /// The login form is checked against the era the winning mechanism
    /// declares, on the automatic path and the by-name path alike. A cipher from
    /// outside oss-core reports no kind of its own, so before mechanisms declared
    /// an era the check could not tell and switched itself off; now it reads
    /// the era, and catches the mismatch.
    #[test]
    fn the_login_form_check_reads_the_era_on_both_paths() {
        for (era, wants_continuum_form) in [
            (Era::Continuum, true),
            (Era::SubSpace, false),
            (Era::Modern, false),
        ] {
            let mut registry = MechanismRegistry::builtin();
            registry
                .register(dummy_mechanism("dummy@example.org", "Dummy", era, 2000))
                .unwrap();
            let zone = || {
                vec![
                    registry
                        .find("dummy@example.org")
                        .unwrap()
                        .acceptor()
                        .unwrap(),
                ]
            };
            let named = SecurityMode::parse("dummy@example.org", &registry).unwrap();
            for mode in [SecurityMode::Auto, named] {
                let negotiation =
                    client_handshake(&registry, &mode, &context(7, None, None)).expect("builds");
                let used = negotiation.mechanism.clone();
                let (cipher, _, kind) = connect_kind(negotiation.handshake, zone())
                    .unwrap_or_else(|| panic!("{mode} connects"));
                assert_eq!(cipher, "dummy");
                assert_eq!(kind, Some(CipherKind::Other), "the cipher says nothing");
                assert_eq!(used.era(), Some(era), "{mode}");

                let era_seen = crate::login_era(Some(&used), kind);
                assert_eq!(era_seen, Some(era), "{mode}");
                assert_eq!(
                    crate::login_form_mismatch(era_seen, wants_continuum_form),
                    None,
                    "{mode}: the right form for {era:?}"
                );
                assert!(
                    crate::login_form_mismatch(era_seen, !wants_continuum_form).is_some(),
                    "{mode}: the wrong form for {era:?} is caught"
                );
                // Without the mechanism, the cipher alone cannot tell.
                assert_eq!(crate::login_era(None, kind), None);
            }
        }
    }

    /// What a test handshake did, step by step.
    type Trace = Vec<(u64, String, Vec<Vec<u8>>, bool, bool)>;

    /// A handshake that plays a script: it sends one datagram per poll, says
    /// it was answered from `answered_at`, and at `ends_at` returns `end`.
    struct Scripted {
        answered_at: Option<u64>,
        ends_at: u64,
        end: fn() -> Step,
        abandoned: Arc<Mutex<Vec<u64>>>,
        polls: Arc<AtomicUsize>,
        now: u64,
    }

    impl Handshake for Scripted {
        fn on_datagram(&mut self, _now: u64, _data: &[u8], _out: &mut Vec<Vec<u8>>) -> Step {
            Step::Pending
        }
        fn poll(&mut self, now: u64, out: &mut Vec<Vec<u8>>) -> Step {
            self.now = now;
            let n = self.polls.fetch_add(1, Ordering::SeqCst);
            out.push(vec![0, 1, n as u8]);
            if now >= self.ends_at {
                (self.end)()
            } else {
                Step::Pending
            }
        }
        fn answered(&self) -> bool {
            self.answered_at.is_some_and(|t| self.now >= t)
        }
        fn abandon(&mut self, now: u64, _out: &mut Vec<Vec<u8>>) {
            self.abandoned.lock().unwrap().push(now);
        }
    }

    /// Runs `hs` for 5 s, every 10 ms, and records what it did.
    fn trace(mut hs: Box<dyn Handshake>) -> Trace {
        let mut out = Vec::new();
        for now in (0..5_000).step_by(10) {
            let mut sent = Vec::new();
            let step = match hs.poll(now, &mut sent) {
                Step::Pending => "pending".to_owned(),
                Step::Established(c) => format!("established {}", c.name()),
                Step::Failed(why) => format!("failed {why}"),
                Step::StepDown(why) => format!("step down {why}"),
            };
            let done = step != "pending";
            out.push((now, step, sent, hs.answered(), hs.take_fresh_endpoint()));
            if done {
                break;
            }
        }
        out
    }

    /// **auto** with a key on record is a ladder of one rung where it used to
    /// be that mechanism's bare handshake. A one-rung ladder must be the bare
    /// handshake exactly: same steps, same datagrams, same answers to
    /// `answered` and `take_fresh_endpoint`, nothing abandoned, under every
    /// ending -- silence, an answer, a success, a failure and a step down.
    #[test]
    fn a_ladder_of_one_rung_is_the_bare_handshake() {
        // When it says it was answered, when it ends, and how.
        type Script = (Option<u64>, u64, fn() -> Step);
        let scripts: [Script; 5] = [
            (None, u64::MAX, || Step::Pending),
            (Some(300), 2_000, || Step::Established(Box::new(NullCipher))),
            (None, 1_600, || Step::Established(Box::new(NullCipher))),
            (None, 400, || Step::Failed("refused")),
            (None, 500, || Step::StepDown("another mechanism")),
        ];
        for (answered_at, ends_at, end) in scripts {
            let make = |abandoned: &Arc<Mutex<Vec<u64>>>| Scripted {
                answered_at,
                ends_at,
                end,
                abandoned: Arc::clone(abandoned),
                polls: Arc::new(AtomicUsize::new(0)),
                now: 0,
            };
            let bare_abandoned = Arc::new(Mutex::new(Vec::new()));
            let rung_abandoned = Arc::new(Mutex::new(Vec::new()));
            let bare = trace(Box::new(make(&bare_abandoned)));
            let ladder = Ladder::new(
                FALLBACK_AFTER_MS,
                vec![(
                    MechanismId::new("opensec").unwrap(),
                    Era::Modern,
                    Box::new(make(&rung_abandoned)) as Box<dyn Handshake>,
                )],
            )
            .unwrap();
            let one_rung = trace(Box::new(ladder));
            assert_eq!(one_rung, bare, "answered {answered_at:?}, ends {ends_at}");
            assert!(bare_abandoned.lock().unwrap().is_empty());
            assert!(rung_abandoned.lock().unwrap().is_empty());
        }
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
            SecurityMode::parse("OpenSec", &MechanismRegistry::builtin()),
            Ok(SecurityMode::Named(KnownServers::mechanism()))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
