//! The zone browser: zones from the configured directory servers, with live
//! population and ping. Opened with F3 on the login screen.
//!
//! Only directory servers listed in the settings (`[directory] servers`) or given
//! with `--directory` are contacted. A fresh install starts with the game's own,
//! [`crate::config::DEFAULT_DIRECTORIES`], so the browser has somewhere to ask;
//! `servers = none` in the settings file turns that off.

use std::time::{Duration, Instant};

use super::input::Stroke;
use oss_session::directory::{DirectoryClient, ZoneInfo, ZonePinger};
use winit::keyboard::KeyCode;

/// "1 zone" or "7 zones" -- the count goes in front of the player, so it reads
/// as English rather than as a field.
fn count_zones(n: usize) -> String {
    match n {
        1 => "1 zone".to_owned(),
        n => format!("{n} zones"),
    }
}

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
    /// Progress or problems, shown under the list. Written only by
    /// [`ZoneBrowser::restate`], from the three counts below it.
    pub status: String,
    /// How many directories were configured, for "1 of 3 did not answer".
    configured: usize,
    /// Directories that could not be reached, as "name: reason". Held apart
    /// from [`ZoneBrowser::status`] so that one directory being down cannot
    /// overwrite a list the others returned.
    failed: Vec<String>,
    pinger: Option<ZonePinger>,
    last_ping: Option<Instant>,
    started: Instant,
}

impl ZoneBrowser {
    /// Starts fetching from every configured directory.
    pub fn open(directories: &[String]) -> Self {
        let mut failed = Vec::new();
        let mut fetches = Vec::new();
        for d in directories {
            match DirectoryClient::connect(d, 0) {
                Ok(c) => fetches.push((d.clone(), c)),
                Err(e) => failed.push(format!("{d}: {e}")),
            }
        }
        let mut browser = Self {
            fetches,
            zones: Vec::new(),
            selected: 0,
            touched: false,
            status: String::new(),
            configured: directories.len(),
            failed,
            pinger: ZonePinger::new().ok(),
            last_ping: None,
            started: Instant::now(),
        };
        browser.restate();
        browser
    }

    /// Rewrites the line under the list from what has actually happened: the
    /// zones that arrived, the directories still being waited on, and the ones
    /// that failed.
    ///
    /// This used to be whatever had gone wrong most recently, and a failure
    /// overwrote it outright -- so a directory that was down reported itself
    /// while the zones the others had returned sat on the screen above it.
    /// Shipping three by default made that the ordinary case rather than a
    /// rare one, because two of the three do not answer today.
    fn restate(&mut self) {
        if self.configured == 0 && self.zones.is_empty() {
            self.status =
                "No directory servers set. Add them under [directory] servers in the settings file."
                    .into();
            return;
        }
        let counted = count_zones(self.zones.len());
        let mut line = match (self.fetches.is_empty(), self.zones.is_empty()) {
            (false, true) => "Fetching zone list...".to_owned(),
            (false, false) => format!("{counted}, still fetching..."),
            (true, _) => counted,
        };
        // A directory that is down is worth a word once the list it could not
        // contribute to has arrived. When nothing arrived at all it is the
        // whole story, so it is told in full -- an empty browser that explains
        // itself beats one that just sits there.
        if !self.failed.is_empty() {
            if !self.zones.is_empty() {
                line += &format!(
                    "  ({} of {} directories did not answer)",
                    self.failed.len(),
                    self.configured
                );
            } else if self.fetches.is_empty() {
                line = self.failed.join("  ");
            }
        }
        self.status = line;
    }

    /// Network work for this frame.
    pub fn pump(&mut self) {
        let timed_out = self.started.elapsed() > FETCH_TIMEOUT;
        let mut done = Vec::new();
        let mut found = Vec::new();
        for (i, (name, client)) in self.fetches.iter_mut().enumerate() {
            if let Err(e) = client.pump() {
                self.failed.push(format!("{name}: {e}"));
                done.push(i);
                continue;
            }
            match client.result() {
                Some(Ok(zones)) => {
                    found.extend(zones.iter().cloned());
                    done.push(i);
                }
                Some(Err(e)) => {
                    self.failed.push(format!("{name}: {e}"));
                    done.push(i);
                }
                None if timed_out => {
                    self.failed.push(format!("{name}: no answer"));
                    done.push(i);
                }
                None => {}
            }
        }
        let settled = !done.is_empty();
        for i in done.into_iter().rev() {
            self.fetches.remove(i);
        }
        if !found.is_empty() {
            // `add` restates for us once the new zones are in and sorted.
            self.add(found);
        } else if settled {
            self.restate();
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
        self.restate();
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

    /// Moves the selection by whole rows, stopping at the ends.
    ///
    /// The scroll wheel's path in. Continuum's skin gives the list `zoneup` and
    /// `zonedown` buttons beside it; a wheel over the list is where a pointer
    /// looks for those now.
    pub fn scroll_by(&mut self, rows: i32) {
        if self.zones.is_empty() || rows == 0 {
            return;
        }
        self.touched = true;
        let last = self.zones.len() as i64 - 1;
        self.selected = (self.selected as i64 + rows as i64).clamp(0, last) as usize;
    }

    /// Picks a row outright, as a click on it does. Out-of-range is ignored
    /// rather than clamped: a click that lands on no row is not a choice.
    pub fn select(&mut self, row: usize) {
        if row < self.zones.len() {
            self.touched = true;
            self.selected = row;
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

    /// A directory that is down must not hide the zones the other directories
    /// returned. The line under the list used to be whatever had gone wrong
    /// most recently, so once three directories shipped by default -- two of
    /// which do not answer today -- a full zone list announced itself as a
    /// failure.
    #[test]
    fn a_dead_directory_does_not_hide_the_zones_that_arrived() {
        // Neither of these carries a port, so `connect` refuses them on the
        // spot and no name is ever looked up: the test stays offline.
        let mut b = ZoneBrowser::open(&["nowhere".to_owned(), "also-nowhere".to_owned()]);
        assert!(b.zones.is_empty());
        assert!(
            !b.status.contains("No directory servers"),
            "two were set -- they failed, which is a different thing: {}",
            b.status
        );
        assert!(
            b.status.contains("nowhere"),
            "with nothing to show, the reason is the whole story: {}",
            b.status
        );

        b.add(vec![zone("Trench Wars", "10.0.0.1:5400", 49)]);
        assert!(
            b.status.starts_with("1 zone") && !b.status.starts_with("1 zones"),
            "the count reads as English, and leads: {}",
            b.status
        );
        assert!(
            b.status.contains("2 of 2 directories did not answer"),
            "the failures keep a word, after the list: {}",
            b.status
        );
    }

    /// With no directories at all there is nothing to wait for, so the browser
    /// says what to do rather than reporting an empty list.
    #[test]
    fn no_directories_configured_says_so() {
        let b = ZoneBrowser::open(&[]);
        assert!(b.status.contains("No directory servers"), "{}", b.status);
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
