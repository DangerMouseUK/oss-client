//! The zone browser: zones from the configured directory servers, with live
//! population and ping. Opened with F3 on the login screen.
//!
//! Only directory servers listed in the settings (`[directory] servers`) or given
//! with `--directory` are contacted; none are configured by default.

use std::time::{Duration, Instant};

use super::input::Stroke;
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
    /// Whether the player has moved the selection. Until they have, the highlight
    /// belongs on the top row and should follow the sort, not a particular zone.
    touched: bool,
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
            touched: false,
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
        // The pings carry the real populations; the directory's figures were only a
        // starting point. Re-sort when they disagree, or the list ends up ordered by
        // one set of numbers while displaying another -- which is how a 5-player
        // zone came to sit below a 4-player one.
        let mut population_changed = false;
        if let Some(p) = self.pinger.as_mut() {
            population_changed = p.update(&mut self.zones);
        }
        if population_changed {
            self.sort();
        }
        if let Some(p) = self.pinger.as_mut()
            && !self.zones.is_empty()
            && self.last_ping.is_none_or(|t| t.elapsed() > PING_EVERY)
        {
            p.ping(&self.zones);
            self.last_ping = Some(Instant::now());
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
        self.sort();
        self.last_ping = None;
    }

    /// Orders by population, busiest first, keeping whatever the player had
    /// highlighted under the highlight: re-sorting out from under a selection
    /// would move the row they were about to choose.
    fn sort(&mut self) {
        // Only once they have chosen a row is there anything to keep: before that
        // the highlight marks "the top of the list", and pinning it to whichever
        // zone happened to be first while the directories were still arriving left
        // it stranded on an arbitrary one.
        let chosen = self
            .touched
            .then(|| self.zones.get(self.selected).map(|z| z.address.clone()))
            .flatten();
        self.zones
            .sort_by(|a, b| b.players.cmp(&a.players).then(a.name.cmp(&b.name)));
        match chosen.and_then(|a| self.zones.iter().position(|z| z.address == a)) {
            Some(i) => self.selected = i,
            None => self.selected = 0,
        }
    }

    /// Keys: Up/Down choose, Enter picks, Esc goes back.
    ///
    /// The keys that walk the list take the OS repeats, so holding Down runs
    /// down a long zone list; Enter and Escape act once however long they are
    /// held, so arriving on a row with Enter still down does not pick it twice.
    pub fn input(&mut self, strokes: &[Stroke]) -> Option<BrowserChoice> {
        for stroke in strokes {
            if let Some(KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter) = stroke.key {
                match stroke.first_press() {
                    Some(KeyCode::Escape) => return Some(BrowserChoice::Back),
                    Some(KeyCode::Enter | KeyCode::NumpadEnter) => {
                        if let Some(z) = self.zones.get(self.selected) {
                            return Some(BrowserChoice::Pick(z.address.clone()));
                        }
                    }
                    _ => {}
                }
                continue;
            }
            match stroke.key {
                Some(KeyCode::ArrowDown) => {
                    self.touched = true;
                    self.selected = (self.selected + 1).min(self.zones.len().saturating_sub(1))
                }
                Some(KeyCode::ArrowUp) => {
                    self.touched = true;
                    self.selected = self.selected.saturating_sub(1)
                }
                Some(KeyCode::PageDown) => {
                    self.touched = true;
                    self.selected = (self.selected + 10).min(self.zones.len().saturating_sub(1))
                }
                Some(KeyCode::PageUp) => {
                    self.touched = true;
                    self.selected = self.selected.saturating_sub(10)
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

    /// The keystrokes a frame of these keys arrives as.
    fn keys(keys: &[KeyCode]) -> Vec<Stroke> {
        keys.iter().copied().map(Stroke::press).collect()
    }

    fn zone(name: &str, address: &str, players: u32) -> ZoneInfo {
        ZoneInfo {
            name: name.into(),
            description: String::new(),
            address: address.into(),
            players,
            ping_ms: None,
        }
    }

    /// The directory's populations are a starting point; the pings carry the real
    /// ones. Sorting only on insert left the list ordered by the first set while
    /// showing the second -- live, a 5-player zone sat below a 4-player one.
    #[test]
    fn a_refreshed_population_reorders_the_list() {
        let mut b = ZoneBrowser::open(&[]);
        b.add(vec![
            zone("Busy", "127.0.0.1:6000", 40),
            zone("Middling", "127.0.0.1:7000", 4),
            zone("Quiet", "127.0.0.1:5000", 1),
        ]);
        assert_eq!(names(&b), ["Busy", "Middling", "Quiet"]);

        // The player is pointing at the one about to move.
        b.input(&keys(&[KeyCode::ArrowDown, KeyCode::ArrowDown]));
        assert_eq!(b.zones[b.selected].name, "Quiet");
        // A ping says Quiet is really the second busiest.
        b.zones[2].players = 9;
        b.sort();

        assert_eq!(names(&b), ["Busy", "Quiet", "Middling"]);
        // ... and the highlight followed the zone, not the row number.
        assert_eq!(b.zones[b.selected].name, "Quiet");
    }

    /// Until the player picks a row the highlight means "top of the list", so it
    /// must follow the sort rather than stay on whichever zone arrived first.
    #[test]
    fn an_untouched_selection_stays_at_the_top() {
        let mut b = ZoneBrowser::open(&[]);
        b.add(vec![zone("Quiet", "127.0.0.1:5000", 1)]);
        assert_eq!(b.zones[b.selected].name, "Quiet");
        // A busier zone arrives from a second directory and takes the top row.
        b.add(vec![zone("Busy", "127.0.0.1:6000", 40)]);
        assert_eq!(b.selected, 0);
        assert_eq!(b.zones[b.selected].name, "Busy");
    }

    fn names(b: &ZoneBrowser) -> Vec<&str> {
        b.zones.iter().map(|z| z.name.as_str()).collect()
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
            b.input(&keys(&[
                KeyCode::ArrowDown,
                KeyCode::ArrowDown,
                KeyCode::ArrowDown
            ])),
            None
        );
        assert_eq!(b.selected, 2);
        assert_eq!(
            b.input(&keys(&[KeyCode::ArrowUp, KeyCode::Enter])),
            Some(BrowserChoice::Pick("127.0.0.1:7000".into()))
        );
        assert_eq!(
            b.input(&keys(&[KeyCode::Escape])),
            Some(BrowserChoice::Back)
        );
    }
}
