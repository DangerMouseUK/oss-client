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

pub struct InputState {
    pub bindings: Bindings,
    pub pad_bindings: PadBindings,
    /// Gamepad buttons held.
    pad_held: HashSet<PadButton>,
    held: HashSet<KeyCode>,
    /// Actions whose key went down since the last tick (edge-triggered actions).
    edges: Vec<Action>,
    /// Raw key presses since the UI last looked (menus, chat).
    pressed: Vec<KeyCode>,
    /// Characters typed since the UI last looked, each with the key that typed it.
    /// The key is kept because a key can be both a character and a game key, and
    /// only the chat line can say which it is this time; see
    /// [`Sandbox::ui_input`](crate::game::sandbox::Sandbox).
    pub typed: Vec<(Option<KeyCode>, char)>,
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
            pressed: Vec::new(),
            typed: Vec::new(),
            typing: false,
            pad: PadState::default(),
        }
    }

    pub fn key(&mut self, code: KeyCode, down: bool) {
        if down {
            if self.held.insert(code) {
                self.pressed.push(code);
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
        } else {
            self.held.remove(&code);
        }
    }

    /// Releases everything (window lost focus).
    pub fn clear(&mut self) {
        self.held.clear();
        self.pressed.clear();
        self.edges.clear();
        self.typed.clear();
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

    /// Takes the raw key presses since the last call.
    pub fn take_pressed(&mut self) -> Vec<KeyCode> {
        std::mem::take(&mut self.pressed)
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
}
