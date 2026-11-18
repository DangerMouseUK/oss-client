//! Remembering the password: the start-up screen's side of `oss-credstore`.
//!
//! Every call to a password store is made away from the thread that draws. On
//! Windows and macOS they are quick; on Linux each one is a question put to the
//! desktop's keyring over its message bus, which can be slow, or can put a
//! prompt up and wait for the player. A screen that waited with it would freeze.
//!
//! What happens, and when (`D34`, and the research behind it):
//!
//! * **At start**, [`Vault::probe`] asks whether this machine has a store. Until
//!   it answers, and for good if it says no, the form has no Remember line.
//! * **With a name and a zone filled in and the password box empty**,
//!   [`Recall`] asks the store for a saved password, and puts it in the box and
//!   ticks Remember if there is one. It never writes over anything typed.
//! * **When the zone accepts the login** -- not before, so a mistyped password
//!   is never kept -- [`settle`] saves the password if Remember is ticked, and
//!   deletes any saved one if it is not.
//! * **Unticking** deletes the saved password at once.
//!
//! Log lines name the entry (`pilot@zone:5000`) and never the password.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use oss_credstore::{OsStore, PasswordStore, account};

use super::login::Login;

/// How long the store has to say whether it is there before the client stops
/// waiting and carries on without it.
const PROBE_DEADLINE: Duration = Duration::from_secs(5);
/// How long the name and zone must stay as they are before the store is asked
/// about them, so that typing a name does not ask once per letter.
const SETTLE: Duration = Duration::from_millis(400);

/// This machine's password store, once it has said whether there is one.
pub enum Vault {
    /// Still asking, since `started`.
    Asking(Receiver<Option<Arc<dyn PasswordStore>>>, Instant),
    Ready(Arc<dyn PasswordStore>),
    /// No store, or none answered in time.
    None,
}

impl Vault {
    /// Asks, on a thread of its own, whether this machine has a password store.
    pub fn probe() -> Self {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("password-store".into())
            .spawn(move || {
                // A store that opens is offered even if it turns out to be
                // locked when a password is read or saved: the store asks the
                // player then, and saying no is their choice, not a missing
                // feature (`D11b`). One that will not even open is not offered.
                let store: Option<Arc<dyn PasswordStore>> = match OsStore::open() {
                    Ok(Some(s)) => Some(Arc::new(s)),
                    Ok(None) => None,
                    Err(e) => {
                        log::info!("password store: {e}");
                        None
                    }
                };
                let _ = tx.send(store);
            });
        match spawned {
            Ok(_) => Vault::Asking(rx, Instant::now()),
            Err(e) => {
                log::warn!("couldn't ask for a password store: {e}");
                Vault::None
            }
        }
    }

    /// Takes the answer, if it has come, or gives up once the deadline passes.
    pub fn poll(&mut self) {
        let Vault::Asking(rx, started) = self else {
            return;
        };
        *self = match rx.try_recv() {
            Ok(Some(store)) => {
                log::info!("password store found: Remember password is offered");
                Vault::Ready(store)
            }
            Ok(None) | Err(TryRecvError::Disconnected) => Vault::None,
            Err(TryRecvError::Empty) if started.elapsed() > PROBE_DEADLINE => {
                log::info!("the password store did not answer in time; not offering to remember");
                Vault::None
            }
            Err(TryRecvError::Empty) => return,
        };
    }

    pub fn store(&self) -> Option<Arc<dyn PasswordStore>> {
        match self {
            Vault::Ready(s) => Some(Arc::clone(s)),
            _ => None,
        }
    }
}

/// Runs `job` with the store on a thread of its own, so the screen never waits.
fn in_background(
    store: &Arc<dyn PasswordStore>,
    job: impl FnOnce(&dyn PasswordStore) + Send + 'static,
) {
    let store = Arc::clone(store);
    if let Err(e) = std::thread::Builder::new()
        .name("password-store".into())
        .spawn(move || job(store.as_ref()))
    {
        log::warn!("couldn't reach the password store: {e}");
    }
}

/// Deletes the saved password for `who`, as unticking Remember asks.
pub fn forget(store: &Arc<dyn PasswordStore>, who: String) {
    in_background(store, move |s| match s.forget(&who) {
        Ok(()) => log::info!("forgot the saved password for {who}"),
        Err(e) => log::warn!("couldn't forget the saved password for {who}: {e}"),
    });
}

/// The zone has accepted the login made from `login`: keep the password if
/// Remember is ticked, and delete any kept one if it is not.
pub fn settle(store: &Arc<dyn PasswordStore>, login: &Login) {
    if !login.vault_shown() || login.name.trim().is_empty() {
        return;
    }
    let who = account(&login.name, &login.server);
    if login.remember && !login.password.is_empty() {
        let password = login.password.clone();
        in_background(store, move |s| match s.save(&who, &password) {
            Ok(()) => log::info!("saved the password for {who}"),
            Err(e) => log::warn!("couldn't save the password for {who}: {e}"),
        });
    } else {
        forget(store, who);
    }
}

/// Fills in a saved password on the start-up screen.
#[derive(Default)]
pub struct Recall {
    /// The account the name and zone add up to now, and since when.
    watching: Option<(String, Instant)>,
    /// The account last asked about, so it is asked once.
    asked: Option<String>,
    /// An answer on its way.
    pending: Option<(String, Receiver<Option<String>>)>,
    /// The account whose saved password is in the box, put there by us rather
    /// than typed: if the name or zone then changes, it is taken out again.
    filled_for: Option<String>,
}

impl Recall {
    /// One frame's worth: notices a new name or zone, asks once it has stopped
    /// changing, and fills the box when the answer comes.
    pub fn tick(&mut self, login: &mut Login, store: &Arc<dyn PasswordStore>, now: Instant) {
        let who = (!login.name.trim().is_empty() && login.server.contains(':'))
            .then(|| account(&login.name, &login.server));

        // A password we filled in belongs to the account it was saved for.
        if let Some(filled) = &self.filled_for
            && who.as_ref() != Some(filled)
        {
            login.password.clear();
            login.remember = false;
            self.filled_for = None;
        }

        if let Some((asked_for, rx)) = &self.pending {
            match rx.try_recv() {
                Ok(found) => {
                    if let Some(password) = found
                        && who.as_ref() == Some(asked_for)
                        && login.password.is_empty()
                    {
                        login.password = password;
                        login.remember = true;
                        self.filled_for = Some(asked_for.clone());
                    }
                    self.pending = None;
                }
                Err(TryRecvError::Disconnected) => self.pending = None,
                Err(TryRecvError::Empty) => {}
            }
        }

        let Some(who) = who else {
            self.watching = None;
            return;
        };
        let since = match &self.watching {
            Some((w, since)) if *w == who => *since,
            _ => {
                self.watching = Some((who, now));
                return;
            }
        };
        if now.duration_since(since) < SETTLE
            || self.asked.as_ref() == Some(&who)
            || self.pending.is_some()
            || !login.password.is_empty()
        {
            return;
        }
        self.asked = Some(who.clone());
        let (tx, rx) = mpsc::channel();
        let asking = who.clone();
        in_background(store, move |s| {
            let found = match s.load(&asking) {
                Ok(found) => found,
                Err(e) => {
                    log::info!("couldn't read a saved password for {asking}: {e}");
                    None
                }
            };
            let _ = tx.send(found);
        });
        self.pending = Some((who, rx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oss_credstore::MemoryStore;

    const SECRET: &str = "s3cret-for-tests";

    fn form(store: bool) -> Login {
        let mut l = Login::default();
        l.name = "Pilot".into();
        l.server = "zone.example:5000".into();
        l.set_vault(store);
        l
    }

    /// Waits for the background threads, ticking as the screen would.
    fn tick_until(
        recall: &mut Recall,
        l: &mut Login,
        store: &Arc<dyn PasswordStore>,
        done: impl Fn(&Login) -> bool,
    ) {
        let start = Instant::now();
        let mut now = start;
        while !done(l) && start.elapsed() < Duration::from_secs(5) {
            now += SETTLE;
            recall.tick(l, store, now);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for(store: &MemoryStore, who: &str, want: Option<&str>) {
        let start = Instant::now();
        while store.load(who).ok().flatten().as_deref() != want
            && start.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(store.load(who).ok().flatten().as_deref(), want);
    }

    /// Ticked and accepted: saved. Unticked and accepted: deleted. And `settle`
    /// is only ever called once the zone has accepted, so a refused login --
    /// a mistyped password -- is never kept.
    #[test]
    fn the_zone_accepting_is_what_saves_it() {
        let mem = Arc::new(MemoryStore::default());
        let store: Arc<dyn PasswordStore> = mem.clone();
        let who = account("Pilot", "zone.example:5000");

        let mut l = form(true);
        l.password = SECRET.into();
        assert!(!l.remember, "unticked to begin with");
        settle(&store, &l);
        wait_for(&mem, &who, None);

        l.remember = true;
        settle(&store, &l);
        wait_for(&mem, &who, Some(SECRET));

        l.remember = false;
        settle(&store, &l);
        wait_for(&mem, &who, None);
    }

    /// Unticking deletes the saved password at once, without waiting to play.
    #[test]
    fn unticking_forgets_at_once() {
        let mem = Arc::new(MemoryStore::default());
        let store: Arc<dyn PasswordStore> = mem.clone();
        let who = account("Pilot", "zone.example:5000");
        mem.save(&who, SECRET).expect("saved");
        let mut l = form(true);
        l.remember = true;
        l.toggle_remember();
        let asked = l.take_forget().expect("asked to forget");
        assert_eq!(asked, who);
        forget(&store, asked);
        wait_for(&mem, &who, None);
    }

    /// A saved password fills an empty box and ticks Remember; it never writes
    /// over a password being typed; and it leaves again if the name changes.
    #[test]
    fn a_saved_password_fills_an_empty_box_only() {
        let mem = Arc::new(MemoryStore::default());
        let store: Arc<dyn PasswordStore> = mem.clone();
        mem.save(&account("Pilot", "zone.example:5000"), SECRET)
            .expect("saved");

        let mut l = form(true);
        let mut recall = Recall::default();
        tick_until(&mut recall, &mut l, &store, |l| !l.password.is_empty());
        assert_eq!(l.password, SECRET);
        assert!(l.remember);

        // The name changes: that password was someone else's.
        l.name = "Another".into();
        recall.tick(&mut l, &store, Instant::now());
        assert!(l.password.is_empty());
        assert!(!l.remember);

        // Typed first: left alone.
        let mut l = form(true);
        l.password = "typed".into();
        let mut recall = Recall::default();
        let start = Instant::now();
        for i in 1..20u32 {
            recall.tick(&mut l, &store, start + SETTLE * i);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(l.password, "typed");
    }

    /// No store, no line: the form is the six lines it always was, and Tab never
    /// lands on a Remember that is not drawn.
    #[test]
    fn no_store_no_line() {
        use super::super::login::Field;
        let mut l = form(false);
        assert!(!l.fields().contains(&Field::Remember));
        for _ in 0..20 {
            l.focus += 1;
            assert_ne!(l.focused(), Field::Remember);
        }
        l.toggle_remember();
        assert!(!l.remember, "nothing to tick");

        let l = form(true);
        let fields = l.fields();
        let at = fields.iter().position(|f| *f == Field::Remember);
        assert_eq!(at, Some(2), "under the password");
    }
}
