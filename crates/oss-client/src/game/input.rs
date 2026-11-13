//! Keyboard and gamepad input mapped to game actions through a binding table.
//!
//! The default table is Continuum's: arrows fly, Shift afterburner, Ctrl guns, Tab
//! bombs, Shift+Tab mines, Shift+Ctrl repel, Delete multifire, Shift+Delete burst,
//! Home/Shift+Home stealth/cloak, End/Shift+End X-radar/antiwarp, Insert warp,
//! Shift+Insert portal, F3 rocket, F4 brick, F5 decoy, F6 thor.

use std::collections::{HashMap, HashSet};

use oss_sim::ShipInput;
use winit::keyboard::KeyCode;

/// Analog stick dead zone.
const DEAD_ZONE: f32 = 0.35;

/// Everything a key can be bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Forward,
    Backward,
    Left,
    Right,
    Afterburner,
    Gun,
    Bomb,
    Mine,
    Repel,
    Burst,
    Thor,
    Decoy,
    Rocket,
    Brick,
    Portal,
    Warp,
    Multifire,
    Stealth,
    Cloak,
    XRadar,
    Antiwarp,
    /// Escape: menu.
    Menu,
    /// Cycle the stat box.
    StatBox,
    StatBoxUp,
    StatBoxDown,
    /// Hold to show the whole-map radar.
    BigRadar,
    /// Show the frame-rate line.
    ShowFps,
    ZoomIn,
    ZoomOut,
    /// Attach to the player selected in the stat box (or detach).
    Attach,
    /// Shake off ships attached to you.
    DetachTurrets,
    /// Save a screenshot.
    Screenshot,
}

impl Action {
    pub const ALL: [Action; 32] = [
        Action::Forward,
        Action::Backward,
        Action::Left,
        Action::Right,
        Action::Afterburner,
        Action::Gun,
        Action::Bomb,
        Action::Mine,
        Action::Repel,
        Action::Burst,
        Action::Thor,
        Action::Decoy,
        Action::Rocket,
        Action::Brick,
        Action::Portal,
        Action::Warp,
        Action::Multifire,
        Action::Stealth,
        Action::Cloak,
        Action::XRadar,
        Action::Antiwarp,
        Action::Menu,
        Action::StatBox,
        Action::StatBoxUp,
        Action::StatBoxDown,
        Action::BigRadar,
        Action::ShowFps,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::Attach,
        Action::DetachTurrets,
        Action::Screenshot,
    ];

    /// Name used in the config file.
    pub fn name(self) -> &'static str {
        match self {
            Action::Forward => "forward",
            Action::Backward => "backward",
            Action::Left => "left",
            Action::Right => "right",
            Action::Afterburner => "afterburner",
            Action::Gun => "gun",
            Action::Bomb => "bomb",
            Action::Mine => "mine",
            Action::Repel => "repel",
            Action::Burst => "burst",
            Action::Thor => "thor",
            Action::Decoy => "decoy",
            Action::Rocket => "rocket",
            Action::Brick => "brick",
            Action::Portal => "portal",
            Action::Warp => "warp",
            Action::Multifire => "multifire",
            Action::Stealth => "stealth",
            Action::Cloak => "cloak",
            Action::XRadar => "xradar",
            Action::Antiwarp => "antiwarp",
            Action::Menu => "menu",
            Action::StatBox => "statbox",
            Action::StatBoxUp => "statbox_up",
            Action::StatBoxDown => "statbox_down",
            Action::BigRadar => "big_radar",
            Action::ShowFps => "show_fps",
            Action::ZoomIn => "zoom_in",
            Action::ZoomOut => "zoom_out",
            Action::Attach => "attach",
            Action::DetachTurrets => "detach_turrets",
            Action::Screenshot => "screenshot",
        }
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Self::ALL
            .into_iter()
            .find(|a| a.name().eq_ignore_ascii_case(name))
    }
}

/// Which modifier state a binding needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mods {
    /// Works with or without modifiers (movement, afterburner).
    Any,
    /// Only without Shift or Ctrl.
    None,
    Shift,
    Ctrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Binding {
    pub key: KeyCode,
    pub mods: Mods,
}

/// Action → keys table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings {
    map: HashMap<Action, Vec<Binding>>,
}

impl Default for Bindings {
    fn default() -> Self {
        use KeyCode as K;
        use Mods::*;
        let table: &[(Action, KeyCode, Mods)] = &[
            (Action::Forward, K::ArrowUp, Any),
            (Action::Backward, K::ArrowDown, Any),
            (Action::Left, K::ArrowLeft, Any),
            (Action::Right, K::ArrowRight, Any),
            (Action::Afterburner, K::ShiftLeft, Any),
            (Action::Afterburner, K::ShiftRight, Any),
            (Action::Gun, K::ControlLeft, None),
            (Action::Gun, K::ControlRight, None),
            (Action::Repel, K::ControlLeft, Shift),
            (Action::Repel, K::ControlRight, Shift),
            (Action::Repel, K::Backquote, Any),
            (Action::Bomb, K::Tab, None),
            (Action::Mine, K::Tab, Shift),
            (Action::Multifire, K::Delete, None),
            (Action::Burst, K::Delete, Shift),
            (Action::Stealth, K::Home, None),
            (Action::Cloak, K::Home, Shift),
            (Action::XRadar, K::End, None),
            (Action::Antiwarp, K::End, Shift),
            (Action::Warp, K::Insert, None),
            (Action::Portal, K::Insert, Shift),
            (Action::Rocket, K::F3, Any),
            (Action::Brick, K::F4, Any),
            (Action::Decoy, K::F5, Any),
            (Action::Thor, K::F6, Any),
            (Action::Menu, K::Escape, Any),
            (Action::StatBox, K::F2, Any),
            (Action::StatBoxUp, K::PageUp, Any),
            (Action::StatBoxDown, K::PageDown, Any),
            (Action::BigRadar, K::AltLeft, Any),
            (Action::BigRadar, K::AltRight, Any),
            (Action::ShowFps, K::F12, Any),
            (Action::ZoomIn, K::Equal, Ctrl),
            (Action::ZoomIn, K::NumpadAdd, Ctrl),
            (Action::ZoomOut, K::Minus, Ctrl),
            (Action::ZoomOut, K::NumpadSubtract, Ctrl),
            (Action::Attach, K::F7, None),
            (Action::DetachTurrets, K::F7, Ctrl),
            (Action::Screenshot, K::PrintScreen, Any),
            (Action::Screenshot, K::F11, Any),
        ];
        let mut b = Bindings {
            map: HashMap::new(),
        };
        for &(a, key, mods) in table {
            b.add(a, Binding { key, mods });
        }
        b
    }
}

impl Bindings {
    #[allow(dead_code)] // read by the settings file loader
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn add(&mut self, action: Action, binding: Binding) {
        let list = self.map.entry(action).or_default();
        if !list.contains(&binding) {
            list.push(binding);
        }
    }

    /// Replaces all bindings for `action`.
    pub fn set(&mut self, action: Action, bindings: Vec<Binding>) {
        self.map.insert(action, bindings);
    }

    pub fn get(&self, action: Action) -> &[Binding] {
        self.map.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }

    /// What to show a player for `action`, read from this table rather than
    /// written down beside it.
    ///
    /// Continuum's help works this way: its `Commands.hlp` carries `%KB_BULLET`
    /// where the key goes and the client fills it in, so the page shows the keys
    /// you actually have. Alternatives that read the same — the two Shifts, the two
    /// Ctrls — collapse to one, and at most two are shown so a line stays a line.
    pub fn label(&self, action: Action) -> String {
        let mut shown: Vec<String> = Vec::new();
        for b in self.get(action) {
            let name = binding_name(*b);
            if !shown.contains(&name) {
                shown.push(name);
            }
        }
        // A numpad key is a convenience rather than news, so it only speaks for an
        // action that has nothing else bound to it.
        if shown.iter().any(|n| !n.contains("Numpad")) {
            shown.retain(|n| !n.contains("Numpad"));
        }
        shown.truncate(2);
        if shown.is_empty() {
            "(unbound)".to_string()
        } else {
            shown.join(" or ")
        }
    }
}

/// A key's name as a player would write it: `Up`, `Ctrl`, `PgUp`, `` ` ``, `F3`.
///
/// `KeyCode`'s own `Debug` is close enough for most of them — `Home`, `Delete`,
/// `F7` — so the families that read badly are named here and the rest fall through
/// to it with their prefix taken off.
pub fn key_name(key: KeyCode) -> String {
    use KeyCode as K;
    let named = match key {
        K::ArrowUp => "Up",
        K::ArrowDown => "Down",
        K::ArrowLeft => "Left",
        K::ArrowRight => "Right",
        K::ShiftLeft | K::ShiftRight => "Shift",
        K::ControlLeft | K::ControlRight => "Ctrl",
        K::AltLeft | K::AltRight => "Alt",
        K::Escape => "Esc",
        K::PageUp => "PgUp",
        K::PageDown => "PgDn",
        K::PrintScreen => "Print Screen",
        K::Backquote => "`",
        K::Minus => "-",
        K::Equal => "=",
        K::BracketLeft => "[",
        K::BracketRight => "]",
        K::Backslash => "\\",
        K::Semicolon => ";",
        K::Quote => "'",
        K::Comma => ",",
        K::Period => ".",
        K::Slash => "/",
        K::NumpadAdd => "Numpad +",
        K::NumpadSubtract => "Numpad -",
        K::NumpadMultiply => "Numpad *",
        K::NumpadDivide => "Numpad /",
        K::NumpadEnter => "Numpad Enter",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    let debug = format!("{key:?}");
    // `KeyA` is `A`, `Digit1` is `1`, `Numpad3` is `Numpad 3`; `F3` and `Home`
    // already read correctly.
    if let Some(rest) = debug.strip_prefix("Key") {
        return rest.to_string();
    }
    if let Some(rest) = debug.strip_prefix("Digit") {
        return rest.to_string();
    }
    if let Some(rest) = debug.strip_prefix("Numpad") {
        return format!("Numpad {rest}");
    }
    debug
}

/// How one binding reads: the key, with the modifier it needs in front of it.
pub fn binding_name(b: Binding) -> String {
    let prefix = match b.mods {
        Mods::Shift => "Shift+",
        Mods::Ctrl => "Ctrl+",
        // `None` means "without Shift", which is the bare key as far as a player
        // is concerned, and `Any` means the modifier does not matter.
        Mods::None | Mods::Any => "",
    };
    format!("{prefix}{}", key_name(b.key))
}

impl Mods {
    /// Whether this modifier requirement holds for `key` with Shift/Ctrl as given.
    pub fn matches(self, shift: bool, ctrl: bool, key: KeyCode) -> bool {
        mods_match(self, shift, ctrl, key)
    }
}

/// Whether a key puts a character on the chat line when pressed on its own.
///
/// SubSpace and Continuum have no chat mode: a key like `` ` `` (repel) is both a
/// game key and a character, and which one it is depends on whether the chat line
/// already has something on it. This is the test for "could be a character",
/// decided by the key's place on the keyboard rather than by the layout, so it
/// holds wherever the character itself differs.
pub fn types_text(key: KeyCode) -> bool {
    use KeyCode as K;
    matches!(
        key,
        K::Backquote
            | K::Minus
            | K::Equal
            | K::BracketLeft
            | K::BracketRight
            | K::Backslash
            | K::Semicolon
            | K::Quote
            | K::Comma
            | K::Period
            | K::Slash
            | K::Space
            | K::IntlBackslash
            | K::IntlRo
            | K::IntlYen
            | K::KeyA
            | K::KeyB
            | K::KeyC
            | K::KeyD
            | K::KeyE
            | K::KeyF
            | K::KeyG
            | K::KeyH
            | K::KeyI
            | K::KeyJ
            | K::KeyK
            | K::KeyL
            | K::KeyM
            | K::KeyN
            | K::KeyO
            | K::KeyP
            | K::KeyQ
            | K::KeyR
            | K::KeyS
            | K::KeyT
            | K::KeyU
            | K::KeyV
            | K::KeyW
            | K::KeyX
            | K::KeyY
            | K::KeyZ
            | K::Digit0
            | K::Digit1
            | K::Digit2
            | K::Digit3
            | K::Digit4
            | K::Digit5
            | K::Digit6
            | K::Digit7
            | K::Digit8
            | K::Digit9
            | K::Numpad0
            | K::Numpad1
            | K::Numpad2
            | K::Numpad3
            | K::Numpad4
            | K::Numpad5
            | K::Numpad6
            | K::Numpad7
            | K::Numpad8
            | K::Numpad9
            | K::NumpadAdd
            | K::NumpadSubtract
            | K::NumpadMultiply
            | K::NumpadDivide
    )
}

/// The character a key types on its own, where it types one.
///
/// A window keyboard event carries the character with it, so nothing in the
/// running client needs this. A scripted run has only a key name, and a key
/// without its character drives nothing that reads text — which is every menu,
/// every form field and the chat line. US QWERTY unshifted, which is the layout
/// the key names themselves assume (`Digit1` is `1`, `Semicolon` is `;`).
pub fn char_for(key: KeyCode) -> Option<char> {
    use KeyCode as K;
    if !types_text(key) {
        return None;
    }
    let ch = match key {
        K::Space => ' ',
        K::Numpad0 => '0',
        K::Numpad1 => '1',
        K::Numpad2 => '2',
        K::Numpad3 => '3',
        K::Numpad4 => '4',
        K::Numpad5 => '5',
        K::Numpad6 => '6',
        K::Numpad7 => '7',
        K::Numpad8 => '8',
        K::Numpad9 => '9',
        K::NumpadAdd => '+',
        K::NumpadSubtract => '-',
        K::NumpadMultiply => '*',
        K::NumpadDivide => '/',
        // The rest are named by what they type: `KeyA` reads `A`, `Digit1`
        // reads `1`, and the punctuation keys are named by their character
        // already. Taking it from there keeps one table rather than two that
        // can drift. The international keys name themselves instead, and have
        // no character that holds across layouts, so they fall out here.
        other => {
            let name = key_name(other);
            let mut chars = name.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            ch
        }
    };
    Some(ch.to_ascii_lowercase())
}

/// The character a key types with Shift held, where it types one.
///
/// The other half of [`char_for`], US QWERTY for the same reason, and needed for
/// the same one: a scripted run cannot type a zone address without it, because
/// the colon in `host:port` is Shift and the semicolon key.
pub fn shifted_char(key: KeyCode) -> Option<char> {
    use KeyCode as K;
    let ch = match key {
        K::Backquote => '~',
        K::Minus => '_',
        K::Equal => '+',
        K::BracketLeft => '{',
        K::BracketRight => '}',
        K::Backslash => '|',
        K::Semicolon => ':',
        K::Quote => '"',
        K::Comma => '<',
        K::Period => '>',
        K::Slash => '?',
        K::Digit1 => '!',
        K::Digit2 => '@',
        K::Digit3 => '#',
        K::Digit4 => '$',
        K::Digit5 => '%',
        K::Digit6 => '^',
        K::Digit7 => '&',
        K::Digit8 => '*',
        K::Digit9 => '(',
        K::Digit0 => ')',
        // A letter is its capital, and the numpad and the space bar type the
        // same character either way.
        other => return char_for(other).map(|c| c.to_ascii_uppercase()),
    };
    Some(ch)
}

/// Whether a binding loses to the chat line while something is being typed.
/// A Ctrl binding does not: Ctrl and a key put no character on the line.
pub fn yields_to_chat(b: Binding) -> bool {
    b.mods != Mods::Ctrl && types_text(b.key)
}

fn mods_match(mods: Mods, shift: bool, ctrl: bool, key: KeyCode) -> bool {
    // A modifier key never counts as its own modifier (Ctrl alone is not "Ctrl+Ctrl").
    let is_shift = matches!(key, KeyCode::ShiftLeft | KeyCode::ShiftRight);
    let is_ctrl = matches!(key, KeyCode::ControlLeft | KeyCode::ControlRight);
    let shift = shift && !is_shift;
    let ctrl = ctrl && !is_ctrl;
    match mods {
        Mods::Any => true,
        Mods::None => !shift,
        Mods::Shift => shift,
        Mods::Ctrl => ctrl,
    }
}

/// Gamepad buttons, by their position on a standard pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PadButton {
    South,
    East,
    West,
    North,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    Select,
    Start,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

impl PadButton {
    pub const ALL: [PadButton; 16] = [
        PadButton::South,
        PadButton::East,
        PadButton::West,
        PadButton::North,
        PadButton::L1,
        PadButton::R1,
        PadButton::L2,
        PadButton::R2,
        PadButton::L3,
        PadButton::R3,
        PadButton::Select,
        PadButton::Start,
        PadButton::DPadUp,
        PadButton::DPadDown,
        PadButton::DPadLeft,
        PadButton::DPadRight,
    ];

    /// Name in the settings file.
    pub fn name(self) -> &'static str {
        match self {
            PadButton::South => "south",
            PadButton::East => "east",
            PadButton::West => "west",
            PadButton::North => "north",
            PadButton::L1 => "l1",
            PadButton::R1 => "r1",
            PadButton::L2 => "l2",
            PadButton::R2 => "r2",
            PadButton::L3 => "l3",
            PadButton::R3 => "r3",
            PadButton::Select => "select",
            PadButton::Start => "start",
            PadButton::DPadUp => "dpad_up",
            PadButton::DPadDown => "dpad_down",
            PadButton::DPadLeft => "dpad_left",
            PadButton::DPadRight => "dpad_right",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|b| b.name().eq_ignore_ascii_case(s))
    }

    pub fn from_gilrs(b: gilrs::Button) -> Option<Self> {
        use gilrs::Button as B;
        Some(match b {
            B::South => PadButton::South,
            B::East => PadButton::East,
            B::West => PadButton::West,
            B::North => PadButton::North,
            B::LeftTrigger => PadButton::L1,
            B::RightTrigger => PadButton::R1,
            B::LeftTrigger2 => PadButton::L2,
            B::RightTrigger2 => PadButton::R2,
            B::LeftThumb => PadButton::L3,
            B::RightThumb => PadButton::R3,
            B::Select => PadButton::Select,
            B::Start => PadButton::Start,
            B::DPadUp => PadButton::DPadUp,
            B::DPadDown => PadButton::DPadDown,
            B::DPadLeft => PadButton::DPadLeft,
            B::DPadRight => PadButton::DPadRight,
            _ => return None,
        })
    }
}

/// Action → gamepad buttons. The left stick always steers and thrusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadBindings {
    map: HashMap<Action, Vec<PadButton>>,
}

impl Default for PadBindings {
    fn default() -> Self {
        use PadButton::*;
        let table: &[(Action, PadButton)] = &[
            (Action::Left, DPadLeft),
            (Action::Right, DPadRight),
            (Action::Forward, DPadUp),
            (Action::Backward, DPadDown),
            (Action::Forward, R2),
            (Action::Backward, L2),
            (Action::Afterburner, L1),
            (Action::Afterburner, R1),
            (Action::Gun, South),
            (Action::Bomb, East),
            (Action::Mine, West),
            (Action::Multifire, North),
            (Action::Warp, Select),
            (Action::Menu, Start),
        ];
        let mut b = Self {
            map: HashMap::new(),
        };
        for &(a, button) in table {
            b.map.entry(a).or_default().push(button);
        }
        b
    }
}

impl PadBindings {
    pub fn get(&self, action: Action) -> &[PadButton] {
        self.map.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn set(&mut self, action: Action, buttons: Vec<PadButton>) {
        self.map.insert(action, buttons);
    }

    /// Actions a button drives.
    fn actions(&self, button: PadButton) -> impl Iterator<Item = Action> + '_ {
        Action::ALL
            .into_iter()
            .filter(move |a| self.get(*a).contains(&button))
    }
}

#[derive(Default, Clone, Copy)]
struct PadState {
    stick_x: f32,
    stick_y: f32,
}

/// One keystroke, whole: the key that went down and the character it typed.
///
/// A keyboard press is **one** event carrying both — press `Q` and the window
/// delivers the key `KeyQ` and the character `q` together. Carrying them as two
/// separate lists is what let a scripted `--press` drive half of this client and
/// not the other half (it could only fill the key list, so the menu, the chat
/// line and every form field ignored it), and it is why a key being held had to
/// be deduped for the whole UI to keep one press from dropping a bomb every
/// frame. Both of those are gone once a keystroke is one thing with the OS
/// repeat flag on it, and each reader says for itself whether a repeat counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stroke {
    /// The key that went down, where the window named one. A character arriving
    /// on its own — an IME finishing a composition — has none.
    pub key: Option<KeyCode>,
    /// The character it typed, where it typed one. Keys like Tab and Escape type
    /// a control character or nothing; readers that insert text drop those.
    pub text: Option<char>,
    /// The OS sent this because the key is being held, rather than because it
    /// went down. Text editing wants these — holding Backspace must clear a
    /// field — and a one-shot action does not.
    pub repeat: bool,
}

impl Stroke {
    /// A key going down with no character on it (Tab, Enter, an arrow).
    pub fn press(key: KeyCode) -> Self {
        Self {
            key: Some(key),
            text: None,
            repeat: false,
        }
    }

    /// A character arriving with no key of its own.
    pub fn text(ch: char) -> Self {
        Self {
            key: None,
            text: Some(ch),
            repeat: false,
        }
    }

    /// The character this stroke types.
    pub fn typing(self, ch: char) -> Self {
        Self {
            text: Some(ch),
            ..self
        }
    }

    /// The same stroke, marked as the OS repeating a held key.
    pub fn repeated(self, repeat: bool) -> Self {
        Self { repeat, ..self }
    }

    /// The key, but only where this is the key going down rather than the OS
    /// repeating it.
    ///
    /// One-shot keys read this, so holding Enter sends one line and holding F1
    /// turns one help page. Text editing and anything that scrolls reads
    /// [`Stroke::key`] instead, so holding Backspace clears a field at the rate
    /// the player's own keyboard repeats.
    pub fn first_press(self) -> Option<KeyCode> {
        if self.repeat { None } else { self.key }
    }
}

pub struct InputState {
    pub bindings: Bindings,
    pub pad_bindings: PadBindings,
    /// Gamepad buttons held.
    pad_held: HashSet<PadButton>,
    held: HashSet<KeyCode>,
    /// Actions whose key went down since the last tick (edge-triggered actions).
    edges: Vec<Action>,
    /// Keystrokes since the UI last looked, in the order they arrived — menus,
    /// the chat line and every form field read these.
    ///
    /// One list rather than two, because the key and the character it typed are
    /// one event: a reader that takes them apart can no longer put a Backspace
    /// back between the characters either side of it. The key is kept on the
    /// character because a key can be both a character and a game key, and only
    /// the chat line can say which it is this time; see
    /// [`Sandbox::ui_input`](crate::game::sandbox::Sandbox).
    strokes: Vec<Stroke>,
    /// The chat line has something on it, so keys that type characters belong to
    /// it rather than to the game.
    typing: bool,
    pad: PadState,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new(Bindings::default())
    }
}

impl InputState {
    /// Replaces the key table (from the settings screen).
    pub fn set_bindings(&mut self, bindings: Bindings) {
        self.bindings = bindings;
        self.clear();
    }

    pub fn new(bindings: Bindings) -> Self {
        Self {
            bindings,
            pad_bindings: PadBindings::default(),
            pad_held: HashSet::new(),
            held: HashSet::new(),
            edges: Vec::new(),
            strokes: Vec::new(),
            typing: false,
            pad: PadState::default(),
        }
    }

    /// A keystroke, from the window or from a scripted run.
    ///
    /// Edge-triggered game actions fire on the key going down and not on the OS
    /// repeating it: holding Tab drops one bomb, however long it is held. The
    /// stroke itself is queued either way, so the UI sees the repeats.
    pub fn press(&mut self, stroke: Stroke) {
        if let Some(code) = stroke.key {
            let fresh = self.held.insert(code);
            if fresh && !stroke.repeat {
                let (shift, ctrl) = (self.shift(), self.ctrl());
                let typing = self.typing;
                for a in Action::ALL {
                    if self.bindings.get(a).iter().any(|b| {
                        b.key == code
                            && mods_match(b.mods, shift, ctrl, code)
                            && !(typing && yields_to_chat(*b))
                    }) {
                        self.edges.push(a);
                    }
                }
            }
        }
        self.strokes.push(stroke);
    }

    /// A key coming back up.
    pub fn release(&mut self, code: KeyCode) {
        self.held.remove(&code);
    }

    /// A key going down or coming up with no character on it: the plain form of
    /// [`InputState::press`] and [`InputState::release`].
    pub fn key(&mut self, code: KeyCode, down: bool) {
        if down {
            self.press(Stroke::press(code));
        } else {
            self.release(code);
        }
    }

    /// Tracks a modifier without queueing a keystroke.
    ///
    /// The settings screen has the keyboard to itself, so nothing drains the
    /// queue while it is up; it still needs Shift and Ctrl held so that a
    /// captured binding reads correctly.
    pub fn modifier(&mut self, code: KeyCode, down: bool) {
        if down {
            self.held.insert(code);
        } else {
            self.held.remove(&code);
        }
    }

    /// Releases everything (window lost focus).
    pub fn clear(&mut self) {
        self.held.clear();
        self.strokes.clear();
        self.edges.clear();
        self.pad = PadState::default();
    }

    pub fn held(&self, code: KeyCode) -> bool {
        self.held.contains(&code)
    }

    pub fn shift(&self) -> bool {
        self.held(KeyCode::ShiftLeft) || self.held(KeyCode::ShiftRight)
    }

    pub fn ctrl(&self) -> bool {
        self.held(KeyCode::ControlLeft) || self.held(KeyCode::ControlRight)
    }

    /// Tells the key table whether the chat line has something on it. While it
    /// has, keys that type characters are the chat line's and drive nothing.
    pub fn set_typing(&mut self, typing: bool) {
        self.typing = typing;
    }

    /// Whether `key` with these modifiers is bound to anything, and so is the
    /// game's rather than a character when the chat line is empty.
    pub fn key_is_bound(&self, key: KeyCode, shift: bool, ctrl: bool) -> bool {
        Action::ALL.into_iter().any(|a| {
            self.bindings
                .get(a)
                .iter()
                .any(|b| b.key == key && mods_match(b.mods, shift, ctrl, key))
        })
    }

    /// Whether any binding of `action` is held right now.
    pub fn action_held(&self, action: Action) -> bool {
        let (shift, ctrl) = (self.shift(), self.ctrl());
        self.bindings.get(action).iter().any(|b| {
            self.held(b.key)
                && mods_match(b.mods, shift, ctrl, b.key)
                && !(self.typing && yields_to_chat(*b))
        }) || self
            .pad_bindings
            .get(action)
            .iter()
            .any(|b| self.pad_held.contains(b))
    }

    /// Takes the keystrokes since the last call, in the order they arrived.
    pub fn take_strokes(&mut self) -> Vec<Stroke> {
        std::mem::take(&mut self.strokes)
    }

    /// Drops the keystrokes since the last call.
    ///
    /// Used where another screen has the keyboard: whatever was typed under it
    /// is not the screen underneath's to read.
    pub fn drop_strokes(&mut self) {
        self.strokes.clear();
    }

    /// Takes the edge-triggered actions since the last call.
    pub fn take_edges(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.edges)
    }

    /// Builds one tick of ship input from held keys, the gamepad and the given edges.
    pub fn ship_input(&self, edges: &[Action]) -> ShipInput {
        let pad = self.pad;
        let h = |a| self.action_held(a);
        let e = |a| edges.contains(&a);
        ShipInput {
            forward: h(Action::Forward) || pad.stick_y < -DEAD_ZONE,
            backward: h(Action::Backward) || pad.stick_y > DEAD_ZONE,
            left: h(Action::Left) || pad.stick_x < -DEAD_ZONE,
            right: h(Action::Right) || pad.stick_x > DEAD_ZONE,
            afterburner: h(Action::Afterburner),
            gun: h(Action::Gun),
            bomb: h(Action::Bomb),
            mine: h(Action::Mine),
            repel: h(Action::Repel),
            burst: h(Action::Burst),
            thor: h(Action::Thor),
            decoy: h(Action::Decoy),
            rocket: h(Action::Rocket),
            brick: h(Action::Brick),
            portal: e(Action::Portal),
            warp: e(Action::Warp),
            toggle_multifire: e(Action::Multifire),
            toggle_stealth: e(Action::Stealth),
            toggle_cloak: e(Action::Cloak),
            toggle_xradar: e(Action::XRadar),
            toggle_antiwarp: e(Action::Antiwarp),
        }
    }

    /// Scripted input for smoke tests: thrust, weave and fire.
    pub fn autopilot(&mut self, t: f32) {
        let phase = t % 4.0;
        self.key(KeyCode::ArrowUp, phase < 3.0);
        self.key(KeyCode::ArrowRight, (1.0..1.4).contains(&phase));
        self.key(KeyCode::ControlLeft, (t % 0.5) < 0.25);
        self.key(KeyCode::Tab, (1.5..1.6).contains(&phase));
        self.key(KeyCode::F5, (0.5..0.6).contains(&phase));
        self.key(KeyCode::F6, (2.4..2.5).contains(&phase));
    }

    /// Applies one gamepad event.
    /// Applies a gamepad event. Returns the button pressed, if any (the settings
    /// screen uses it to rebind).
    pub fn gamepad(&mut self, event: gilrs::EventType) -> Option<PadButton> {
        use gilrs::{Axis, EventType};
        match event {
            EventType::ButtonChanged(button, value, _) => {
                return self.pad_button(PadButton::from_gilrs(button)?, value > 0.5);
            }
            EventType::AxisChanged(Axis::LeftStickX, v, _) => self.pad.stick_x = v,
            // gilrs reports up as positive; screen up is negative.
            EventType::AxisChanged(Axis::LeftStickY, v, _) => self.pad.stick_y = -v,
            EventType::Disconnected => {
                self.pad = PadState::default();
                self.pad_held.clear();
            }
            _ => {}
        }
        None
    }

    /// A gamepad button went down or up; returns it when newly pressed.
    pub fn pad_button(&mut self, b: PadButton, down: bool) -> Option<PadButton> {
        let was = self.pad_held.contains(&b);
        if down {
            self.pad_held.insert(b);
        } else {
            self.pad_held.remove(&b);
        }
        if down && !was {
            let actions: Vec<Action> = self.pad_bindings.actions(b).collect();
            self.edges.extend(actions);
            return Some(b);
        }
        None
    }

    /// Replaces the gamepad button table.
    pub fn set_pad_bindings(&mut self, bindings: PadBindings) {
        self.pad_bindings = bindings;
        self.pad_held.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_the_chat_line_s_only_once_there_is_a_line() {
        // SubSpace and Continuum have no chat mode, so the keyboard is shared. The
        // backquote is the case the stock table actually has: repel, and a
        // character. Which one it is depends on whether anything is typed yet.
        let mut i = InputState::default();
        i.key(KeyCode::Backquote, true);
        assert!(i.action_held(Action::Repel), "empty line: the key repels");

        i.set_typing(true);
        assert!(
            !i.action_held(Action::Repel),
            "with something typed, the key is a character"
        );
        // Everything that types nothing carries on, so you fly while you talk.
        i.key(KeyCode::ArrowUp, true);
        i.key(KeyCode::ControlLeft, true);
        i.key(KeyCode::Tab, true);
        let ship = i.ship_input(&[]);
        assert!(ship.forward && ship.gun && ship.bomb);

        i.set_typing(false);
        assert!(
            i.action_held(Action::Repel),
            "line sent: the key repels again"
        );
    }

    #[test]
    fn a_ctrl_binding_keeps_its_key_while_typing() {
        // Ctrl and a key put no character on the line, so there is nothing to lose
        // to: zoom stays on Ctrl+= and Ctrl+- whatever is being typed.
        let mut i = InputState::default();
        i.set_typing(true);
        i.key(KeyCode::ControlLeft, true);
        i.take_edges();
        i.key(KeyCode::Equal, true);
        assert!(i.take_edges().contains(&Action::ZoomIn));
    }

    #[test]
    fn keys_that_type_are_the_ones_with_a_character_on_them() {
        assert!(types_text(KeyCode::Backquote));
        assert!(types_text(KeyCode::KeyQ));
        assert!(types_text(KeyCode::Digit1));
        assert!(types_text(KeyCode::Space));
        assert!(!types_text(KeyCode::Tab));
        assert!(!types_text(KeyCode::Escape));
        assert!(!types_text(KeyCode::ArrowUp));
        assert!(!types_text(KeyCode::F1));
        assert!(!types_text(KeyCode::PageUp));
        // Which is what decides whether a binding gives way.
        let q = Binding {
            key: KeyCode::KeyQ,
            mods: Mods::Any,
        };
        assert!(yields_to_chat(q));
        assert!(!yields_to_chat(Binding {
            mods: Mods::Ctrl,
            ..q
        }));
        assert!(!yields_to_chat(Binding {
            key: KeyCode::F3,
            mods: Mods::Any,
        }));
    }

    #[test]
    fn a_bound_key_is_known_as_one() {
        let i = InputState::default();
        assert!(i.key_is_bound(KeyCode::Backquote, false, false));
        assert!(i.key_is_bound(KeyCode::Tab, false, false));
        assert!(!i.key_is_bound(KeyCode::KeyQ, false, false));
        assert!(!i.key_is_bound(KeyCode::Equal, false, false));
        assert!(i.key_is_bound(KeyCode::Equal, false, true));
    }

    #[test]
    fn keys_read_the_way_a_player_writes_them() {
        assert_eq!(key_name(KeyCode::ArrowUp), "Up");
        assert_eq!(key_name(KeyCode::ControlLeft), "Ctrl");
        assert_eq!(key_name(KeyCode::PageDown), "PgDn");
        assert_eq!(key_name(KeyCode::Backquote), "`");
        assert_eq!(key_name(KeyCode::KeyQ), "Q");
        assert_eq!(key_name(KeyCode::Digit8), "8");
        assert_eq!(key_name(KeyCode::Numpad3), "Numpad 3");
        // These already read correctly as they are.
        assert_eq!(key_name(KeyCode::F7), "F7");
        assert_eq!(key_name(KeyCode::Home), "Home");
    }

    #[test]
    fn the_label_is_read_from_the_table_and_follows_a_rebind() {
        let mut b = Bindings::default();
        assert_eq!(b.label(Action::Bomb), "Tab");
        assert_eq!(b.label(Action::Mine), "Shift+Tab");
        // The two Shifts are one key to a player, not two.
        assert_eq!(b.label(Action::Afterburner), "Shift");
        // Two genuinely different ways in are both worth knowing.
        assert_eq!(b.label(Action::Repel), "Shift+Ctrl or `");

        // Rebind, and the label follows -- which is the whole point of reading it
        // from the table instead of writing it down next to it.
        b.set(
            Action::Bomb,
            vec![Binding {
                key: KeyCode::KeyZ,
                mods: Mods::None,
            }],
        );
        assert_eq!(b.label(Action::Bomb), "Z");

        // An action with nothing bound says so rather than showing a blank.
        b.set(Action::Bomb, Vec::new());
        assert_eq!(b.label(Action::Bomb), "(unbound)");
    }

    #[test]
    fn modifiers_split_shared_keys() {
        let mut i = InputState::default();
        i.key(KeyCode::Tab, true);
        assert!(i.action_held(Action::Bomb) && !i.action_held(Action::Mine));
        i.key(KeyCode::ShiftLeft, true);
        assert!(i.action_held(Action::Mine) && !i.action_held(Action::Bomb));
        assert!(i.action_held(Action::Afterburner));
        i.key(KeyCode::ControlLeft, true);
        assert!(i.action_held(Action::Repel) && !i.action_held(Action::Gun));
        i.key(KeyCode::ShiftLeft, false);
        assert!(i.action_held(Action::Gun) && !i.action_held(Action::Repel));
        // Arrows work whatever modifiers are held.
        i.key(KeyCode::ShiftLeft, true);
        i.key(KeyCode::ArrowUp, true);
        assert!(i.ship_input(&[]).forward);
    }

    #[test]
    fn gamepad_buttons_follow_the_pad_table() {
        use PadButton as P;
        let mut i = InputState::default();
        // Defaults: south fires guns, north toggles multifire (an edge).
        assert_eq!(i.pad_button(P::South, true), Some(P::South));
        assert_eq!(
            i.pad_button(P::South, true),
            None,
            "held, not pressed again"
        );
        assert!(i.ship_input(&[]).gun);
        i.pad_button(P::North, true);
        assert!(i.take_edges().contains(&Action::Multifire));
        i.pad_button(P::South, false);
        assert!(!i.ship_input(&[]).gun);
        // Remapped: east fires guns, south does nothing.
        let mut pad = PadBindings::default();
        pad.set(Action::Gun, vec![PadButton::East]);
        pad.set(Action::Bomb, vec![]);
        i.set_pad_bindings(pad);
        i.pad_button(P::South, true);
        assert!(!i.ship_input(&[]).gun);
        i.pad_button(P::East, true);
        let input = i.ship_input(&[]);
        assert!(input.gun && !input.bomb);
        assert_eq!(P::from_name("DPAD_UP"), Some(P::DPadUp));
        assert_eq!(P::from_gilrs(gilrs::Button::LeftTrigger2), Some(P::L2));
    }

    #[test]
    fn edges_fire_once_per_press() {
        let mut i = InputState::default();
        i.key(KeyCode::Insert, true);
        i.key(KeyCode::Insert, true); // key repeat
        assert_eq!(i.take_edges(), vec![Action::Warp]);
        i.key(KeyCode::Insert, false);
        i.key(KeyCode::ShiftLeft, true);
        i.key(KeyCode::Insert, true);
        // Shift itself is the afterburner key, so it shows up as an edge too.
        assert_eq!(i.take_edges(), vec![Action::Afterburner, Action::Portal]);
        let input = i.ship_input(&[Action::Portal]);
        assert!(input.portal && !input.warp);
    }

    #[test]
    fn a_held_key_repeats_for_the_ui_and_not_for_the_game() {
        // The two halves of the same keystroke, which is why they are one list.
        // Tab is the bomb: however long it is held it drops one. The UI sees
        // every repeat and decides for itself, which is how holding Backspace
        // can clear a field while holding Tab drops nothing more.
        let mut i = InputState::default();
        i.press(Stroke::press(KeyCode::Tab));
        i.press(Stroke::press(KeyCode::Tab).repeated(true));
        i.press(Stroke::press(KeyCode::Tab).repeated(true));
        assert_eq!(i.take_edges(), vec![Action::Bomb], "one bomb, not three");

        let strokes = i.take_strokes();
        assert_eq!(strokes.len(), 3, "the UI sees all three");
        assert_eq!(strokes[0].first_press(), Some(KeyCode::Tab));
        assert_eq!(strokes[1].first_press(), None, "a repeat is not a press");
        assert_eq!(
            strokes[2].key,
            Some(KeyCode::Tab),
            "but it is still the key"
        );
    }

    #[test]
    fn a_keystroke_carries_its_character() {
        // What `--press` could not do: the menu and every form field read the
        // character, so a key arriving without one drove nothing.
        let mut i = InputState::default();
        i.press(Stroke::press(KeyCode::KeyQ).typing('q'));
        let strokes = i.take_strokes();
        assert_eq!(strokes.len(), 1, "one event, not two");
        assert_eq!(strokes[0].key, Some(KeyCode::KeyQ));
        assert_eq!(strokes[0].text, Some('q'));
    }

    #[test]
    fn every_key_that_types_has_a_character_for_a_scripted_run() {
        use KeyCode as K;
        // The international keys are named rather than drawn, and what they type
        // depends on the layout, so they are the deliberate exceptions.
        let unnamed = [K::IntlBackslash, K::IntlRo, K::IntlYen];
        for key in KEYS_THAT_TYPE {
            if unnamed.contains(&key) {
                continue;
            }
            assert!(
                char_for(key).is_some(),
                "{key:?} types text but has no character"
            );
        }
        assert_eq!(char_for(K::KeyQ), Some('q'));
        assert_eq!(char_for(K::Digit1), Some('1'));
        assert_eq!(char_for(K::Space), Some(' '));
        assert_eq!(char_for(K::Semicolon), Some(';'));
        assert_eq!(char_for(K::Numpad7), Some('7'));
        assert_eq!(char_for(K::NumpadAdd), Some('+'));
        // A key that types nothing has nothing to synthesise.
        assert_eq!(char_for(K::Enter), None);
        assert_eq!(char_for(K::Tab), None);
        assert_eq!(char_for(K::F1), None);
    }

    #[test]
    fn shift_reaches_the_characters_an_address_needs() {
        use KeyCode as K;
        // The one that matters: a scripted run cannot type `host:port` without
        // it, and the form will not submit an address with no colon in it.
        assert_eq!(shifted_char(K::Semicolon), Some(':'));
        assert_eq!(shifted_char(K::KeyQ), Some('Q'));
        assert_eq!(shifted_char(K::Digit1), Some('!'));
        assert_eq!(shifted_char(K::Minus), Some('_'));
        assert_eq!(shifted_char(K::Space), Some(' '));
        assert_eq!(shifted_char(K::Numpad7), Some('7'));
        assert_eq!(shifted_char(K::Enter), None);
    }

    /// Every key [`types_text`] accepts, so the character table can be checked
    /// against it rather than against itself.
    const KEYS_THAT_TYPE: [KeyCode; 65] = {
        use KeyCode as K;
        [
            K::Backquote,
            K::Minus,
            K::Equal,
            K::BracketLeft,
            K::BracketRight,
            K::Backslash,
            K::Semicolon,
            K::Quote,
            K::Comma,
            K::Period,
            K::Slash,
            K::Space,
            K::IntlBackslash,
            K::IntlRo,
            K::IntlYen,
            K::KeyA,
            K::KeyB,
            K::KeyC,
            K::KeyD,
            K::KeyE,
            K::KeyF,
            K::KeyG,
            K::KeyH,
            K::KeyI,
            K::KeyJ,
            K::KeyK,
            K::KeyL,
            K::KeyM,
            K::KeyN,
            K::KeyO,
            K::KeyP,
            K::KeyQ,
            K::KeyR,
            K::KeyS,
            K::KeyT,
            K::KeyU,
            K::KeyV,
            K::KeyW,
            K::KeyX,
            K::KeyY,
            K::KeyZ,
            K::Digit0,
            K::Digit1,
            K::Digit2,
            K::Digit3,
            K::Digit4,
            K::Digit5,
            K::Digit6,
            K::Digit7,
            K::Digit8,
            K::Digit9,
            K::Numpad0,
            K::Numpad1,
            K::Numpad2,
            K::Numpad3,
            K::Numpad4,
            K::Numpad5,
            K::Numpad6,
            K::Numpad7,
            K::Numpad8,
            K::Numpad9,
            K::NumpadAdd,
            K::NumpadSubtract,
            K::NumpadMultiply,
            K::NumpadDivide,
        ]
    };

    #[test]
    fn the_key_list_matches_what_types_text_accepts() {
        // The list above is hand-written, so it is checked against the function
        // it stands in for rather than trusted.
        for key in KEYS_THAT_TYPE {
            assert!(types_text(key), "{key:?} is in the list but types nothing");
        }
    }
}
