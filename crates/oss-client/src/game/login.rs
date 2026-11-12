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

use std::path::Path;

use winit::keyboard::KeyCode;

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

#[derive(Debug, Clone, PartialEq)]
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

impl Default for Login {
    fn default() -> Self {
        Self {
            server: "127.0.0.1:5000".into(),
            name: String::new(),
            password: String::new(),
            arena: String::new(),
            ship: 0,
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
                    "ship" => l.ship = parse_ship(&v).unwrap_or(0),
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

    /// Applies one frame of keys and typed text.
    pub fn input(&mut self, pressed: &[KeyCode], typed: &str, shift: bool) -> Option<Choice> {
        if let Some(t) = self.text_mut() {
            for c in typed.chars() {
                if !c.is_control() && t.chars().count() < MAX_FIELD {
                    t.push(c);
                }
            }
        }
        // While the list has the focus it owns Up and Down, so they walk zones
        // instead of moving on to the next field.
        let on_list = self.focused() == Field::Zones;
        for key in pressed {
            match key {
                KeyCode::Escape => return Some(Choice::Quit),
                KeyCode::F2 => return Some(Choice::Options),
                KeyCode::F3 => return Some(Choice::RefreshZones),
                KeyCode::F5 => return Some(Choice::Sandbox),
                KeyCode::Enter | KeyCode::NumpadEnter => return self.submit(),
                KeyCode::Tab if !shift => self.focus = (self.focus + 1) % FIELDS.len(),
                KeyCode::Tab => self.focus = (self.focus + FIELDS.len() - 1) % FIELDS.len(),
                KeyCode::ArrowDown if !on_list => self.focus = (self.focus + 1) % FIELDS.len(),
                KeyCode::ArrowUp if !on_list => {
                    self.focus = (self.focus + FIELDS.len() - 1) % FIELDS.len()
                }
                KeyCode::ArrowLeft | KeyCode::ArrowRight if self.focused() == Field::Ship => {
                    // Order: 1..8 then spectator.
                    let i = if self.ship == SPECTATOR { 8 } else { self.ship };
                    let i = if *key == KeyCode::ArrowRight {
                        (i + 1) % 9
                    } else {
                        (i + 8) % 9
                    };
                    self.ship = if i == 8 { SPECTATOR } else { i };
                }
                KeyCode::Backspace => {
                    if let Some(t) = self.text_mut() {
                        t.pop();
                    }
                }
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
        let Some(Choice::Connect(got)) = l.input(&[KeyCode::Enter], "", false) else {
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

    #[test]
    fn typing_moving_and_connecting() {
        let mut l = Login::load(None, Some("Ace"), Box::new(Login::default().into_base()));
        // A name was remembered, so the cursor starts on the password.
        assert_eq!(l.focused(), Field::Password);
        assert_eq!(l.input(&[], "pw", false), None);
        assert_eq!(l.value(Field::Password), "**");
        l.input(
            &[KeyCode::ArrowDown, KeyCode::ArrowDown, KeyCode::ArrowDown],
            "",
            false,
        );
        assert_eq!(l.focused(), Field::Ship);
        l.input(&[KeyCode::ArrowLeft], "x", false);
        assert_eq!(l.value(Field::Ship), "spec");
        l.input(&[KeyCode::ArrowRight, KeyCode::ArrowRight], "", false);
        assert_eq!(l.value(Field::Ship), "2");
        // One more step reaches the zone list, which takes no text and gives Up and
        // Down to the list instead of to the form.
        l.input(&[KeyCode::Tab], "", false);
        assert_eq!(l.focused(), Field::Zones);
        l.input(&[KeyCode::ArrowDown, KeyCode::ArrowUp], "zzz", false);
        assert_eq!(l.focused(), Field::Zones);
        assert_eq!(l.value(Field::Zones), "");
        let Some(Choice::Connect(o)) = l.input(&[KeyCode::Enter], "", false) else {
            panic!("expected connect");
        };
        assert_eq!(
            (o.name.as_str(), o.password.as_str(), o.ship, o.arena),
            ("Ace", "pw", 1, None)
        );
        assert_eq!(l.input(&[KeyCode::F5], "", false), Some(Choice::Sandbox));
        assert_eq!(l.input(&[KeyCode::F2], "", false), Some(Choice::Options));
        assert_eq!(
            l.input(&[KeyCode::F3], "", false),
            Some(Choice::RefreshZones)
        );
        assert_eq!(l.input(&[KeyCode::Escape], "", false), Some(Choice::Quit));
    }

    #[test]
    fn checks_fields_and_remembers_all_but_the_password() {
        let mut l = Login::load(None, None, Box::new(Login::default().into_base()));
        assert_eq!(l.focused(), Field::Name);
        assert_eq!(l.input(&[KeyCode::Enter], "", false), None);
        assert_eq!(l.error.as_deref(), Some("Enter a name"));
        l.input(&[], "Bob", false);
        l.input(&[KeyCode::Tab], "", false);
        l.input(&[], "secret", false);
        l.input(&[KeyCode::Tab], "", false);
        l.input(&[], "duel", false);
        l.input(&[KeyCode::Tab, KeyCode::Backspace], "", false);
        assert_eq!(l.focused(), Field::Server);
        assert_eq!(l.server, "127.0.0.1:500");
        // A zone address with no port is refused, and the cursor goes to the field
        // that is wrong rather than to the top of the form.
        l.input(&[], "0", false);
        l.server = "nowhere".into();
        assert_eq!(l.input(&[KeyCode::Enter], "", false), None);
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
