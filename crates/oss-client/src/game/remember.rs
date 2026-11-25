//! Remembering the password: the start-up screen's side of `oss-credstore`.
//!
//! Every call to a password store is made away from the thread that draws. On
//! Windows and macOS they are quick; on Linux each one is a question put to the
//! desktop's keyring over its message bus, which can be slow, or can put a
//! prompt up and wait for the player. A screen that waited with it would freeze.
//!
//! What happens, and when:
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
use std::sync::mpsc::Sender;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use oss_credstore::{OsStore, PasswordStore, account};

use super::input::Stroke;
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
                // feature. One that will not even open is not offered.
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

/// A job for the store, run off the drawing thread.
type Job = Box<dyn FnOnce() + Send>;

/// The one thread every store job runs on, in the order they were asked for.
///
/// One queue rather than a thread per job, because order matters: untick
/// (delete), then tick again and play (save), must not end with the delete
/// arriving last and the password gone. On Linux, where each call is a question
/// to the desktop's keyring that may take a while, two threads could finish
/// either way round.
fn queue() -> Option<&'static Mutex<Sender<Job>>> {
    static QUEUE: OnceLock<Option<Mutex<Sender<Job>>>> = OnceLock::new();
    QUEUE
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("password-store".into())
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                })
                .map_err(|e| log::warn!("couldn't start the password store's thread: {e}"))
                .ok()?;
            Some(Mutex::new(tx))
        })
        .as_ref()
}

/// Runs `job` with the store on the store's own thread, after any job asked for
/// before it, so the screen never waits. The answer says when it has finished:
/// the screen ignores it, and a test waits on it.
fn in_background(
    store: &Arc<dyn PasswordStore>,
    job: impl FnOnce(&dyn PasswordStore) + Send + 'static,
) -> Receiver<()> {
    let (done, finished) = mpsc::channel();
    let store = Arc::clone(store);
    let job: Job = Box::new(move || {
        job(store.as_ref());
        let _ = done.send(());
    });
    match queue().map(|q| q.lock().map(|tx| tx.send(job))) {
        Some(Ok(Ok(()))) => {}
        _ => log::warn!("couldn't reach the password store"),
    }
    finished
}

/// Deletes the saved password for `who`, as unticking Remember asks.
pub fn forget(store: &Arc<dyn PasswordStore>, who: String) -> Receiver<()> {
    in_background(store, move |s| match s.forget(&who) {
        Ok(()) => log::info!("forgot the saved password for {who}"),
        Err(e) => log::warn!("couldn't forget the saved password for {who}: {e}"),
    })
}

/// The zone has accepted the login made from `login`: keep the password if
/// Remember is ticked, and delete any kept one if it is not.
pub fn settle(store: &Arc<dyn PasswordStore>, login: &Login) -> Option<Receiver<()>> {
    if !login.vault_shown() || login.name.trim().is_empty() {
        return None;
    }
    let who = account(&login.name, &login.server);
    if login.remember && !login.password.is_empty() {
        let password = login.password.clone();
        Some(in_background(store, move |s| {
            match s.save(&who, &password) {
                Ok(()) => log::info!("saved the password for {who}"),
                Err(e) => log::warn!("couldn't save the password for {who}: {e}"),
            }
        }))
    } else {
        Some(forget(store, who))
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
        self.follow(login);
        let who = account_of(login);

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

    /// Takes a password we filled in back out of the box when the form no
    /// longer names the account it was saved for. Says whether it did.
    ///
    /// Called every frame, and again at the moment of Play: the zone can change
    /// and Play be pressed in the same frame -- Enter on a zone in the list
    /// picks it and plays at once -- and without the second call the password
    /// saved for the old zone went to the new one, and was then filed under
    /// the new zone's name if it let us in.
    pub fn follow(&mut self, login: &mut Login) -> bool {
        match &self.filled_for {
            Some(filled) if account_of(login).as_ref() != Some(filled) => {
                login.password.clear();
                login.remember = false;
                self.filled_for = None;
                true
            }
            _ => false,
        }
    }

    /// What the store said about the account the form names now: `Some(true)`
    /// when its saved password is in the box, `Some(false)` when it has none,
    /// `None` while it has not been asked yet or is still answering.
    pub fn answer_for(&self, login: &Login) -> Option<bool> {
        let who = account_of(login)?;
        if self.filled_for.as_ref() == Some(&who) {
            return Some(true);
        }
        let answering = self.pending.as_ref().is_some_and(|(w, _)| *w == who);
        (self.asked.as_ref() == Some(&who) && !answering).then_some(false)
    }
}

/// The account the form's name and zone add up to, when they add up to one.
fn account_of(login: &Login) -> Option<String> {
    (!login.name.trim().is_empty() && login.server.contains(':'))
        .then(|| account(&login.name, &login.server))
}

/// How long a Play waits for the store to look up a saved password before it
/// goes ahead without one. Past the store's own deadline for saying whether it
/// is there, plus the pause before a lookup and the lookup itself.
pub const HOLD_LIMIT: Duration = Duration::from_secs(8);

/// What to do with a Play once the store has said there is no saved password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfNone {
    /// Connect with the box as it is. The player pressed Play themself, so an
    /// empty password is what they asked for.
    Play,
    /// Stay on the form with the password box waiting. Nobody pressed anything:
    /// the command line named a zone and no password, and sending an empty one
    /// unasked is a wrong try that can count towards a zone's lock-out.
    Stay,
}

/// A Play held back until the saved password has been looked up.
///
/// Without it, Play pressed in the moment between the form appearing and the
/// store answering -- the store is asked off the drawing thread, after the name
/// and zone have stood still for a moment -- connected with an empty password
/// although one was saved for that very name and zone, and the zone refused it.
#[derive(Debug, Clone, Copy)]
pub struct Hold {
    pub until: Instant,
    pub if_none: IfNone,
}

/// What a held Play does this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Wait,
    Play,
    Stay,
}

impl Hold {
    /// Whether this frame's keys or click take the form back from a waiting
    /// Play: any key going down, or a click. The OS repeating a key still held
    /// is not one, or a held Enter would cancel its own Play.
    pub fn interrupted(clicked: bool, strokes: &[Stroke]) -> bool {
        clicked || strokes.iter().any(|s| !s.repeat)
    }

    pub fn new(now: Instant, if_none: IfNone) -> Self {
        Self {
            until: now + HOLD_LIMIT,
            if_none,
        }
    }

    /// Whether a Play with this form would be held back at all: only with the
    /// password box empty, a name and zone to look up, and a store that is
    /// there or still saying whether it is.
    pub fn needed(login: &Login, recall: &Recall, vault: &Vault) -> bool {
        login.password.is_empty()
            && account_of(login).is_some()
            && !matches!(vault, Vault::None)
            && recall.answer_for(login).is_none()
    }

    /// Decides, from the store and the form as they are now.
    pub fn verdict(&self, login: &Login, recall: &Recall, vault: &Vault, now: Instant) -> Verdict {
        let none = match self.if_none {
            IfNone::Play => Verdict::Play,
            IfNone::Stay => Verdict::Stay,
        };
        // A password in the box, recalled or typed, is what Play wanted.
        if !login.password.is_empty() {
            return Verdict::Play;
        }
        // Nothing to look up, or nowhere to look it up: there is no saved one.
        if account_of(login).is_none() || matches!(vault, Vault::None) {
            return none;
        }
        match recall.answer_for(login) {
            Some(_) => none,
            None if now >= self.until => {
                log::info!("the password store did not answer in time");
                none
            }
            None => Verdict::Wait,
        }
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

    /// Waits for a background job to finish.
    fn join(job: Option<Receiver<()>>) {
        job.expect("a job was asked for")
            .recv_timeout(Duration::from_secs(5))
            .expect("and finished");
    }

    fn kept(store: &MemoryStore, who: &str) -> Option<String> {
        store.load(who).expect("readable")
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
        join(settle(&store, &l));
        assert_eq!(kept(&mem, &who), None);

        l.remember = true;
        join(settle(&store, &l));
        assert_eq!(kept(&mem, &who).as_deref(), Some(SECRET));

        l.remember = false;
        join(settle(&store, &l));
        assert_eq!(kept(&mem, &who), None);
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
        join(Some(forget(&store, asked)));
        assert_eq!(kept(&mem, &who), None);
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

    /// Play pressed before the store has answered waited for nothing and sent
    /// an empty password, although one was saved for that very name and zone.
    /// It now waits for the saved one and plays with it.
    #[test]
    fn play_waits_for_the_saved_password() {
        let mem = Arc::new(MemoryStore::default());
        let store: Arc<dyn PasswordStore> = mem.clone();
        mem.save(&account("Pilot", "zone.example:5000"), SECRET)
            .expect("saved");
        let vault = Vault::Ready(Arc::clone(&store));
        let mut l = form(true);
        let mut recall = Recall::default();
        let start = Instant::now();

        assert!(
            Hold::needed(&l, &recall, &vault),
            "Play with an empty box before the store has answered is held"
        );
        for if_none in [IfNone::Play, IfNone::Stay] {
            let hold = Hold::new(start, if_none);
            assert_eq!(hold.verdict(&l, &recall, &vault, start), Verdict::Wait);
        }
        let hold = Hold::new(start, IfNone::Stay);
        tick_until(&mut recall, &mut l, &store, |l| !l.password.is_empty());
        assert_eq!(hold.verdict(&l, &recall, &vault, start), Verdict::Play);
        assert_eq!(l.password, SECRET, "and plays with the saved password");
        assert!(!Hold::needed(&l, &recall, &vault));
    }

    /// Nothing saved: a Play the player pressed goes ahead with the box as it
    /// is, and the command line's waits on the form for a password to be typed.
    #[test]
    fn with_nothing_saved_the_player_plays_and_the_command_line_stays() {
        let mem = Arc::new(MemoryStore::default());
        let store: Arc<dyn PasswordStore> = mem.clone();
        let vault = Vault::Ready(Arc::clone(&store));
        let mut l = form(true);
        let mut recall = Recall::default();
        let start = Instant::now();
        let mut now = start;
        while recall.answer_for(&l).is_none() && start.elapsed() < Duration::from_secs(5) {
            now += SETTLE;
            recall.tick(&mut l, &store, now);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            recall.answer_for(&l),
            Some(false),
            "asked, and nothing saved"
        );
        assert!(
            !Hold::needed(&l, &recall, &vault),
            "nothing left to wait for"
        );
        let play = Hold::new(start, IfNone::Play);
        let stay = Hold::new(start, IfNone::Stay);
        assert_eq!(play.verdict(&l, &recall, &vault, start), Verdict::Play);
        assert_eq!(stay.verdict(&l, &recall, &vault, start), Verdict::Stay);
    }

    /// No store on this machine: nothing is held, and the command line's Play
    /// stays on the form rather than sending an empty password.
    #[test]
    fn with_no_store_nothing_is_held() {
        let l = form(false);
        let recall = Recall::default();
        let now = Instant::now();
        assert!(!Hold::needed(&l, &recall, &Vault::None));
        assert_eq!(
            Hold::new(now, IfNone::Stay).verdict(&l, &recall, &Vault::None, now),
            Verdict::Stay
        );
        assert_eq!(
            Hold::new(now, IfNone::Play).verdict(&l, &recall, &Vault::None, now),
            Verdict::Play
        );
    }

    /// A store that never answers holds Play for [`HOLD_LIMIT`] and no longer.
    #[test]
    fn a_store_that_never_answers_is_waited_for_only_so_long() {
        let (_still_asking, rx) = mpsc::channel();
        let vault = Vault::Asking(rx, Instant::now());
        let l = form(false);
        let recall = Recall::default();
        let start = Instant::now();
        assert!(Hold::needed(&l, &recall, &vault));
        let hold = Hold::new(start, IfNone::Play);
        assert_eq!(hold.verdict(&l, &recall, &vault, start), Verdict::Wait);
        assert_eq!(
            hold.verdict(&l, &recall, &vault, start + HOLD_LIMIT),
            Verdict::Play
        );
    }

    /// A key going down takes the form back from a waiting Play, so a
    /// character typed while it waits is not sent as the whole password. The
    /// OS repeating the Enter that started it does not.
    #[test]
    fn a_waiting_play_gives_way_to_the_next_key_only() {
        use winit::keyboard::KeyCode;
        assert!(Hold::interrupted(false, &[Stroke::text('x')]));
        assert!(Hold::interrupted(false, &[Stroke::press(KeyCode::Enter)]));
        assert!(Hold::interrupted(true, &[]));
        assert!(!Hold::interrupted(
            false,
            &[Stroke::press(KeyCode::Enter).repeated(true)]
        ));
        assert!(!Hold::interrupted(false, &[]));
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
