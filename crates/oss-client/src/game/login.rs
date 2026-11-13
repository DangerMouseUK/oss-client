//! The start-up screen's form: name, password, arena, zone address and ship.
//!
//! Continuum's start-up screen is one screen carrying the profile, the ship you will
//! fly, the zone list and the way through to options, so this is one part of ours
//! rather than a screen of its own; the zone list beside it is [`super::zones`] and
//! the two are drawn and driven together.
//!
//! Shown when the game starts without `--server` or `--sandbox`, and again when a
//! connection fails or you leave a game. Tab or Up/Down move between fields,
//! Left/Right change the ship, Enter plays, F2 opens the options, F3 asks the
//! directories again, F5 flies the offline sandbox and Esc quits. The last zone, name,
//! arena and ship (never the password) are remembered in `login.ini` beside the
//! settings.

use std::fmt;
use std::path::Path;

use winit::keyboard::KeyCode;

use super::input::Stroke;
use super::online::ServerOptions;
use oss_session::SPECTATOR;

pub const FILE_NAME: &str = "login.ini";
const MAX_FIELD: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Password,
    Arena,
    Server,
    Ship,
    /// The zone list. It holds no text of its own: while it has the focus, Up and
    /// Down walk the list rather than moving between fields, and whichever zone is
    /// picked fills in [`Field::Server`].
    Zones,
}

/// Down the profile, then the ship, then across to the list, which is the order
/// Continuum's start-up screen reads in.
const FIELDS: [Field; 6] = [
    Field::Name,
    Field::Password,
    Field::Arena,
    Field::Server,
    Field::Ship,
    Field::Zones,
];

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Server => "Zone",
            Field::Name => "Name",
            Field::Password => "Password",
            Field::Arena => "Arena",
            Field::Ship => "Ship",
            Field::Zones => "Zones",
        }
    }
}

/// What the player chose.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Connect(Box<ServerOptions>),
    Sandbox,
    /// Open the settings, which is what Continuum's `options` button does.
    Options,
    /// Ask the directory servers again.
    RefreshZones,
    Quit,
}

/// `Debug` is written out rather than derived so the password cannot be printed.
#[derive(Clone, PartialEq)]
pub struct Login {
    pub server: String,
    pub name: String,
    pub password: String,
    /// Empty = any public arena.
    pub arena: String,
    /// 0..=7, or [`SPECTATOR`].
    pub ship: u8,
    pub focus: usize,
    /// Shown under the form (a failed connection, a missing field).
    pub error: Option<String>,
    /// Everything the command line asked for that this form does not own.
    ///
    /// [`Self::submit`] lays its own five fields over this rather than building a
    /// [`ServerOptions`] from nothing, so an option added to that struct reaches
    /// the zone through this screen without anyone having to remember to carry it.
    /// That is the whole point: `--freq`, `--follow`, `--attach`, `--record` and
    /// `--new-user` were all silently dropped by the version that listed fields by
    /// hand, and a longer list would only have delayed the next one.
    base: Box<ServerOptions>,
}

/// Everything but the password, which prints as `<redacted>`. The `base` behind it
/// carries one too and redacts its own.
impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Login")
            .field("server", &self.server)
            .field("name", &self.name)
            .field("password", &oss_proto::wire::Redacted)
            .field("arena", &self.arena)
            .field("ship", &self.ship)
            .field("focus", &self.focus)
            .field("error", &self.error)
            .field("base", &self.base)
            .finish()
    }
}

impl Default for Login {
    fn default() -> Self {
        Self {
            server: "127.0.0.1:5000".into(),
            name: String::new(),
            password: String::new(),
            arena: String::new(),
            // Spectator, which is what `--ship` documents for an online
            // default, what the `base` options eight lines below already use,
            // and what a real client sends on a first visit: you arrive
            // watching and pick a ship once you are in. This said 0, which is
            // Warbird, so a zone reached through this screen was asked to
            // spawn us straight into a ship.
            ship: SPECTATOR,
            focus: 0,
            error: None,
            base: Box::new(ServerOptions {
                server: String::new(),
                name: String::new(),
                password: String::new(),
                arena: None,
                ship: oss_session::SPECTATOR,
                freq: None,
                follow: None,
                attach: None,
                record: None,
                security: Default::default(),
                known_servers: None,
                identities: None,
                new_user: false,
            }),
        }
    }
}

impl Login {
    /// The form filled from `login.ini` (if any), a name from the settings, and
    /// whatever the command line asked for.
    pub fn load(path: Option<&Path>, name: Option<&str>, base: Box<ServerOptions>) -> Login {
        let mut l = Login {
            base,
            ..Default::default()
        };
        if let Some(text) = path.and_then(|p| std::fs::read(p).ok()) {
            for line in String::from_utf8_lossy(&text).lines() {
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                let v: String = v.trim().chars().take(MAX_FIELD).collect();
                match k.trim() {
                    "server" if !v.is_empty() => l.server = v,
                    "name" => l.name = v,
                    "arena" => l.arena = v,
                    // Same fallback as the default above: an unreadable
                    // `ship` in the file must not quietly mean Warbird.
                    "ship" => l.ship = parse_ship(&v).unwrap_or(SPECTATOR),
                    _ => {}
                }
            }
        }
        if l.name.is_empty()
            && let Some(n) = name
        {
            l.name = n.chars().take(MAX_FIELD).collect();
        }
        // What the command line gave wins over what was remembered. The ship is
        // left alone: the parser cannot tell `--ship spec` from no `--ship` at all,
        // so overlaying it would quietly discard a remembered choice.
        let field = |s: &str| -> Option<String> {
            (!s.is_empty()).then(|| s.chars().take(MAX_FIELD).collect())
        };
        if let Some(v) = field(&l.base.server) {
            l.server = v;
        }
        if let Some(v) = field(&l.base.name) {
            l.name = v;
        }
        if let Some(v) = field(&l.base.password) {
            l.password = v;
        }
        if let Some(v) = l.base.arena.as_deref().and_then(field) {
            l.arena = v;
        }
        // Start where there's something to type.
        l.focus = usize::from(!l.name.is_empty());
        l
    }

    /// Remembers everything but the password.
    pub fn save(&self, path: &Path) {
        let text = format!(
            "server = {}\nname = {}\narena = {}\nship = {}\n",
            self.server,
            self.name,
            self.arena,
            ship_name(self.ship)
        );
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(path, text));
        if let Err(e) = result {
            log::warn!("couldn't save {}: {e}", path.display());
        }
    }

    /// The empty set of command-line options, for a run that had none.
    pub fn into_base(self) -> ServerOptions {
        *self.base
    }

    /// Puts the cursor on the zone list (`--zones`).
    pub fn focus_zones(&mut self) {
        self.focus = FIELDS.len() - 1;
    }

    /// Puts the cursor on a field outright, as a click on it does.
    ///
    /// The pointer and the keyboard reach the same focus model, so clicking a
    /// field leaves the screen in a state tabbing could also have reached --
    /// which is what keeps "the tabbing does work well enough" true after the
    /// pointer arrives.
    pub fn focus_on(&mut self, field: Field) {
        if let Some(i) = FIELDS.iter().position(|f| *f == field) {
            self.focus = i;
        }
    }

    /// Plays, as Enter does: the `play` hot spot's way in.
    ///
    /// Shares [`Login::submit`] rather than repeating it, so the pointer cannot
    /// connect with options the keyboard would have refused, and the field
    /// checks (an address with no port, an empty name) happen either way.
    pub fn play(&mut self) -> Option<Choice> {
        self.submit()
    }

    pub fn focused(&self) -> Field {
        FIELDS[self.focus % FIELDS.len()]
    }

    /// The text shown for `field` (password masked).
    pub fn value(&self, field: Field) -> String {
        match field {
            Field::Server => self.server.clone(),
            Field::Name => self.name.clone(),
            Field::Password => "*".repeat(self.password.chars().count()),
            Field::Arena if self.arena.is_empty() => "(any public arena)".into(),
            Field::Arena => self.arena.clone(),
            Field::Ship => ship_name(self.ship),
            Field::Zones => String::new(),
        }
    }

    pub fn fields() -> &'static [Field] {
        &FIELDS
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self.focused() {
            Field::Server => Some(&mut self.server),
            Field::Name => Some(&mut self.name),
            Field::Password => Some(&mut self.password),
            Field::Arena => Some(&mut self.arena),
            Field::Ship | Field::Zones => None,
        }
    }

    /// Applies one frame of keystrokes.
    ///
    /// One pass in arrival order, because a keystroke is one thing: typing `ab`,
    /// backspacing and typing `c` leaves `ac`, which it cannot if the characters
    /// are all applied before the keys.
    ///
    /// A key held down repeats, and the form takes the repeats for the keys a
    /// person holds on purpose — Backspace to clear a field, Tab and the arrows
    /// to move through it. Enter, Escape and the function keys act once however
    /// long they are held.
    pub fn input(&mut self, strokes: &[Stroke], shift: bool) -> Option<Choice> {
        for stroke in strokes {
            // While the list has the focus it owns Up and Down, so they walk
            // zones instead of moving on to the next field. Read per stroke:
            // the stroke before this one may have moved the focus.
            let on_list = self.focused() == Field::Zones;
            if let Some(c) = stroke.text
                && let Some(t) = self.text_mut()
                && !c.is_control()
                && t.chars().count() < MAX_FIELD
            {
                t.push(c);
            }
            // Keys a person holds on purpose, repeats and all.
            match stroke.key {
                Some(KeyCode::Tab) if !shift => self.focus = (self.focus + 1) % FIELDS.len(),
                Some(KeyCode::Tab) => self.focus = (self.focus + FIELDS.len() - 1) % FIELDS.len(),
                Some(KeyCode::ArrowDown) if !on_list => {
                    self.focus = (self.focus + 1) % FIELDS.len()
                }
                Some(KeyCode::ArrowUp) if !on_list => {
                    self.focus = (self.focus + FIELDS.len() - 1) % FIELDS.len()
                }
                Some(key @ (KeyCode::ArrowLeft | KeyCode::ArrowRight))
                    if self.focused() == Field::Ship =>
                {
                    // Order: 1..8 then spectator.
                    let i = if self.ship == SPECTATOR { 8 } else { self.ship };
                    let i = if key == KeyCode::ArrowRight {
                        (i + 1) % 9
                    } else {
                        (i + 8) % 9
                    };
                    self.ship = if i == 8 { SPECTATOR } else { i };
                }
                Some(KeyCode::Backspace) => {
                    if let Some(t) = self.text_mut() {
                        t.pop();
                    }
                }
                _ => {}
            }
            // Keys that act once, however long they are held.
            match stroke.first_press() {
                Some(KeyCode::Escape) => return Some(Choice::Quit),
                Some(KeyCode::F2) => return Some(Choice::Options),
                Some(KeyCode::F3) => return Some(Choice::RefreshZones),
                Some(KeyCode::F5) => return Some(Choice::Sandbox),
                Some(KeyCode::Enter | KeyCode::NumpadEnter) => return self.submit(),
                _ => {}
            }
        }
        None
    }

    fn submit(&mut self) -> Option<Choice> {
        let server = self.server.trim();
        if !server.contains(':') {
            self.error = Some("Zone must be host:port".into());
            self.focus = 3;
            return None;
        }
        if self.name.trim().is_empty() {
            self.error = Some("Enter a name".into());
            self.focus = 0;
            return None;
        }
        self.error = None;
        // The five fields this form owns, over everything else the command line
        // asked for. The `..` is what stops the next option being dropped.
        Some(Choice::Connect(Box::new(ServerOptions {
            server: server.to_string(),
            name: self.name.trim().to_string(),
            password: self.password.clone(),
            arena: Some(self.arena.trim().to_string()).filter(|a| !a.is_empty()),
            ship: self.ship,
            ..(*self.base).clone()
        })))
    }
}

fn ship_name(ship: u8) -> String {
    if ship == SPECTATOR {
        "spec".into()
    } else {
        (ship + 1).to_string()
    }
}

fn parse_ship(v: &str) -> Option<u8> {
    if v.eq_ignore_ascii_case("spec") {
        return Some(SPECTATOR);
    }
    v.parse::<u8>()
        .ok()
        .filter(|s| (1..=8).contains(s))
        .map(|s| s - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keystrokes a frame of these keys and this text arrives as.
    ///
    /// Text first, then the keys, which is the order the form used to apply them
    /// in when they were two lists — so a test that still passes is one whose
    /// meaning the single list did not change.
    fn strokes(keys: &[KeyCode], typed: &str) -> Vec<Stroke> {
        let mut v: Vec<Stroke> = typed.chars().map(Stroke::text).collect();
        v.extend(keys.iter().copied().map(Stroke::press));
        v
    }

    /// What the lead called "very unfriendly": one character per press, because
    /// the OS repeats were thrown away before anything could read them.
    #[test]
    fn a_held_backspace_clears_the_field() {
        let mut l = Login::load(None, Some("Ace"), Box::new(Login::default().into_base()));
        l.focus = 0;
        assert_eq!(l.name, "Ace");
        // One frame of a key held down: the press, then the OS repeating it.
        let held = [
            Stroke::press(KeyCode::Backspace),
            Stroke::press(KeyCode::Backspace).repeated(true),
            Stroke::press(KeyCode::Backspace).repeated(true),
        ];
        l.input(&held, false);
        assert_eq!(l.name, "", "a held Backspace clears the field");
    }

    /// The other half of one keystroke being one thing: a Backspace lands where
    /// it was typed rather than after everything else in the frame.
    #[test]
    fn a_backspace_lands_between_the_characters_either_side_of_it() {
        let mut l = Login::load(None, None, Box::new(Login::default().into_base()));
        assert_eq!(l.focused(), Field::Name);
        l.input(
            &[
                Stroke::press(KeyCode::KeyA).typing('a'),
                Stroke::press(KeyCode::KeyB).typing('b'),
                Stroke::press(KeyCode::Backspace),
                Stroke::press(KeyCode::KeyC).typing('c'),
            ],
            false,
        );
        assert_eq!(l.name, "ac", "not abc with the end bitten off");
    }

    /// A key that types a character now arrives carrying it, which is what a
    /// scripted `--press` could not do: the form read characters, so every field
    /// ignored it.
    #[test]
    fn a_scripted_key_fills_a_field() {
        let mut l = Login::load(None, None, Box::new(Login::default().into_base()));
        let typed: Vec<Stroke> = "ace"
            .chars()
            .zip([KeyCode::KeyA, KeyCode::KeyC, KeyCode::KeyE])
            .map(|(c, k)| Stroke::press(k).typing(c))
            .collect();
        l.input(&typed, false);
        assert_eq!(l.name, "ace");
        // And Enter still acts once however long it is held.
        l.server = "zone.example:5000".into();
        let held_enter = [
            Stroke::press(KeyCode::Enter),
            Stroke::press(KeyCode::Enter).repeated(true),
        ];
        assert!(matches!(
            l.input(&held_enter, false),
            Some(Choice::Connect(_))
        ));
    }

    /// Every option the command line accepts has to survive the trip through this
    /// screen. It is written as a whole-struct comparison on purpose: the `..base`
    /// means a field added to [`ServerOptions`] is expected to be carried without
    /// anyone editing this test, and the five named fields are exactly the ones the
    /// form is allowed to change. Listing fields by hand is what dropped `--freq`,
    /// `--follow`, `--attach`, `--record` and `--new-user` in the first place, so
    /// this test refuses to do it either.
    #[test]
    fn the_form_carries_every_option_it_does_not_own() {
        use std::path::PathBuf;
        let base = ServerOptions {
            // The fields the form owns. It replaces all five, so their values here
            // only have to differ from what the form will put in their place.
            server: "ignored:1".into(),
            name: "ignored".into(),
            password: "ignored".into(),
            arena: Some("ignored".into()),
            ship: 7,
            // Everything else, each set to something no default would produce.
            freq: Some(42),
            follow: Some("Someone".into()),
            attach: Some("Teammate".into()),
            record: Some(PathBuf::from("visit.ossr")),
            security: oss_session::security::SecurityMode::OpenSec,
            known_servers: Some(PathBuf::from("known.txt")),
            identities: Some(PathBuf::from("ids.txt")),
            new_user: true,
        };

        let mut l = Login::load(None, Some("Ace"), Box::new(base.clone()));
        l.server = "zone.example:5000".into();
        l.name = "Ace".into();
        l.password = "pw".into();
        l.arena = "duel".into();
        l.ship = 2;
        let Some(Choice::Connect(got)) = l.input(&strokes(&[KeyCode::Enter], ""), false) else {
            panic!("expected connect");
        };

        assert_eq!(
            *got,
            ServerOptions {
                server: "zone.example:5000".into(),
                name: "Ace".into(),
                password: "pw".into(),
                arena: Some("duel".into()),
                ship: 2,
                ..base
            }
        );
    }

    #[test]
    fn the_command_line_fills_the_form_in() {
        // What the command line gave beats what was remembered, except the ship,
        // which the parser cannot tell from its own default.
        let base = ServerOptions {
            server: "given:5000".into(),
            name: "Given".into(),
            password: "pw".into(),
            arena: Some("duel".into()),
            ..Login::default().into_base()
        };
        let l = Login::load(None, Some("FromSettings"), Box::new(base));
        assert_eq!(
            (
                l.server.as_str(),
                l.name.as_str(),
                l.password.as_str(),
                l.arena.as_str()
            ),
            ("given:5000", "Given", "pw", "duel")
        );
    }

    /// A derived `Debug` on anything holding a password is a leak waiting for the
    /// first `log::debug!`, and the first one is always added by someone chasing a
    /// different bug. Both of these are printed whenever an `assert_eq!` on them
    /// fails, so the leak had a live path even with no logging at all.
    #[test]
    fn neither_the_form_nor_the_options_can_print_the_password() {
        let mut l = Login::load(None, Some("Ace"), Box::new(Login::default().into_base()));
        l.password = "hunter2-in-the-clear".into();
        let shown = format!("{l:?}");
        assert!(!shown.contains("hunter2"), "the form printed it: {shown}");
        assert!(shown.contains("<redacted>"), "and says so: {shown}");
        // The rest is still readable, which is the point of redacting the field
        // rather than the type.
        assert!(shown.contains("Ace"), "the name is still there: {shown}");

        let opts = ServerOptions {
            password: "hunter2-in-the-clear".into(),
            ..Login::default().into_base()
        };
        let shown = format!("{opts:?}");
        assert!(
            !shown.contains("hunter2"),
            "the options printed it: {shown}"
        );
        assert!(shown.contains("<redacted>"), "and say so: {shown}");
    }

    /// The merged start-up screen walked the way a person walks it: tab down the
    /// profile typing as you go, pick a ship, and press Enter.
    ///
    /// This is not a substitute for somebody actually sitting at it — nobody has —
    /// but the submit path had no coverage of its own at all, and the commit that
    /// merged the three screens changed it. Everything here goes through
    /// `input`, so the focus order, the typing and the Enter are exercised rather
    /// than the fields being set from the outside.
    #[test]
    fn a_person_walking_the_form_arrives_at_the_right_zone() {
        let mut l = Login::load(None, None, Box::new(Login::default().into_base()));
        assert_eq!(l.focused(), Field::Name, "no saved name: start at the name");

        l.input(&strokes(&[], "Ace"), false);
        l.input(&strokes(&[KeyCode::Tab], ""), false);
        assert_eq!(l.focused(), Field::Password);
        l.input(&strokes(&[], "pw"), false);
        l.input(&strokes(&[KeyCode::Tab], ""), false);
        assert_eq!(l.focused(), Field::Arena);
        l.input(&strokes(&[], "duel"), false);
        l.input(&strokes(&[KeyCode::Tab], ""), false);
        assert_eq!(l.focused(), Field::Server);

        // Clear what was remembered and type an address, one key at a time.
        for _ in 0..l.server.chars().count() {
            l.input(&strokes(&[KeyCode::Backspace], ""), false);
        }
        assert_eq!(l.server, "");
        l.input(&strokes(&[], "zone.example:5000"), false);

        l.input(&strokes(&[KeyCode::Tab], ""), false);
        assert_eq!(l.focused(), Field::Ship);
        // From a known ship, so this tests the movement and not the default.
        l.ship = 0;
        l.input(&strokes(&[KeyCode::ArrowRight], ""), false);
        assert_eq!(l.value(Field::Ship), "2", "right moves on a ship");

        // And on to the zone list, where Enter still plays rather than being eaten.
        l.input(&strokes(&[KeyCode::Tab], ""), false);
        assert_eq!(l.focused(), Field::Zones);
        let Some(Choice::Connect(opts)) = l.input(&strokes(&[KeyCode::Enter], ""), false) else {
            panic!("Enter on the form should play");
        };
        assert_eq!(
            (
                opts.server.as_str(),
                opts.name.as_str(),
                opts.password.as_str(),
                opts.arena.as_deref(),
                opts.ship
            ),
            ("zone.example:5000", "Ace", "pw", Some("duel"), 1)
        );
        assert_eq!(l.error, None);
    }

    #[test]
    fn typing_moving_and_connecting() {
        let mut l = Login::load(None, Some("Ace"), Box::new(Login::default().into_base()));
        // A name was remembered, so the cursor starts on the password.
        assert_eq!(l.focused(), Field::Password);
        assert_eq!(l.input(&strokes(&[], "pw"), false), None);
        assert_eq!(l.value(Field::Password), "**");
        l.input(
            &strokes(
                &[KeyCode::ArrowDown, KeyCode::ArrowDown, KeyCode::ArrowDown],
                "",
            ),
            false,
        );
        assert_eq!(l.focused(), Field::Ship);
        l.ship = 0;
        l.input(&strokes(&[KeyCode::ArrowLeft], "x"), false);
        assert_eq!(l.value(Field::Ship), "spec");
        l.input(
            &strokes(&[KeyCode::ArrowRight, KeyCode::ArrowRight], ""),
            false,
        );
        assert_eq!(l.value(Field::Ship), "2");
        // One more step reaches the zone list, which takes no text and gives Up and
        // Down to the list instead of to the form.
        l.input(&strokes(&[KeyCode::Tab], ""), false);
        assert_eq!(l.focused(), Field::Zones);
        l.input(
            &strokes(&[KeyCode::ArrowDown, KeyCode::ArrowUp], "zzz"),
            false,
        );
        assert_eq!(l.focused(), Field::Zones);
        assert_eq!(l.value(Field::Zones), "");
        let Some(Choice::Connect(o)) = l.input(&strokes(&[KeyCode::Enter], ""), false) else {
            panic!("expected connect");
        };
        assert_eq!(
            (o.name.as_str(), o.password.as_str(), o.ship, o.arena),
            ("Ace", "pw", 1, None)
        );
        assert_eq!(
            l.input(&strokes(&[KeyCode::F5], ""), false),
            Some(Choice::Sandbox)
        );
        assert_eq!(
            l.input(&strokes(&[KeyCode::F2], ""), false),
            Some(Choice::Options)
        );
        assert_eq!(
            l.input(&strokes(&[KeyCode::F3], ""), false),
            Some(Choice::RefreshZones)
        );
        assert_eq!(
            l.input(&strokes(&[KeyCode::Escape], ""), false),
            Some(Choice::Quit)
        );
    }

    /// Both ship fallbacks are spectator.
    ///
    /// The form used to default to 0 while the `ServerOptions` it carries
    /// defaulted to `SPECTATOR` eight lines below -- two contradictory
    /// defaults in one function -- and a malformed `ship` in `login.ini` fell
    /// back to 0 as well. 0 is Warbird, so reaching a zone through this
    /// screen asked it to spawn us straight into a ship, while
    /// `--server --name` asked to spectate. A capture of a real client shows
    /// it spectating on a first visit.
    #[test]
    fn an_unset_or_unreadable_ship_means_spectator_not_warbird() {
        let base = || Login::default().base;
        assert_eq!(Login::default().ship, SPECTATOR, "the form's own default");
        assert_eq!(
            Login::default().base.ship,
            SPECTATOR,
            "and the options it carries, which already agreed"
        );

        let dir = std::env::temp_dir().join(format!("oss-login-ship-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let ini = dir.join("login.ini");

        std::fs::write(
            &ini,
            "ship = not-a-ship
",
        )
        .expect("write");
        assert_eq!(
            Login::load(Some(&ini), None, base()).ship,
            SPECTATOR,
            "an unreadable ship must not quietly mean Warbird"
        );

        // A ship the player really did choose still survives a round trip.
        std::fs::write(
            &ini,
            "ship = 3
",
        )
        .expect("write");
        assert_eq!(
            Login::load(Some(&ini), None, base()).ship,
            2,
            "ship 3 is index 2"
        );
        std::fs::write(
            &ini,
            "ship = spec
",
        )
        .expect("write");
        assert_eq!(Login::load(Some(&ini), None, base()).ship, SPECTATOR);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checks_fields_and_remembers_all_but_the_password() {
        let mut l = Login::load(None, None, Box::new(Login::default().into_base()));
        assert_eq!(l.focused(), Field::Name);
        assert_eq!(l.input(&strokes(&[KeyCode::Enter], ""), false), None);
        assert_eq!(l.error.as_deref(), Some("Enter a name"));
        l.input(&strokes(&[], "Bob"), false);
        l.input(&strokes(&[KeyCode::Tab], ""), false);
        l.input(&strokes(&[], "secret"), false);
        l.input(&strokes(&[KeyCode::Tab], ""), false);
        l.input(&strokes(&[], "duel"), false);
        l.input(&strokes(&[KeyCode::Tab, KeyCode::Backspace], ""), false);
        assert_eq!(l.focused(), Field::Server);
        assert_eq!(l.server, "127.0.0.1:500");
        // A zone address with no port is refused, and the cursor goes to the field
        // that is wrong rather than to the top of the form.
        l.input(&strokes(&[], "0"), false);
        l.server = "nowhere".into();
        assert_eq!(l.input(&strokes(&[KeyCode::Enter], ""), false), None);
        assert_eq!(l.error.as_deref(), Some("Zone must be host:port"));
        assert_eq!(l.focused(), Field::Server);
        l.server = "127.0.0.1:5000".into();
        let dir = std::env::temp_dir().join(format!("oss-login-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        l.save(&path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("secret"));
        let back = Login::load(
            Some(&path),
            Some("Other"),
            Box::new(Login::default().into_base()),
        );
        assert_eq!(
            (
                back.server.as_str(),
                back.name.as_str(),
                back.arena.as_str()
            ),
            ("127.0.0.1:5000", "Bob", "duel")
        );
        assert!(back.password.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
