//! The settings file: video, audio, accessibility, networking, the player's name,
//! key and gamepad bindings, macro keys and directory servers.
//!
//! A small INI file (`oss-client.ini` in the user's config folder, or `--config`).
//! Missing entries keep their defaults, unknown or bad lines are reported and skipped,
//! and the file is created with every default written out (as a reference) the first
//! time the game runs. Command-line flags override it.
//!
//! ```ini
//! [video]
//! scale = 0          ; 0 = from the display's DPI
//! [accessibility]
//! palette = standard ; or deuteranopia, protanopia, tritanopia, high_contrast
//! [keys]
//! gun = NoShift+ControlLeft, NoShift+ControlRight
//! mine = Shift+Tab
//! [macros]
//! Ctrl+F1 = 'on my way to %coord
//! ```

use std::path::{Path, PathBuf};

use winit::keyboard::KeyCode;

use crate::game::input::{Action, Binding, Bindings, Mods, PadBindings, PadButton};

pub const FILE_NAME: &str = "oss-client.ini";

/// The directory servers a fresh install asks for zones, the one proven to
/// answer first.
///
/// These are the game's own, not ours to choose: the same three names, in the
/// same role, that ASSS ships in its `conf/global.conf` for a zone to register
/// with. That is what makes them the right list -- they are where the zones
/// actually list themselves, so they are where a client has to ask.
///
/// Measured on 2026-11-17 through this client's own directory code:
/// `sscentral.sscuservers.net` answered with 21 zones;
/// `ssdir.playsubspace.com` resolved but did not answer; `zones.sscentral.com`
/// did not resolve at all, its whole domain returning SERVFAIL, having
/// answered on 2026-11-12. The two that are down are kept rather than dropped,
/// because a directory list exists to be redundant and these are known-good
/// addresses having a bad week, not guesses. An unreachable one costs a first
/// run nothing: it is dropped with a note and the rest are merged.
///
/// Port 4990 is the client-facing one; zones register on 4991.
pub const DEFAULT_DIRECTORIES: [&str; 3] = [
    "sscentral.sscuservers.net:4990",
    "ssdir.playsubspace.com:4990",
    "zones.sscentral.com:4990",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Integer pixel scale; `None` picks one from the display.
    pub scale: Option<u32>,
    pub vsync: bool,
    pub fullscreen: bool,
    pub mute: bool,
    /// Master volume, 0-100.
    pub volume: u32,
    /// Default player name online.
    pub name: Option<String>,
    pub bindings: Bindings,
    /// Macro keys: binding and the chat line they send.
    pub macros: Vec<(Binding, String)>,
    /// Accessibility: colour palette, and extra integer scale for the interface.
    pub palette: crate::game::palette::PaletteKind,
    pub ui_scale: u32,
    /// Gamepad buttons per action (the left stick always steers).
    pub pad: PadBindings,
    /// Directory servers for the zone browser ("host:port").
    /// [`DEFAULT_DIRECTORIES`] unless the settings file says otherwise.
    pub directories: Vec<String>,
    /// Connection encryption (auto = OpenSec, falling back to VIE).
    pub security: oss_session::security::SecurityMode,
    /// Offer zones an OpenSec identity key (one per zone) they can use for bans.
    pub identity: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scale: None,
            vsync: false,
            fullscreen: false,
            mute: false,
            volume: 60,
            name: None,
            bindings: Bindings::default(),
            macros: Vec::new(),
            palette: Default::default(),
            ui_scale: 0,
            pad: PadBindings::default(),
            directories: DEFAULT_DIRECTORIES.iter().map(|&s| s.to_owned()).collect(),
            security: Default::default(),
            identity: true,
        }
    }
}

/// The settings folder: `%APPDATA%\OpenSubSpace` on Windows, else
/// `$XDG_CONFIG_HOME/open-subspace` or `~/.config/open-subspace`.
pub fn default_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("OpenSubSpace"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join("open-subspace"))
    }
}

impl Config {
    /// Loads `path`. A missing file at the default location is created with the
    /// defaults; a missing file given explicitly is an error, because a mistyped
    /// `--config` should not quietly mint a second profile somewhere else. Returns
    /// the settings and a warning per line that couldn't be used.
    pub fn load(path: &Path, explicit: bool) -> Result<(Config, Vec<String>), String> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Config::parse(&String::from_utf8_lossy(&bytes))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => {
                let config = Config::default();
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|_| std::fs::write(path, config.to_text()));
                match written {
                    Ok(()) => log::info!("wrote default settings to {}", path.display()),
                    Err(e) => log::warn!("couldn't write {}: {e}", path.display()),
                }
                Ok((config, Vec::new()))
            }
            // Only an explicit path reaches this, since the arm above takes the
            // default one. Saying the rule here is the point: whoever meets it has
            // just typed a path and been handed `os error 2` -- or `os error 3`,
            // which is the folder missing rather than the file.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!(
                "{}: {e}. --config names a settings file to read; only the \
                 default one is written when it is missing. Create it first -- \
                 an empty file is enough.",
                path.display()
            )),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut c = Config::default();
        let mut warnings = Vec::new();
        let mut section = String::new();
        // Whether the file binds `spectate` itself; see [`settle_spectate`].
        let mut spectate_listed = false;
        for (n, raw) in text.lines().enumerate() {
            let whole = raw.trim();
            if whole.is_empty() || whole.starts_with(';') || whole.starts_with('#') {
                continue;
            }
            // Macro lines are chat text, where ';' and '#' mean channels and squads.
            let line = if section == "macros" && !whole.starts_with('[') {
                whole
            } else {
                strip_comment(raw).trim()
            };
            if line.is_empty() {
                continue;
            }
            let mut warn = |msg: String| warnings.push(format!("line {}: {msg}", n + 1));
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_ascii_lowercase();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                warn(format!("expected `name = value`, got `{line}`"));
                continue;
            };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            let result = match (section.as_str(), key.as_str()) {
                ("video", "scale") => number(value, 0, 8).map(|n| c.scale = (n > 0).then_some(n)),
                ("video", "vsync") => flag(value).map(|b| c.vsync = b),
                ("video", "fullscreen") => flag(value).map(|b| c.fullscreen = b),
                ("audio", "mute") => flag(value).map(|b| c.mute = b),
                ("audio", "volume") => number(value, 0, 100).map(|n| c.volume = n),
                ("accessibility", "palette") => crate::game::palette::PaletteKind::from_name(value)
                    .map(|p| c.palette = p)
                    .ok_or_else(|| format!("unknown palette `{value}`")),
                ("accessibility", "ui_scale") => number(value, 0, 4).map(|n| c.ui_scale = n),
                ("network", "identity") => flag(value).map(|b| c.identity = b),
                ("network", "security") => oss_session::security::SecurityMode::from_name(value)
                    .map(|m| c.security = m)
                    .ok_or_else(|| format!("expected auto, opensec or vie, got `{value}`")),
                ("directory", "servers") => {
                    // Every settings file written before this key had defaults
                    // carries `servers =` with nothing after it, because
                    // nothing was the default. Reading that as "no directory
                    // servers" would leave precisely the players who have run
                    // the client before -- the only ones who have a settings
                    // file at all -- still staring at the empty zone browser
                    // these defaults exist to fill. So an empty value means
                    // the built-in list, and a player who genuinely wants no
                    // zone browser says so in a word.
                    let listed: Vec<String> = value
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                    c.directories = match listed.as_slice() {
                        [] => DEFAULT_DIRECTORIES.iter().map(|&s| s.to_owned()).collect(),
                        [one] if one.eq_ignore_ascii_case("none") => Vec::new(),
                        _ => listed,
                    };
                    Ok(())
                }
                ("player", "name") => {
                    c.name = (!value.is_empty()).then(|| value.to_string());
                    // The name is the start-up screen's, remembered in
                    // `login.ini`. This key is only the seed for a run that has
                    // never been through that screen, so a reader who sets it
                    // and sees nothing change is told why rather than left to
                    // work it out. It used to have a box in the options as well,
                    // which made two places setting one thing and only one of
                    // them winning.
                    warn(
                        "player name is set on the start-up screen and remembered \
                         there; this key only seeds the first run"
                            .to_string(),
                    );
                    Ok(())
                }
                ("macros", _) => {
                    // The key keeps its case-insensitive name; the text keeps its case.
                    let key_text = line.split_once('=').map_or("", |(k, _)| k.trim());
                    parse_binding(key_text).map(|b| {
                        if !value.is_empty() {
                            c.macros.push((b, value.to_string()));
                        }
                    })
                }
                ("gamepad", action) => match Action::from_name(action) {
                    Some(a) => value
                        .split(',')
                        .map(str::trim)
                        .filter(|b| !b.is_empty())
                        .map(|b| {
                            PadButton::from_name(b).ok_or_else(|| format!("unknown button `{b}`"))
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map(|buttons| c.pad.set(a, buttons)),
                    None => Err(format!("unknown action `{action}`")),
                },
                ("keys", action) => match Action::from_name(action) {
                    Some(a) => {
                        spectate_listed |= a == Action::Spectate;
                        parse_bindings(value).map(|b| c.bindings.set(a, b))
                    }
                    None => Err(format!("unknown action `{action}`")),
                },
                _ => Err(format!("unknown setting `{key}` in [{section}]")),
            };
            if let Err(e) = result {
                warn(e);
            }
        }
        if !spectate_listed {
            settle_spectate(&mut c.bindings);
        }
        (c, warnings)
    }

    /// The whole file, with comments; parsing it gives back `self`.
    pub fn to_text(&self) -> String {
        let mut s = String::from(
            "; oss-client settings. Delete a line to get its default back.\n\
             ; Command-line flags override these.\n\n[video]\n",
        );
        s += &format!(
            "scale = {}          ; integer pixel scale 1-8, 0 = from the display\n",
            self.scale.unwrap_or(0)
        );
        s += &format!(
            "vsync = {}\nfullscreen = {}\n\n",
            self.vsync, self.fullscreen
        );
        s += &format!(
            "[audio]\nmute = {}\nvolume = {}         ; 0-100\n\n",
            self.mute, self.volume
        );
        s += &format!(
            "[accessibility]\n\
             palette = {}  ; standard, deuteranopia, protanopia, tritanopia, high_contrast\n\
             ui_scale = {}        ; 0-4: draw the interface (text, HUD) this much larger\n\n",
            self.palette.name(),
            self.ui_scale
        );
        s += &format!(
            "[network]\n\
             ; auto: OpenSec, falling back to VIE for servers without it (never once a\n\
             ; server's key is on record); opensec or vie to force one.\n\
             security = {}\n\
             ; Offer each zone an OpenSec identity key of its own (kept in\n\
             ; identities.txt), which zones may use for bans.\n\
             identity = {}\n\n",
            self.security.name(),
            self.identity
        );
        s += &format!(
            "[directory]\n\
             ; Directory servers for the zone browser (F3 on the login screen), as\n\
             ; host:port, comma separated. These are the game's own, and the zones\n\
             ; list themselves on them. One that is unreachable is skipped and the\n\
             ; rest are merged, so a directory having a bad day costs you nothing.\n\
             ; Empty brings this list back; `none` turns the zone browser off.\n\
             servers = {}\n\n",
            if self.directories.is_empty() {
                "none".to_owned()
            } else {
                self.directories.join(", ")
            }
        );
        // No `[player] name`. The name is the start-up screen's and is
        // remembered in `login.ini`; a file that still carries the key is read
        // (and warned about) but it is never written again, because writing a
        // key while telling the reader not to set it is the two-sources fault
        // in another coat.
        s += "[keys]\n\
                ; action = key, key... Keys use their US-layout names (ArrowUp, ControlLeft,\n\
                ; KeyA, Digit1, F3, Tab, Backquote...). Prefixes: Shift+ and Ctrl+ need that\n\
                ; modifier, NoShift+ works only without Shift, and no prefix works either way.\n\
                ; Leave the value empty to unbind an action.\n";
        for a in Action::ALL {
            let keys: Vec<String> = self.bindings.get(a).iter().map(binding_name).collect();
            s += &format!("{} = {}\n", a.name(), keys.join(", "));
        }
        s += "\n[gamepad]\n\
              ; action = buttons: south east west north l1 r1 l2 r2 l3 r3 select start\n\
              ; dpad_up dpad_down dpad_left dpad_right. The left stick always steers.\n";
        for a in Action::ALL {
            let buttons: Vec<&str> = self.pad.get(a).iter().map(|b| b.name()).collect();
            if !buttons.is_empty() {
                s += &format!("{} = {}\n", a.name(), buttons.join(", "));
            }
        }
        s += "\n[macros]\n\
              ; key = chat line, sent when the key is pressed (not while typing). Same keys\n\
              ; and prefixes as [keys], e.g. `Ctrl+F1 = 'need help at base` or\n\
              ; `F9 = ;2;hello channel 2`. Comments here must be on their own line.\n";
        for (b, text) in &self.macros {
            s += &format!("{} = {text}\n", binding_name(b));
        }
        s
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find([';', '#']) {
        Some(i) => &line[..i],
        None => line,
    }
}

fn flag(v: &str) -> Result<bool, String> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(format!("expected true or false, got `{v}`")),
    }
}

fn number(v: &str, min: u32, max: u32) -> Result<u32, String> {
    v.parse::<u32>()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| format!("expected a number {min}-{max}, got `{v}`"))
}

/// F11 moved from a second screenshot key to spectator mode, which is what it
/// is in Continuum. Every settings file written before that lists every action
/// with its keys, so it says `screenshot = PrintScreen, F11` and has no
/// `spectate` line: read as it stands, F11 would both take a screenshot and
/// spectate.
///
/// So for a file with no `spectate` line: a screenshot binding that is exactly
/// the old default loses F11, and if F11 is still bound to anything after that
/// -- the player put it there on purpose -- spectate starts unbound rather than
/// taking the key away from them.
fn settle_spectate(bindings: &mut Bindings) {
    let old_default = [
        Binding {
            key: KeyCode::PrintScreen,
            mods: Mods::Any,
        },
        Binding {
            key: KeyCode::F11,
            mods: Mods::Any,
        },
    ];
    if bindings.get(Action::Screenshot) == old_default {
        bindings.set(Action::Screenshot, old_default[..1].to_vec());
    }
    let f11_taken = Action::ALL
        .into_iter()
        .filter(|&a| a != Action::Spectate)
        .any(|a| bindings.get(a).iter().any(|b| b.key == KeyCode::F11));
    if f11_taken {
        bindings.set(Action::Spectate, Vec::new());
    }
}

fn parse_bindings(v: &str) -> Result<Vec<Binding>, String> {
    v.split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(parse_binding)
        .collect()
}

fn parse_binding(s: &str) -> Result<Binding, String> {
    let (mods, key) = match s.split_once('+') {
        Some((m, k)) => {
            let mods = match m.trim().to_ascii_lowercase().as_str() {
                "shift" => Mods::Shift,
                "ctrl" => Mods::Ctrl,
                "noshift" => Mods::None,
                _ => return Err(format!("unknown modifier `{m}` in `{s}`")),
            };
            (mods, k.trim())
        }
        None => (Mods::Any, s),
    };
    let key = key_from_name(key).ok_or_else(|| format!("unknown key `{key}`"))?;
    Ok(Binding { key, mods })
}

fn binding_name(b: &Binding) -> String {
    let prefix = match b.mods {
        Mods::Any => "",
        Mods::None => "NoShift+",
        Mods::Shift => "Shift+",
        Mods::Ctrl => "Ctrl+",
    };
    format!("{prefix}{:?}", b.key)
}

pub(crate) fn key_from_name(name: &str) -> Option<KeyCode> {
    KEYS.iter()
        .copied()
        .find(|k| format!("{k:?}").eq_ignore_ascii_case(name))
}

/// Keys that can be bound, by their winit names.
const KEYS: &[KeyCode] = {
    use KeyCode::*;
    &[
        KeyA,
        KeyB,
        KeyC,
        KeyD,
        KeyE,
        KeyF,
        KeyG,
        KeyH,
        KeyI,
        KeyJ,
        KeyK,
        KeyL,
        KeyM,
        KeyN,
        KeyO,
        KeyP,
        KeyQ,
        KeyR,
        KeyS,
        KeyT,
        KeyU,
        KeyV,
        KeyW,
        KeyX,
        KeyY,
        KeyZ,
        Digit0,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        ShiftLeft,
        ShiftRight,
        ControlLeft,
        ControlRight,
        AltLeft,
        AltRight,
        SuperLeft,
        SuperRight,
        Space,
        Enter,
        Tab,
        Escape,
        Backspace,
        Insert,
        Delete,
        Home,
        End,
        PageUp,
        PageDown,
        CapsLock,
        Backquote,
        Minus,
        Equal,
        BracketLeft,
        BracketRight,
        Backslash,
        Semicolon,
        Quote,
        Comma,
        Period,
        Slash,
        Numpad0,
        Numpad1,
        Numpad2,
        Numpad3,
        Numpad4,
        Numpad5,
        Numpad6,
        Numpad7,
        Numpad8,
        Numpad9,
        NumpadAdd,
        NumpadSubtract,
        NumpadMultiply,
        NumpadDivide,
        NumpadDecimal,
        NumpadEnter,
        NumLock,
        ScrollLock,
        Pause,
        PrintScreen,
        ContextMenu,
    ]
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_round_trips() {
        let c = Config::default();
        let (back, warnings) = Config::parse(&c.to_text());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back, c);
        let custom = Config {
            scale: Some(3),
            vsync: true,
            mute: true,
            volume: 25,
            palette: crate::game::palette::PaletteKind::Tritanopia,
            ui_scale: 2,
            pad: {
                let mut p = PadBindings::default();
                p.set(Action::Gun, vec![PadButton::R2, PadButton::South]);
                p
            },
            directories: vec!["127.0.0.1:4990".into(), "localhost:4991".into()],
            security: oss_session::security::SecurityMode::OpenSec,
            identity: false,
            ..Config::default()
        };
        assert_eq!(Config::parse(&custom.to_text()).0, custom);
    }

    /// A player who has just installed the client must have somewhere to play
    /// without editing a file first. The browser had nothing to ask, so it
    /// told them to go and add a directory server by hand before they could
    /// see a single zone.
    #[test]
    fn a_fresh_install_has_directory_servers() {
        let c = Config::default();
        assert_eq!(c.directories, DEFAULT_DIRECTORIES, "out of the box");
        assert!(
            c.to_text().contains("sscentral.sscuservers.net:4990"),
            "and the file written on the first run says so"
        );
    }

    /// Every settings file written before this key had defaults carries
    /// `servers =` with nothing after it. Those players -- the only ones with
    /// a settings file at all -- are exactly the ones stuck with an empty zone
    /// browser, so an empty value has to mean the built-in list rather than
    /// "the player chose none".
    #[test]
    fn an_old_empty_list_picks_the_defaults_up_and_none_opts_out() {
        let (old, warnings) = Config::parse("[directory]\nservers =\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            old.directories, DEFAULT_DIRECTORIES,
            "an old file catches up"
        );

        let (off, warnings) = Config::parse("[directory]\nservers = none\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(off.directories.is_empty(), "and `none` is how you opt out");

        // ... which has to survive being written back out, or the opt-out
        // would last exactly until the next save.
        assert!(off.to_text().contains("servers = none"));
        assert!(Config::parse(&off.to_text()).0.directories.is_empty());

        let (mine, _) = Config::parse("[directory]\nservers = dir.example:4990\n");
        assert_eq!(mine.directories, ["dir.example:4990"], "a choice is kept");
    }

    /// The player name is read for an old file and never written back: its home
    /// is the start-up screen, which remembers it in `login.ini`. Two places
    /// setting one thing is the fault this removed, and a file that is still
    /// written with the key would be that fault in another coat.
    #[test]
    fn the_player_name_is_read_once_and_never_written() {
        let (c, warnings) = Config::parse("[player]\nname = Pilot One\n");
        assert_eq!(
            c.name.as_deref(),
            Some("Pilot One"),
            "an old file still loads"
        );
        assert_eq!(warnings.len(), 1, "and is told where the name really lives");
        assert!(
            !c.to_text().contains("[player]"),
            "but the key is not written again"
        );
        let (back, warnings) = Config::parse(&c.to_text());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.name, None, "so the seed is spent, not kept in sync");
    }

    #[test]
    fn rebinds_unbinds_and_reports_bad_lines() {
        let text = "\
[video]
scale = 2
fullscreen = maybe
[keys]
gun = Space, ctrl+KeyF   ; two keys
mine =
thor = Hyper+F6
warpdrive = F9
[sound]
loud = yes
";
        let (c, warnings) = Config::parse(text);
        assert_eq!(c.scale, Some(2));
        assert!(!c.fullscreen);
        assert_eq!(
            c.bindings.get(Action::Gun),
            &[
                Binding {
                    key: KeyCode::Space,
                    mods: Mods::Any
                },
                Binding {
                    key: KeyCode::KeyF,
                    mods: Mods::Ctrl
                }
            ]
        );
        assert!(c.bindings.get(Action::Mine).is_empty());
        // The bad thor line leaves its default alone.
        assert_eq!(
            c.bindings.get(Action::Thor),
            Bindings::default().get(Action::Thor)
        );
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings[0].starts_with("line 3:"));
    }

    /// A settings file from before F11 became spectator mode keeps working:
    /// F11 spectates and no longer also takes a screenshot. And a player who
    /// bound F11 to something themselves keeps it.
    #[test]
    fn an_old_settings_file_hands_f11_to_spectator_mode() {
        let f11 = |mods| Binding {
            key: KeyCode::F11,
            mods,
        };
        let print = Binding {
            key: KeyCode::PrintScreen,
            mods: Mods::Any,
        };
        let (c, _) = Config::parse(
            "[keys]
screenshot = PrintScreen, F11
",
        );
        assert_eq!(c.bindings.get(Action::Screenshot), &[print]);
        assert_eq!(c.bindings.get(Action::Spectate), &[f11(Mods::Any)]);

        // Their own use of F11 wins, and spectate waits to be given a key.
        let (c, _) = Config::parse(
            "[keys]
thor = F11
",
        );
        assert_eq!(c.bindings.get(Action::Thor), &[f11(Mods::Any)]);
        assert!(c.bindings.get(Action::Spectate).is_empty());

        // A file that names spectate is taken at its word.
        let (c, _) = Config::parse(
            "[keys]
screenshot = PrintScreen, F11
spectate = F10
",
        );
        assert_eq!(c.bindings.get(Action::Screenshot), &[print, f11(Mods::Any)]);

        // And a fresh file round-trips with spectate on F11 alone.
        let (back, w) = Config::parse(&Config::default().to_text());
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back.bindings.get(Action::Spectate), &[f11(Mods::Any)]);
        assert_eq!(back.bindings.get(Action::Screenshot), &[print]);
    }

    #[test]
    fn macros_keep_semicolons_and_round_trip() {
        let text = "[macros]\n; a comment line\nCtrl+F1 = 'need help ; now\nF9 = ;2;hi #squad\nbadkey = x\n";
        let (c, warnings) = Config::parse(text);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(
            c.macros,
            vec![
                (
                    Binding {
                        key: KeyCode::F1,
                        mods: Mods::Ctrl
                    },
                    "'need help ; now".to_string()
                ),
                (
                    Binding {
                        key: KeyCode::F9,
                        mods: Mods::Any
                    },
                    ";2;hi #squad".to_string()
                ),
            ]
        );
        assert_eq!(Config::parse(&c.to_text()).0.macros, c.macros);
    }

    #[test]
    fn creates_the_default_file_once() {
        let dir = std::env::temp_dir().join(format!("oss-config-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Config::load(&path, true).is_err());
        let (c, _) = Config::load(&path, false).unwrap();
        assert_eq!(c, Config::default());
        std::fs::write(&path, "[audio]\nmute = yes\n").unwrap();
        assert!(Config::load(&path, false).unwrap().0.mute);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The rule itself is pinned by the test above. This pins the *explanation*:
    /// the OS reports a missing explicit file as `os error 2` (or `os error 3`
    /// when the folder is what is missing), which states no rule at all, and
    /// having to rediscover the rule from that is why this came back as an item.
    #[test]
    fn a_missing_explicit_file_says_what_to_do_about_it() {
        let dir = std::env::temp_dir().join(format!("oss-config-say-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let _ = std::fs::remove_dir_all(&dir);
        let e = Config::load(&path, true).unwrap_err();
        assert!(e.contains("--config"), "{e}");
        assert!(e.contains("empty file"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
