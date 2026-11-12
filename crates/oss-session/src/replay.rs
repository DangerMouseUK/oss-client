//! Recording a session to an `.ossr` replay, and playing one back.
//!
//! The recorder appends records as the game runs (uncompressed, so a recording cut
//! short by a crash still plays up to its last complete record): metadata, every game
//! packet received and sent, and the map and LVZ files. It also notes the server clock
//! when recording starts (`server_ticks` metadata), so playback can rebuild the
//! server's time line.
//!
//! Playback is a [`crate::Session`] made with [`crate::Session::playback`]: it never
//! touches the network, and [`Player`] feeds it the records due at each moment of the
//! recording.

use std::io::Write;
use std::path::Path;

use oss_formats::replay::{FLAG_COMPRESSED, Record, RecordKind, Replay, header, write_record};

/// Writes a replay file as the session runs.
pub struct Recorder {
    out: Box<dyn Write + Send>,
    start_ms: u64,
    failed: bool,
    buf: Vec<u8>,
}

/// What replaces a recorded password. A marker rather than zeros, so a reader of
/// the file can tell the field was redacted instead of wondering whether the
/// player left it blank. Exactly [`PASSWORD_LEN`] bytes, so nothing shifts.
const PASSWORD_MARKER: &[u8; 32] = b"[password redacted on recording]";

/// Offset and length of the password in a login packet. `0x09` and `0x24` share
/// the prefix: kind, flags, then a 32-byte name, then the password.
const PASSWORD_AT: usize = 34;
const PASSWORD_LEN: usize = 32;

/// A copy of `data` with the password blanked, when `data` is a login.
///
/// A replay is the natural thing to attach to a bug report or hand to someone
/// else, and the login packet carries the password as the client sent it -- in
/// clear, for a VIE login or a Continuum one without a biller identity. Recording
/// it verbatim turns a debugging aid into a credential leak, and nothing about
/// the file would warn anyone. Nothing replays a login, so removing it costs the
/// recording nothing.
fn redacted_login(data: &[u8]) -> Option<Vec<u8>> {
    let kind = *data.first()?;
    if kind != oss_proto::c2s::kind::LOGIN && kind != oss_proto::c2s::kind::CONT_LOGIN {
        return None;
    }
    if data.len() < PASSWORD_AT + PASSWORD_LEN {
        return None;
    }
    let mut safe = data.to_vec();
    safe[PASSWORD_AT..PASSWORD_AT + PASSWORD_LEN].copy_from_slice(PASSWORD_MARKER);
    Some(safe)
}

impl Recorder {
    /// Starts a recording into `out` at session time `now_ms`, with `meta` key/value
    /// pairs (zone, arena, player, client, started...).
    pub fn new(
        mut out: Box<dyn Write + Send>,
        now_ms: u64,
        meta: &[(&str, &str)],
    ) -> std::io::Result<Self> {
        out.write_all(&header(0))?;
        let mut r = Self {
            out,
            start_ms: now_ms,
            failed: false,
            buf: Vec::new(),
        };
        for (k, v) in meta {
            r.meta(now_ms, k, v);
        }
        Ok(r)
    }

    /// A recording into a new file at `path`.
    pub fn create(path: &Path, now_ms: u64, meta: &[(&str, &str)]) -> std::io::Result<Self> {
        let file = std::fs::File::create(path)?;
        Self::new(Box::new(std::io::BufWriter::new(file)), now_ms, meta)
    }

    fn time(&self, now_ms: u64) -> u32 {
        now_ms.saturating_sub(self.start_ms).min(u32::MAX as u64) as u32
    }

    fn write(&mut self, kind: RecordKind, now_ms: u64, data: &[u8]) {
        if self.failed {
            return;
        }
        let time = self.time(now_ms);
        self.buf.clear();
        write_record(&mut self.buf, kind, time, data);
        if let Err(e) = self.out.write_all(&self.buf) {
            log::warn!("replay recording stopped: {e}");
            self.failed = true;
        }
    }

    pub fn meta(&mut self, now_ms: u64, key: &str, value: &str) {
        let r = Record::meta(self.time(now_ms), key, value);
        self.write(RecordKind::Meta, now_ms, &r.data);
    }

    pub fn server_packet(&mut self, now_ms: u64, data: &[u8]) {
        self.write(RecordKind::ServerPacket, now_ms, data);
    }

    pub fn client_packet(&mut self, now_ms: u64, data: &[u8]) {
        match redacted_login(data) {
            Some(safe) => self.write(RecordKind::ClientPacket, now_ms, &safe),
            None => self.write(RecordKind::ClientPacket, now_ms, data),
        }
    }

    pub fn file(&mut self, now_ms: u64, name: &str, contents: &[u8]) {
        let r = Record::file(self.time(now_ms), name.as_bytes(), contents);
        self.write(RecordKind::File, now_ms, &r.data);
    }

    pub fn marker(&mut self, now_ms: u64, text: &str) {
        self.write(RecordKind::Marker, now_ms, text.as_bytes());
    }

    /// Pushes buffered records to the file.
    pub fn flush(&mut self) {
        if !self.failed && self.out.flush().is_err() {
            self.failed = true;
        }
    }

    /// Whether writing has failed (the recording stopped).
    pub fn failed(&self) -> bool {
        self.failed
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.out.flush();
    }
}

/// Reads a replay and hands out its records by time.
pub struct Player {
    pub replay: Replay,
    next: usize,
}

impl Player {
    pub fn open(path: &Path) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_bytes(&data).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let replay = Replay::parse(data).map_err(|e| e.to_string())?;
        if replay.truncated {
            log::warn!("the replay was cut short; playing what's there");
        }
        Ok(Self { replay, next: 0 })
    }

    /// Records due by `time_ms` (milliseconds into the recording) not yet handed out.
    pub fn due(&mut self, time_ms: u32) -> &[Record] {
        let start = self.next;
        while self
            .replay
            .records
            .get(self.next)
            .is_some_and(|r| r.time_ms <= time_ms)
        {
            self.next += 1;
        }
        &self.replay.records[start..self.next]
    }

    /// Every record has been handed out.
    pub fn finished(&self) -> bool {
        self.next >= self.replay.records.len()
    }

    /// A file the replay carries (map or LVZ), by name.
    pub fn file(&self, name: &str) -> Option<&[u8]> {
        self.replay
            .files()
            .find(|(n, _)| String::from_utf8_lossy(n).eq_ignore_ascii_case(name))
            .map(|(_, data)| data)
    }

    pub fn duration_ms(&self) -> u32 {
        self.replay.duration_ms()
    }
}

/// Whether a replay file is compressed (for information only; both kinds play).
pub fn is_compressed(data: &[u8]) -> bool {
    data.len() >= 6 && u16::from_le_bytes([data[4], data[5]]) & FLAG_COMPRESSED != 0
}

#[cfg(test)]
mod redaction_tests {
    use super::{PASSWORD_AT, PASSWORD_LEN, PASSWORD_MARKER, redacted_login};

    fn login(kind: u8, len: usize) -> Vec<u8> {
        let mut p = vec![0u8; len];
        p[0] = kind;
        p[PASSWORD_AT..PASSWORD_AT + PASSWORD_LEN]
            .copy_from_slice(b"hunter2hunter2hunter2hunter2hunt");
        p
    }

    #[test]
    fn a_recorded_login_does_not_carry_the_password() {
        // Both forms: the 101-byte VIE 0x09 and the 165-byte Continuum 0x24.
        for (kind, len) in [(0x09u8, 101usize), (0x24, 165)] {
            let raw = login(kind, len);
            let safe = redacted_login(&raw).expect("a login should be redacted");
            assert_eq!(safe.len(), raw.len(), "length must not shift");
            assert_eq!(
                &safe[PASSWORD_AT..PASSWORD_AT + PASSWORD_LEN],
                PASSWORD_MARKER
            );
            assert!(
                !safe.windows(7).any(|w| w == b"hunter2"),
                "the password survived in a {kind:#04x} login"
            );
            // Everything outside the field is untouched, so the replay is unchanged.
            assert_eq!(safe[..PASSWORD_AT], raw[..PASSWORD_AT]);
            assert_eq!(
                safe[PASSWORD_AT + PASSWORD_LEN..],
                raw[PASSWORD_AT + PASSWORD_LEN..]
            );
        }
    }

    #[test]
    fn other_packets_are_recorded_untouched() {
        // Not a login: left alone, so nothing else in the recording changes.
        let mut position = vec![0u8; 101];
        position[0] = 0x03;
        assert!(redacted_login(&position).is_none());
        // A login too short to hold the field is left alone rather than panicking;
        // the data comes off the network and may be anything.
        assert!(redacted_login(&[0x24, 0x00, 0x01]).is_none());
        assert!(redacted_login(&[]).is_none());
    }
}
