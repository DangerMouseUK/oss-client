//! The login screen: zone address, name, password, arena and ship.
//!
//! Shown when the game starts without `--server` or `--sandbox`, and again when a
//! connection fails. Up/Down or Tab move between fields, Left/Right change the ship,
//! Enter connects, F5 flies the offline sandbox, Esc quits. The last zone, name, arena
//! and ship (never the password) are remembered in `login.ini` beside the settings.

use std::path::Path;

use winit::keyboard::KeyCode;

use super::online::ServerOptions;
use oss_session::SPECTATOR;

pub const FILE_NAME: &str = "login.ini";
const MAX_FIELD: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Server,
    Name,
    Password,
    Arena,
    Ship,
}

const FIELDS: [Field; 5] = [
    Field::Server,
    Field::Name,
    Field::Password,
    Field::Arena,
    Field::Ship,
];

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Server => "Zone",
            Field::Name => "Name",
            Field::Password => "Password",
            Field::Arena => "Arena",
            Field::Ship => "Ship",
        }
    }
}

/// What the player chose.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Connect(ServerOptions),
    Sandbox,
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
        }
    }
}

impl Login {
    /// The form filled from `login.ini` (if any) and a name from the settings.
    pub fn load(path: Option<&Path>, name: Option<&str>) -> Login {
        let mut l = Login::default();
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
        // Start where there's something to type.
        l.focus = if l.name.is_empty() { 1 } else { 2 };
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
            Field::Ship => None,
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
        for key in pressed {
            match key {
                KeyCode::Escape => return Some(Choice::Quit),
                KeyCode::F5 => return Some(Choice::Sandbox),
                KeyCode::Enter | KeyCode::NumpadEnter => return self.submit(),
                KeyCode::ArrowDown => self.focus = (self.focus + 1) % FIELDS.len(),
                KeyCode::Tab if !shift => self.focus = (self.focus + 1) % FIELDS.len(),
                KeyCode::ArrowUp | KeyCode::Tab => {
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
            self.focus = 0;
            return None;
        }
        if self.name.trim().is_empty() {
            self.error = Some("Enter a name".into());
            self.focus = 1;
            return None;
        }
        self.error = None;
        Some(Choice::Connect(ServerOptions {
            server: server.to_string(),
            name: self.name.trim().to_string(),
            password: self.password.clone(),
            arena: Some(self.arena.trim().to_string()).filter(|a| !a.is_empty()),
            ship: self.ship,
            freq: None,
            follow: None,
            attach: None,
        }))
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

    #[test]
    fn typing_moving_and_connecting() {
        let mut l = Login::load(None, Some("Ace"));
        assert_eq!(l.focused(), Field::Password);
        assert_eq!(l.input(&[], "pw", false), None);
        assert_eq!(l.value(Field::Password), "**");
        l.input(&[KeyCode::ArrowDown, KeyCode::ArrowDown], "", false);
        assert_eq!(l.focused(), Field::Ship);
        l.input(&[KeyCode::ArrowLeft], "x", false);
        assert_eq!(l.value(Field::Ship), "spec");
        l.input(&[KeyCode::ArrowRight, KeyCode::ArrowRight], "", false);
        assert_eq!(l.value(Field::Ship), "2");
        let Some(Choice::Connect(o)) = l.input(&[KeyCode::Enter], "", false) else {
            panic!("expected connect");
        };
        assert_eq!(
            (o.name.as_str(), o.password.as_str(), o.ship, o.arena),
            ("Ace", "pw", 1, None)
        );
        assert_eq!(l.input(&[KeyCode::F5], "", false), Some(Choice::Sandbox));
        assert_eq!(l.input(&[KeyCode::Escape], "", false), Some(Choice::Quit));
    }

    #[test]
    fn checks_fields_and_remembers_all_but_the_password() {
        let mut l = Login::load(None, None);
        assert_eq!(l.focused(), Field::Name);
        assert_eq!(l.input(&[KeyCode::Enter], "", false), None);
        assert_eq!(l.error.as_deref(), Some("Enter a name"));
        l.input(&[], "Bob", false);
        l.input(&[KeyCode::Tab], "", false);
        l.input(&[], "secret", false);
        l.input(&[KeyCode::Tab], "", false);
        l.input(&[], "duel", false);
        l.input(&[KeyCode::Tab, KeyCode::Tab, KeyCode::Backspace], "", false);
        assert_eq!(l.focused(), Field::Server);
        l.input(&[], "", false);
        assert_eq!(l.server, "127.0.0.1:500");
        let dir = std::env::temp_dir().join(format!("oss-login-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        l.save(&path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("secret"));
        let back = Login::load(Some(&path), Some("Other"));
        assert_eq!(
            (
                back.server.as_str(),
                back.name.as_str(),
                back.arena.as_str()
            ),
            ("127.0.0.1:500", "Bob", "duel")
        );
        assert!(back.password.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
