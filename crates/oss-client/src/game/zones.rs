//! The zone browser: zones from the configured directory servers, with live
//! population and ping. Opened with F3 on the login screen.
//!
//! Only directory servers listed in the settings (`[directory] servers`) or given
//! with `--directory` are contacted; none are configured by default.

use std::time::{Duration, Instant};

use oss_session::directory::{DirectoryClient, ZoneInfo, ZonePinger};
use winit::keyboard::KeyCode;

/// How often zones are pinged.
const PING_EVERY: Duration = Duration::from_secs(2);
/// Give up on a directory after this long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// What the player picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserChoice {
    /// Join this zone ("host:port").
    Pick(String),
    Back,
}

pub struct ZoneBrowser {
    fetches: Vec<(String, DirectoryClient)>,
    pub zones: Vec<ZoneInfo>,
    pub selected: usize,
    /// Progress or problems, shown under the list.
    pub status: String,
    pinger: Option<ZonePinger>,
    last_ping: Option<Instant>,
    started: Instant,
}

impl ZoneBrowser {
    /// Starts fetching from every configured directory.
    pub fn open(directories: &[String]) -> Self {
        let mut status = Vec::new();
        let mut fetches = Vec::new();
        for d in directories {
            match DirectoryClient::connect(d, 0) {
                Ok(c) => fetches.push((d.clone(), c)),
                Err(e) => status.push(format!("{d}: {e}")),
            }
        }
        if directories.is_empty() {
            status.push(
                "No directory servers set. Add them under [directory] servers in the settings file."
                    .into(),
            );
        } else if !fetches.is_empty() {
            status.push("Fetching zone list...".into());
        }
        Self {
            fetches,
            zones: Vec::new(),
            selected: 0,
            status: status.join("  "),
            pinger: ZonePinger::new().ok(),
            last_ping: None,
            started: Instant::now(),
        }
    }

    /// Network work for this frame.
    pub fn pump(&mut self) {
        let timed_out = self.started.elapsed() > FETCH_TIMEOUT;
        let mut done = Vec::new();
        let mut found = Vec::new();
        for (i, (name, client)) in self.fetches.iter_mut().enumerate() {
            if let Err(e) = client.pump() {
                self.status = format!("{name}: {e}");
                done.push(i);
                continue;
            }
            match client.result() {
                Some(Ok(zones)) => {
                    found.extend(zones.iter().cloned());
                    done.push(i);
                }
                Some(Err(e)) => {
                    self.status = format!("{name}: {e}");
                    done.push(i);
                }
                None if timed_out => {
                    self.status = format!("{name}: no answer");
                    done.push(i);
                }
                None => {}
            }
        }
        for i in done.into_iter().rev() {
            self.fetches.remove(i);
        }
        if !found.is_empty() {
            self.add(found);
        }
        if self.fetches.is_empty() && self.status.starts_with("Fetching") {
            self.status = format!("{} zones", self.zones.len());
        }
        if let Some(p) = self.pinger.as_mut() {
            p.update(&mut self.zones);
            if !self.zones.is_empty() && self.last_ping.is_none_or(|t| t.elapsed() > PING_EVERY) {
                p.ping(&self.zones);
                self.last_ping = Some(Instant::now());
            }
        }
    }

    /// Adds zones, skipping addresses already listed (several directories often
    /// list the same zones).
    fn add(&mut self, zones: Vec<ZoneInfo>) {
        for z in zones {
            if !self.zones.iter().any(|o| o.address == z.address) {
                self.zones.push(z);
            }
        }
        self.zones
            .sort_by(|a, b| b.players.cmp(&a.players).then(a.name.cmp(&b.name)));
        self.last_ping = None;
    }

    /// Keys: Up/Down choose, Enter picks, Esc goes back.
    pub fn input(&mut self, pressed: &[KeyCode]) -> Option<BrowserChoice> {
        for k in pressed {
            match k {
                KeyCode::Escape => return Some(BrowserChoice::Back),
                KeyCode::ArrowDown => {
                    self.selected = (self.selected + 1).min(self.zones.len().saturating_sub(1))
                }
                KeyCode::ArrowUp => self.selected = self.selected.saturating_sub(1),
                KeyCode::PageDown => {
                    self.selected = (self.selected + 10).min(self.zones.len().saturating_sub(1))
                }
                KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    if let Some(z) = self.zones.get(self.selected) {
                        return Some(BrowserChoice::Pick(z.address.clone()));
                    }
                }
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str, address: &str, players: u32) -> ZoneInfo {
        ZoneInfo {
            name: name.into(),
            description: String::new(),
            address: address.into(),
            players,
            ping_ms: None,
        }
    }

    #[test]
    fn merges_sorts_and_picks() {
        let mut b = ZoneBrowser::open(&[]);
        assert!(b.status.contains("No directory servers"));
        b.add(vec![
            zone("Quiet", "127.0.0.1:5000", 1),
            zone("Busy", "127.0.0.1:6000", 40),
        ]);
        b.add(vec![
            zone("Busy (again)", "127.0.0.1:6000", 40),
            zone("Mid", "127.0.0.1:7000", 9),
        ]);
        let names: Vec<&str> = b.zones.iter().map(|z| z.name.as_str()).collect();
        assert_eq!(names, ["Busy", "Mid", "Quiet"]);
        assert_eq!(
            b.input(&[KeyCode::ArrowDown, KeyCode::ArrowDown, KeyCode::ArrowDown]),
            None
        );
        assert_eq!(b.selected, 2);
        assert_eq!(
            b.input(&[KeyCode::ArrowUp, KeyCode::Enter]),
            Some(BrowserChoice::Pick("127.0.0.1:7000".into()))
        );
        assert_eq!(b.input(&[KeyCode::Escape]), Some(BrowserChoice::Back));
    }
}
