//! Keyboard and gamepad state mapped to ship controls, using Continuum's default keys:
//! arrows fly, Shift afterburner, Ctrl guns, Tab bombs, Shift+Tab mines, Delete
//! toggles multifire.

use std::collections::HashSet;

use oss_sim::ShipInput;
use winit::keyboard::KeyCode;

/// Analog stick dead zone.
const DEAD_ZONE: f32 = 0.35;

#[derive(Default)]
pub struct InputState {
    held: HashSet<KeyCode>,
    /// Keys pressed since the last sim tick consumed them.
    pressed: Vec<KeyCode>,
    pad: PadState,
    pad_multifire_pressed: bool,
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

impl InputState {
    pub fn key(&mut self, code: KeyCode, down: bool) {
        if down {
            if self.held.insert(code) {
                self.pressed.push(code);
            }
        } else {
            self.held.remove(&code);
        }
    }

    /// Releases everything (window lost focus).
    pub fn clear(&mut self) {
        self.held.clear();
        self.pressed.clear();
        self.pad = PadState::default();
    }

    pub fn held(&self, code: KeyCode) -> bool {
        self.held.contains(&code)
    }

    /// Takes the keys pressed since the last call (for menus and toggles).
    pub fn take_pressed(&mut self) -> Vec<KeyCode> {
        std::mem::take(&mut self.pressed)
    }

    fn shift(&self) -> bool {
        self.held(KeyCode::ShiftLeft) || self.held(KeyCode::ShiftRight)
    }

    fn ctrl(&self) -> bool {
        self.held(KeyCode::ControlLeft) || self.held(KeyCode::ControlRight)
    }

    /// Builds this tick's ship input. `toggle_multifire` is passed in because the
    /// edge is detected from [`InputState::take_pressed`].
    pub fn ship_input(&mut self, toggle_multifire: bool) -> ShipInput {
        let shift = self.shift();
        let pad = self.pad;
        let tab = self.held(KeyCode::Tab);
        let mf_pad = std::mem::take(&mut self.pad_multifire_pressed);
        ShipInput {
            forward: self.held(KeyCode::ArrowUp)
                || pad.thrust
                || pad.dpad_up
                || pad.stick_y < -DEAD_ZONE,
            backward: self.held(KeyCode::ArrowDown)
                || pad.reverse
                || pad.dpad_down
                || pad.stick_y > DEAD_ZONE,
            left: self.held(KeyCode::ArrowLeft) || pad.dpad_left || pad.stick_x < -DEAD_ZONE,
            right: self.held(KeyCode::ArrowRight) || pad.dpad_right || pad.stick_x > DEAD_ZONE,
            afterburner: shift || pad.afterburner,
            // Shift+Ctrl is repel in Continuum, so guns need Ctrl without Shift.
            gun: (self.ctrl() && !shift) || pad.gun,
            bomb: (tab && !shift) || pad.bomb,
            mine: (tab && shift) || pad.mine,
            toggle_multifire: toggle_multifire || mf_pad,
        }
    }

    /// Scripted input for smoke tests: thrust, weave and fire.
    pub fn autopilot(&mut self, t: f32) {
        let phase = t % 4.0;
        self.key(KeyCode::ArrowUp, phase < 3.0);
        self.key(KeyCode::ArrowRight, (1.0..1.4).contains(&phase));
        self.key(KeyCode::ControlLeft, (t % 0.5) < 0.25);
        self.key(KeyCode::Tab, (1.5..1.6).contains(&phase));
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
                    Button::North if down => self.pad_multifire_pressed = true,
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
