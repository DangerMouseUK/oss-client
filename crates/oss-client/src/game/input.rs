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
}

impl Action {
    pub const ALL: [Action; 29] = [
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
    ];

    /// Name used in the config file.
    #[allow(dead_code)] // read by the settings file loader
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
        }
    }

    #[allow(dead_code)] // read by the settings file loader
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
    #[allow(dead_code)] // read by the settings file loader
    pub fn set(&mut self, action: Action, bindings: Vec<Binding>) {
        self.map.insert(action, bindings);
    }

    pub fn get(&self, action: Action) -> &[Binding] {
        self.map.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }
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

#[derive(Default, Clone, Copy)]
struct PadState {
    stick_x: f32,
    stick_y: f32,
    dpad_left: bool,
    dpad_right: bool,
    dpad_up: bool,
    dpad_down: bool,
    thrust: bool,
    reverse: bool,
    afterburner: bool,
    gun: bool,
    bomb: bool,
    mine: bool,
}

pub struct InputState {
    pub bindings: Bindings,
    held: HashSet<KeyCode>,
    /// Actions whose key went down since the last tick (edge-triggered actions).
    edges: Vec<Action>,
    /// Raw key presses since the UI last looked (menus, chat).
    pressed: Vec<KeyCode>,
    /// Text typed since the UI last looked.
    pub typed: String,
    pad: PadState,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new(Bindings::default())
    }
}

impl InputState {
    pub fn new(bindings: Bindings) -> Self {
        Self {
            bindings,
            held: HashSet::new(),
            edges: Vec::new(),
            pressed: Vec::new(),
            typed: String::new(),
            pad: PadState::default(),
        }
    }

    pub fn key(&mut self, code: KeyCode, down: bool) {
        if down {
            if self.held.insert(code) {
                self.pressed.push(code);
                let (shift, ctrl) = (self.shift(), self.ctrl());
                for a in Action::ALL {
                    if self
                        .bindings
                        .get(a)
                        .iter()
                        .any(|b| b.key == code && mods_match(b.mods, shift, ctrl, code))
                    {
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

    /// Whether any binding of `action` is held right now.
    pub fn action_held(&self, action: Action) -> bool {
        let (shift, ctrl) = (self.shift(), self.ctrl());
        self.bindings
            .get(action)
            .iter()
            .any(|b| self.held(b.key) && mods_match(b.mods, shift, ctrl, b.key))
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
            forward: h(Action::Forward) || pad.thrust || pad.dpad_up || pad.stick_y < -DEAD_ZONE,
            backward: h(Action::Backward)
                || pad.reverse
                || pad.dpad_down
                || pad.stick_y > DEAD_ZONE,
            left: h(Action::Left) || pad.dpad_left || pad.stick_x < -DEAD_ZONE,
            right: h(Action::Right) || pad.dpad_right || pad.stick_x > DEAD_ZONE,
            afterburner: h(Action::Afterburner) || pad.afterburner,
            gun: h(Action::Gun) || pad.gun,
            bomb: h(Action::Bomb) || pad.bomb,
            mine: h(Action::Mine) || pad.mine,
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
    pub fn gamepad(&mut self, event: gilrs::EventType) {
        use gilrs::{Axis, Button, EventType};
        let p = &mut self.pad;
        match event {
            EventType::ButtonChanged(button, value, _) => {
                let down = value > 0.5;
                match button {
                    Button::DPadLeft => p.dpad_left = down,
                    Button::DPadRight => p.dpad_right = down,
                    Button::DPadUp => p.dpad_up = down,
                    Button::DPadDown => p.dpad_down = down,
                    Button::RightTrigger2 => p.thrust = down,
                    Button::LeftTrigger2 => p.reverse = down,
                    Button::LeftTrigger | Button::RightTrigger => p.afterburner = down,
                    Button::South => p.gun = down,
                    Button::East => p.bomb = down,
                    Button::West => p.mine = down,
                    Button::North if down => self.edges.push(Action::Multifire),
                    Button::Select if down => self.edges.push(Action::Warp),
                    Button::Start if down => self.edges.push(Action::Menu),
                    _ => {}
                }
            }
            EventType::AxisChanged(Axis::LeftStickX, v, _) => p.stick_x = v,
            // gilrs reports up as positive; screen up is negative.
            EventType::AxisChanged(Axis::LeftStickY, v, _) => p.stick_y = -v,
            EventType::Disconnected => self.pad = PadState::default(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
