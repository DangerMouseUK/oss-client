//! The zone browser: zones from the configured directory servers, with live
//! population and ping. Opened with F3 on the login screen.
//!
//! Only directory servers listed in the settings (`[directory] servers`) or given
//! with `--directory` are contacted. A fresh install starts with the game's own,
//! [`crate::config::DEFAULT_DIRECTORIES`], so the browser has somewhere to ask;
//! `servers = none` in the settings file turns that off.

use std::time::{Duration, Instant};

use super::input::Stroke;
use super::login::Login;
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
    /// A refresh is running and the zones on screen are the ones from last time,
    /// waiting to be replaced by the first directory that answers.
    refreshing: bool,
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
            refreshing: false,
        };
        browser.restate();
        browser
    }

    /// Asks the directories again, keeping the list that is already on screen.
    ///
    /// F3 used to throw the browser away and build a new one, so every zone
    /// vanished for the three to eleven seconds the directories take to answer:
    /// the list went blank, the address stayed in the Zone field, and the only
    /// sign of life was a small line reading "Fetching zone list...". A player
    /// cannot tell that from a key that crashed something. So the previous list
    /// stays up, the line under it says a refresh is running, and the zones are
    /// replaced by the first directory that answers rather than up front.
    pub fn refresh(&mut self, directories: &[String]) {
        let zones = std::mem::take(&mut self.zones);
        let (selected, touched) = (self.selected, self.touched);
        *self = Self::open(directories);
        self.selected = selected.min(zones.len().saturating_sub(1));
        self.touched = touched;
        self.zones = zones;
        // Nothing to wait for (every directory refused on the spot, or none is
        // configured): this is not a refresh in progress, it is already over.
        self.refreshing = !self.fetches.is_empty();
        self.restate();
    }

    /// Whether a directory is still being waited on. The screen gives the line
    /// under the list priority over the pointer's hint while this holds, so that
    /// clicking the refresh button -- which leaves the pointer sitting on it --
    /// does not hide the progress behind the button's own description.
    pub fn busy(&self) -> bool {
        !self.fetches.is_empty()
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
            // A refresh with the previous list still on screen. Saying so is the
            // whole point of F3 having an answer: the rows below are last time's
            // until a directory replies.
            (false, false) if self.refreshing => {
                format!("Refreshing the zone list...  ({counted} from last time)")
            }
            // The refresh finished without a single directory answering, so what
            // is on screen is still the old list. The failures are appended below.
            (true, false) if self.refreshing => format!("{counted} from last time"),
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
        // The first answer to a refresh replaces last time's list outright, so a
        // zone that has gone away goes away; the directories after it merge into
        // the new list in the usual way.
        if self.refreshing {
            self.refreshing = false;
            self.zones.clear();
        }
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

    /// One frame of keys while the list has the focus on the start-up screen,
    /// and the Zone field following the row they settle on.
    ///
    /// **Only the keys that walk the list, or Enter picking a row, move the
    /// Zone field.** Any key at all used to -- the Tab or Shift that leaves the
    /// list included -- so tabbing through the form copied the highlighted row,
    /// usually the busiest zone rather than the one in the field, over the
    /// player's zone. That changed the account the form named, which takes
    /// out a saved password filled in for the old one: the password vanished
    /// from the screen while the player was only pressing Tab.
    pub fn form_keys(&mut self, login: &mut Login, strokes: &[Stroke]) {
        let walked = strokes.iter().any(|s| {
            matches!(
                s.key,
                Some(KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::PageUp | KeyCode::PageDown)
            )
        });
        let follow = match self.input(strokes) {
            Some(BrowserChoice::Pick(address)) => Some(address),
            _ if walked => self.zones.get(self.selected).map(|z| z.address.clone()),
            _ => None,
        };
        if let Some(address) = follow {
            login.server = address;
            login.error = None;
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

    /// What the lead saw: a saved password filled in for the zone in the field,
    /// then Tab round the form, through the list and off it -- and the password
    /// gone. The Tab leaving the list wrote the list's top row over the Zone
    /// field, the account changed, and the filled-in password went with it.
    #[test]
    fn tabbing_through_the_list_keeps_the_zone_and_its_saved_password() {
        use super::super::login::Field;
        use super::super::remember::Recall;
        use oss_credstore::{MemoryStore, PasswordStore, account};
        use std::sync::Arc;

        let mut b = ZoneBrowser::open(&[]);
        b.add(vec![
            zone("Busy", "127.0.0.1:6000", 40),
            zone("Mine", "127.0.0.1:5000", 3),
        ]);
        let mem = Arc::new(MemoryStore::default());
        mem.save(&account("Pilot", "127.0.0.1:5000"), "saved-for-tests")
            .expect("saved");
        let store: Arc<dyn PasswordStore> = mem;
        let mut l = Login::default();
        l.name = "Pilot".into();
        l.server = "127.0.0.1:5000".into();
        l.set_vault(true);
        let mut recall = Recall::default();
        let start = Instant::now();
        let mut now = start;
        while l.password.is_empty() && start.elapsed() < Duration::from_secs(5) {
            now += Duration::from_millis(500);
            recall.tick(&mut l, &store, now);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(l.password, "saved-for-tests", "filled in");

        // Tab once round the whole form, a frame per key, as the screen runs it.
        for _ in 0..l.fields().len() {
            let tab = keys(&[KeyCode::Tab]);
            if l.focused() == Field::Zones {
                b.form_keys(&mut l, &tab);
            }
            l.input(&tab, false);
            now += Duration::from_millis(500);
            recall.tick(&mut l, &store, now);
        }
        assert_eq!(l.server, "127.0.0.1:5000", "the zone is the player's still");
        assert_eq!(l.password, "saved-for-tests", "and so is its password");
        assert!(l.remember);

        // Walking the list is choosing a zone, and does move the field.
        l.focus_zones();
        b.form_keys(&mut l, &keys(&[KeyCode::ArrowUp]));
        assert_eq!(l.server, "127.0.0.1:6000");
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

    /// A directory that is listening to nothing: the socket connects (UDP has no
    /// handshake) and nothing is ever sent, because these tests never `pump`. It
    /// is how a fetch-in-progress is reached without going near a real network.
    const PENDING: &str = "127.0.0.1:1";

    /// F3's real fault was not its wording. It threw the browser away, so every
    /// zone vanished for the seconds the directories take, and a refresh looked
    /// exactly like a key that had broken something.
    #[test]
    fn a_refresh_keeps_the_zones_on_screen_and_says_it_is_running() {
        let dirs = [PENDING.to_owned()];
        let mut b = ZoneBrowser::open(&dirs);
        b.add(vec![
            zone("Trench Wars", "10.0.0.1:5400", 49),
            zone("Hyperspace", "10.0.0.2:5400", 34),
        ]);
        b.touched = true;
        b.selected = 1;

        b.refresh(&dirs);

        assert_eq!(
            names(&b),
            ["Trench Wars", "Hyperspace"],
            "the list a player was reading must still be there"
        );
        assert_eq!(b.selected, 1, "and so must the row they had highlighted");
        assert!(
            b.status.starts_with("Refreshing the zone list..."),
            "the line under the list is where a refresh announces itself: {}",
            b.status
        );
        assert!(
            b.status.contains("2 zones from last time"),
            "it has to say the rows below it are stale: {}",
            b.status
        );
        assert!(b.busy(), "a directory is still being waited on");

        // The first directory to answer replaces last time's list outright, so a
        // zone that has gone away goes away rather than lingering for ever.
        b.add(vec![zone("Hyperspace", "10.0.0.2:5400", 40)]);
        assert_eq!(names(&b), ["Hyperspace"]);
        assert!(
            b.status.starts_with("1 zone") && !b.status.contains("from last time"),
            "what is on screen is this fetch's, so it stops being called old: {}",
            b.status
        );
    }

    /// Every directory down during a refresh: the old rows stay, and the line
    /// says they are old. Saying "2 zones" there would be a lie about freshness.
    #[test]
    fn a_refresh_nobody_answers_admits_the_list_is_last_times() {
        let dirs = [PENDING.to_owned()];
        let mut b = ZoneBrowser::open(&dirs);
        b.add(vec![zone("Trench Wars", "10.0.0.1:5400", 49)]);
        b.refresh(&dirs);
        // What `pump` does when a directory gives up, without the ten-second wait.
        b.fetches.clear();
        b.failed.push(format!("{PENDING}: no answer"));
        b.restate();

        assert!(!b.busy(), "nothing left to wait for");
        assert!(
            b.status.starts_with("1 zone from last time"),
            "{}",
            b.status
        );
        assert!(
            b.status.contains("1 of 1 directories did not answer"),
            "{}",
            b.status
        );
        assert_eq!(names(&b), ["Trench Wars"]);
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
