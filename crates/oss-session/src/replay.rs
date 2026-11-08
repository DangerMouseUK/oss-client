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
        self.write(RecordKind::ClientPacket, now_ms, data);
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
