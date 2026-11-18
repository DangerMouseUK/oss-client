//! The settings window: video, audio, accessibility, keys, gamepad and macros.
//!
//! Drawn the way the start-up screen is drawn, in the game's own box: the grey
//! three-ply border, the 8x12 font, green labels, white values, red headings and
//! a yellow line of keys at the foot that are buttons as well. The sections run
//! down the left and the one picked has its settings on the right.
//!
//! **Every part of it works from the keyboard and from the pointer.** Tab moves
//! between the sections, the arrows between the lines and along them, Enter
//! changes what is picked, Ctrl+S saves and Esc closes; a click on any line,
//! arrow, button or key on the foot line does what the key for it does. The
//! keys arrive through the client's own input, the same queue every other screen
//! reads, so `--press` reaches this window as it reaches the rest.
//!
//! **No player name.** It is set on the start-up screen and remembered in
//! `login.ini`, so a box here would be a second place setting one thing -- and
//! the one that almost never wins, since the remembered name beats it. A player
//! who changed it here and saw no effect had been lied to.
//!
//! Opened from the start-up screen (F2, or Options) and from the Esc menu (O).
//! Edits a copy of the settings; Save writes `oss-client.ini` and applies what
//! can change while playing (bindings, macros, volume, pixel scale, fullscreen).
//! Rebinding waits for the next key press, which is captured with its modifiers
//! (see [`SettingsUi::capture`]).

use winit::keyboard::KeyCode;

use crate::config::Config;
use crate::game::chat::{MAX_CHAT, wrap};
use crate::game::input::{Action, Binding, Mods, PadButton, Stroke, key_name};
use crate::game::palette::PaletteKind;
use crate::game::start_spots::Rect;
use crate::game::view::{Canvas, color, ui_color};
use crate::gfx::Renderer;

/// What the window asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    None,
    Save,
    Close,
}

/// A key being captured: replacing an action's keys, adding one, or a new macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capture {
    Replace(Action),
    Add(Action),
    Macro,
    /// The next gamepad button for an action.
    Pad(Action),
}

/// The sections down the left of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Video,
    Audio,
    Accessibility,
    Keys,
    Gamepad,
    Macros,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Video,
        Section::Audio,
        Section::Accessibility,
        Section::Keys,
        Section::Gamepad,
        Section::Macros,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Video => "Video",
            Section::Audio => "Audio",
            Section::Accessibility => "Accessibility",
            Section::Keys => "Keys",
            Section::Gamepad => "Gamepad",
            Section::Macros => "Macros",
        }
    }

    /// The section after this one, or before it, going round.
    fn next(self, back: bool) -> Section {
        let n = Self::ALL.len();
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[if back { (i + n - 1) % n } else { (i + 1) % n }]
    }

    /// The two column headings over the section's lines.
    fn columns(self) -> (&'static str, &'static str) {
        match self {
            Section::Video | Section::Audio | Section::Accessibility => ("Setting", "Value"),
            Section::Keys => ("Action", "Keys"),
            Section::Gamepad => ("Action", "Buttons"),
            Section::Macros => ("Key", "Sends"),
        }
    }

    /// What the section says before its lines, if anything.
    fn note(self) -> &'static str {
        match self {
            Section::Gamepad => "The left stick always steers and thrusts.",
            Section::Macros => {
                "A key sends its line as chat, with the prefixes you would type \
                 yourself. A macro on a key that types a character only fires \
                 while the chat line is empty."
            }
            _ => "",
        }
    }

    /// One line on what the section holds, for the foot while the pointer is
    /// over its name.
    fn hint(self) -> &'static str {
        match self {
            Section::Video => "Pixel scale, fullscreen and vsync.",
            Section::Audio => "Sound on or off, and how loud.",
            Section::Accessibility => "Colours for colour-blind players, and text size.",
            Section::Keys => "The keys for every action.",
            Section::Gamepad => "The gamepad buttons for every action.",
            Section::Macros => "Keys that send a line of chat.",
        }
    }
}

/// One line of a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Scale,
    Fullscreen,
    Vsync,
    Mute,
    Volume,
    Palette,
    InterfaceSize,
    Key(Action),
    RestoreKeys,
    Pad(Action),
    RestorePad,
    /// A macro, by its index in the settings.
    Macro(usize),
    AddMacro,
}

/// What a line holds, which decides what the keys and the pointer do to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A number, changed with Left and Right (Enter steps up and goes round).
    Number,
    /// On or off: Enter, Left, Right or a click flips it.
    Toggle,
    /// One of a list, cycled with Left and Right (Enter goes forward).
    Choice,
    /// Buttons along the line, picked with Left and Right and pressed with Enter.
    Parts,
    /// The whole line is one button.
    Button,
}

/// The pieces of a [`Kind::Parts`] line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// The text a macro sends; typing goes into it.
    Line,
    Change,
    Add,
    Clear,
    Remove,
}

impl Part {
    fn label(self) -> &'static str {
        match self {
            Part::Line => "",
            Part::Change => "Change",
            Part::Add => "Add",
            Part::Clear => "Clear",
            Part::Remove => "Remove",
        }
    }
}

impl Row {
    fn kind(self) -> Kind {
        match self {
            Row::Scale | Row::Volume | Row::InterfaceSize => Kind::Number,
            Row::Fullscreen | Row::Vsync | Row::Mute => Kind::Toggle,
            Row::Palette => Kind::Choice,
            Row::Key(_) | Row::Pad(_) | Row::Macro(_) => Kind::Parts,
            Row::RestoreKeys | Row::RestorePad | Row::AddMacro => Kind::Button,
        }
    }

    fn parts(self) -> &'static [Part] {
        match self {
            Row::Key(_) => &[Part::Change, Part::Add, Part::Clear],
            Row::Pad(_) => &[Part::Change, Part::Clear],
            Row::Macro(_) => &[Part::Line, Part::Remove],
            _ => &[],
        }
    }
}

/// The foot line's keys, each of which is a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Foot {
    Section,
    Change,
    Save,
    Close,
}

/// Something in the window a pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    Section(Section),
    /// A line, by its index in the section.
    Row(usize),
    /// One of a line's buttons: the line and the part's index.
    Part(usize, usize),
    /// A number's or a choice's arrows: the line, and down (-1) or up (1).
    Step(usize, i32),
    /// The arrows that scroll a long list: up (-1) or down (1).
    Scroll(i32),
    Foot(Foot),
}

pub struct SettingsUi {
    pub cfg: Config,
    capture: Option<Capture>,
    /// Shown at the foot (saved, failed, bound...).
    pub status: Option<String>,
    /// The settings as last saved, or as the window opened on them, so that
    /// closing can tell whether anything would be lost.
    saved: Config,
    section: Section,
    /// The picked line, by its index in the section.
    row: usize,
    /// The picked button along that line.
    part: usize,
    /// The first line shown of a list too long for the window.
    scroll: usize,
    /// How many lines the list had room for, as last drawn.
    visible: usize,
    /// Esc was pressed with changes unsaved, and the window said so; a second
    /// Esc closes it.
    warned: bool,
    /// Where everything was drawn last frame, so a click finds it.
    spots: Vec<(Rect, Spot)>,
}

/// How a binding reads in the window and its messages: the key as a player
/// would name it, with the modifier it needs.
pub fn binding_label(b: &Binding) -> String {
    let prefix = match b.mods {
        Mods::Any => "",
        Mods::None => "(no Shift) ",
        Mods::Shift => "Shift+",
        Mods::Ctrl => "Ctrl+",
    };
    format!("{prefix}{}", key_name(b.key))
}

/// An action's name as a heading reads: `statbox_up` is "Statbox up".
fn action_title(a: Action) -> String {
    let words = a.name().replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => words,
    }
}

/// `s` cut to `room` characters, with `..` where it was cut.
fn fit(s: &str, room: usize) -> String {
    if s.chars().count() <= room {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(room.saturating_sub(2)).collect();
    cut.push_str("..");
    cut
}

/// The highest pixel scale the window offers; the renderer takes no more.
const SCALE_MAX: i32 = 8;
/// The most the interface can be drawn larger than the world.
const INTERFACE_MAX: i32 = 4;
/// How far one press moves the volume.
const VOLUME_STEP: i32 = 5;

impl SettingsUi {
    pub fn new(cfg: Config) -> Self {
        Self {
            saved: cfg.clone(),
            cfg,
            capture: None,
            status: None,
            section: Section::Video,
            row: 0,
            part: 0,
            scroll: 0,
            visible: 12,
            warned: false,
            spots: Vec::new(),
        }
    }

    /// Waiting for a key press to bind.
    pub fn capturing(&self) -> bool {
        matches!(
            self.capture,
            Some(Capture::Replace(_) | Capture::Add(_) | Capture::Macro)
        )
    }

    /// A gamepad button was pressed: binds it if one is being captured. Returns
    /// whether it was used.
    pub fn capture_pad(&mut self, button: PadButton) -> bool {
        let Some(Capture::Pad(a)) = self.capture else {
            return false;
        };
        self.capture = None;
        self.cfg.pad.set(a, vec![button]);
        self.status = Some(format!("{} on {}", a.name(), button.name()));
        true
    }

    /// The pressed key (with Shift/Ctrl as held) for the binding being captured.
    /// Escape cancels.
    pub fn capture(&mut self, key: KeyCode, shift: bool, ctrl: bool) {
        let Some(what) = self.capture.take() else {
            return;
        };
        if let Capture::Pad(_) = what {
            // Escape cancels a gamepad capture; other keys are ignored.
            if key != KeyCode::Escape {
                self.capture = Some(what);
            }
            return;
        }
        if key == KeyCode::Escape {
            self.status = Some("Cancelled".into());
            return;
        }
        let is_modifier = matches!(
            key,
            KeyCode::ShiftLeft | KeyCode::ShiftRight | KeyCode::ControlLeft | KeyCode::ControlRight
        );
        let mods = match (ctrl && !is_modifier, shift && !is_modifier) {
            (true, _) => Mods::Ctrl,
            (_, true) => Mods::Shift,
            _ => Mods::Any,
        };
        let b = Binding { key, mods };
        match what {
            Capture::Replace(a) => self.cfg.bindings.set(a, vec![b]),
            Capture::Add(a) => self.cfg.bindings.add(a, b),
            Capture::Macro => self.cfg.macros.push((b, String::new())),
            Capture::Pad(_) => {}
        }
        self.status = Some(format!("Bound {}", binding_label(&b)));
    }

    /// The settings as they are now have been written: closing loses nothing.
    pub fn mark_saved(&mut self) {
        self.saved = self.cfg.clone();
        self.warned = false;
    }

    /// The lines of the section on show, in order.
    fn rows(&self) -> Vec<Row> {
        match self.section {
            Section::Video => vec![Row::Scale, Row::Fullscreen, Row::Vsync],
            Section::Audio => vec![Row::Mute, Row::Volume],
            Section::Accessibility => vec![Row::Palette, Row::InterfaceSize],
            Section::Keys => Action::ALL
                .into_iter()
                .map(Row::Key)
                .chain([Row::RestoreKeys])
                .collect(),
            Section::Gamepad => Action::ALL
                .into_iter()
                .map(Row::Pad)
                .chain([Row::RestorePad])
                .collect(),
            Section::Macros => (0..self.cfg.macros.len())
                .map(Row::Macro)
                .chain([Row::AddMacro])
                .collect(),
        }
    }

    fn current(&self) -> Option<Row> {
        self.rows().get(self.row).copied()
    }

    /// Applies one frame of keystrokes; `shift` and `ctrl` are what is held.
    ///
    /// Tab and Shift+Tab move between the sections, Up and Down between the
    /// lines (PgUp, PgDn, Home and End further), Left and Right change a
    /// number or a choice or move along a line's buttons, Enter changes what is
    /// picked, Ctrl+S saves and Esc closes. Typing goes into a macro's line.
    /// While a key is being captured, the next key goes to that instead.
    pub fn input(&mut self, strokes: &[Stroke], shift: bool, ctrl: bool) -> UiAction {
        for &stroke in strokes {
            if self.capture.is_some() {
                if let Some(key) = stroke.first_press() {
                    self.capture_key(key, shift, ctrl);
                }
                continue;
            }
            let action = self.stroke(stroke, shift, ctrl);
            if action != UiAction::None {
                return action;
            }
        }
        UiAction::None
    }

    /// [`SettingsUi::capture`], and the cursor put on a macro the key just made,
    /// so the line it sends can be typed straight away.
    fn capture_key(&mut self, key: KeyCode, shift: bool, ctrl: bool) {
        let macros = self.cfg.macros.len();
        self.capture(key, shift, ctrl);
        if self.cfg.macros.len() > macros {
            self.row = macros;
            self.part = 0;
        }
    }

    fn stroke(&mut self, s: Stroke, shift: bool, ctrl: bool) -> UiAction {
        // Typing goes into a macro's line, whichever of its buttons is picked;
        // nothing else in the window answers to a character, so nothing can be
        // typed by mistake into a key.
        if let (Some(Row::Macro(i)), Some(c)) = (self.current(), s.text)
            && !c.is_control()
            && !ctrl
        {
            if let Some((_, line)) = self.cfg.macros.get_mut(i)
                && line.chars().count() < MAX_CHAT
            {
                line.push(c);
            }
            self.part = 0;
            return UiAction::None;
        }
        if s.key != Some(KeyCode::Escape) {
            self.warned = false;
        }
        let len = self.rows().len();
        let page = self.visible.saturating_sub(1).max(1);
        match s.key {
            Some(KeyCode::Tab) => self.open_section(self.section.next(shift)),
            Some(KeyCode::ArrowUp) => self.move_to((self.row + len - 1) % len.max(1)),
            Some(KeyCode::ArrowDown) => self.move_to((self.row + 1) % len.max(1)),
            Some(KeyCode::PageUp) => self.move_to(self.row.saturating_sub(page)),
            Some(KeyCode::PageDown) => self.move_to((self.row + page).min(len.saturating_sub(1))),
            Some(KeyCode::Home) => self.move_to(0),
            Some(KeyCode::End) => self.move_to(len.saturating_sub(1)),
            Some(KeyCode::ArrowLeft) => self.sideways(-1),
            Some(KeyCode::ArrowRight) => self.sideways(1),
            Some(KeyCode::Backspace) => {
                if let Some(Row::Macro(i)) = self.current()
                    && let Some((_, line)) = self.cfg.macros.get_mut(i)
                {
                    line.pop();
                    self.part = 0;
                }
            }
            _ => {}
        }
        match s.first_press() {
            Some(KeyCode::Enter | KeyCode::NumpadEnter) => self.activate(),
            Some(KeyCode::Escape) => return self.close(),
            Some(KeyCode::KeyS) if ctrl => return UiAction::Save,
            _ => {}
        }
        UiAction::None
    }

    /// Shows a section, from its first line.
    fn open_section(&mut self, section: Section) {
        self.section = section;
        self.row = 0;
        self.part = 0;
        self.scroll = 0;
    }

    /// Picks a line, keeping the same button along it where it has one.
    fn move_to(&mut self, row: usize) {
        self.row = row;
        let parts = self.current().map_or(0, |r| r.parts().len());
        self.part = self.part.min(parts.saturating_sub(1));
    }

    /// Left (-1) or Right (1) on the picked line.
    fn sideways(&mut self, dir: i32) {
        let Some(row) = self.current() else {
            return;
        };
        match row.kind() {
            Kind::Number | Kind::Choice => self.step(row, dir, false),
            Kind::Toggle => self.flip(row),
            Kind::Parts => {
                let last = row.parts().len().saturating_sub(1);
                self.part = if dir < 0 {
                    self.part.saturating_sub(1)
                } else {
                    (self.part + 1).min(last)
                };
            }
            Kind::Button => {}
        }
    }

    /// Enter, or a click on a button: does what the picked line or button is for.
    fn activate(&mut self) {
        let Some(row) = self.current() else {
            return;
        };
        match row.kind() {
            Kind::Number | Kind::Choice => self.step(row, 1, true),
            Kind::Toggle => self.flip(row),
            Kind::Parts | Kind::Button => {
                let part = row.parts().get(self.part).copied();
                self.press(row, part);
            }
        }
    }

    fn press(&mut self, row: Row, part: Option<Part>) {
        match (row, part) {
            (Row::Key(a), Some(Part::Change)) => self.capture = Some(Capture::Replace(a)),
            (Row::Key(a), Some(Part::Add)) => self.capture = Some(Capture::Add(a)),
            (Row::Key(a), Some(Part::Clear)) => self.cfg.bindings.set(a, Vec::new()),
            (Row::Pad(a), Some(Part::Change)) => self.capture = Some(Capture::Pad(a)),
            (Row::Pad(a), Some(Part::Clear)) => self.cfg.pad.set(a, Vec::new()),
            (Row::Macro(i), Some(Part::Remove)) => {
                if i < self.cfg.macros.len() {
                    self.cfg.macros.remove(i);
                }
                self.part = 0;
            }
            (Row::RestoreKeys, _) => {
                self.cfg.bindings = Default::default();
                self.status = Some("Every key is back as Continuum has it.".into());
            }
            (Row::RestorePad, _) => {
                self.cfg.pad = Default::default();
                self.status = Some("Every button is back to the default layout.".into());
            }
            (Row::AddMacro, _) => self.capture = Some(Capture::Macro),
            _ => {}
        }
    }

    /// Moves a number or a choice one step. `wrap` goes round from the top to the
    /// bottom, which Enter does so that one key can reach every value.
    fn step(&mut self, row: Row, dir: i32, wrap: bool) {
        let around = |v: i32, max: i32| {
            if wrap && v > max { 0 } else { v.clamp(0, max) }
        };
        match row {
            Row::Scale => {
                let v = around(self.cfg.scale.unwrap_or(0) as i32 + dir, SCALE_MAX);
                self.cfg.scale = (v > 0).then_some(v as u32);
            }
            Row::Volume => {
                // To the next multiple of the step, so a volume of 63 from the
                // old slider goes to 65 or 60 rather than staying off the grid.
                let v = self.cfg.volume as i32;
                let next = if dir > 0 {
                    (v / VOLUME_STEP + 1) * VOLUME_STEP
                } else {
                    (v + VOLUME_STEP - 1) / VOLUME_STEP * VOLUME_STEP - VOLUME_STEP
                };
                self.cfg.volume = around(next, 100) as u32;
            }
            Row::InterfaceSize => {
                self.cfg.ui_scale = around(self.cfg.ui_scale as i32 + dir, INTERFACE_MAX) as u32;
            }
            Row::Palette => {
                let all = PaletteKind::ALL;
                let n = all.len() as i32;
                let i = all.iter().position(|p| *p == self.cfg.palette).unwrap_or(0) as i32;
                let next = if wrap {
                    (i + dir).rem_euclid(n)
                } else {
                    (i + dir).clamp(0, n - 1)
                };
                self.cfg.palette = all[next as usize];
            }
            _ => {}
        }
    }

    fn flip(&mut self, row: Row) {
        match row {
            Row::Fullscreen => self.cfg.fullscreen = !self.cfg.fullscreen,
            Row::Vsync => self.cfg.vsync = !self.cfg.vsync,
            Row::Mute => self.cfg.mute = !self.cfg.mute,
            _ => {}
        }
    }

    /// Esc, or the foot's Close. With changes unsaved the first one says so
    /// rather than throwing them away; the second closes.
    fn close(&mut self) -> UiAction {
        if self.cfg != self.saved && !self.warned {
            self.warned = true;
            self.status =
                Some("Not saved. Ctrl+S saves; Esc again closes without saving.".to_string());
            return UiAction::None;
        }
        UiAction::Close
    }

    /// The wheel: `rows` lines down (up when negative), carrying the picked line
    /// with the list where it would otherwise leave the window.
    pub fn scroll(&mut self, rows: i32) {
        let len = self.rows().len();
        let most = len.saturating_sub(self.visible);
        self.scroll = (self.scroll as i64 + rows as i64).clamp(0, most as i64) as usize;
        let last = (self.scroll + self.visible).min(len).saturating_sub(1);
        if self.row < self.scroll || self.row > last {
            self.move_to(self.row.clamp(self.scroll, last));
        }
    }

    /// Moves the list so the picked line is on it.
    fn keep_in_view(&mut self, len: usize) {
        if self.row < self.scroll {
            self.scroll = self.row;
        }
        if self.row >= self.scroll + self.visible {
            self.scroll = self.row + 1 - self.visible;
        }
        self.scroll = self.scroll.min(len.saturating_sub(self.visible));
    }

    /// A click at (`x`, `y`) in UI pixels, against what the last frame drew.
    pub fn click(&mut self, x: f32, y: f32) -> UiAction {
        // A key is being waited for: a click stops waiting rather than doing
        // something else underneath the question.
        if self.capture.take().is_some() {
            self.status = Some("Cancelled".into());
            return UiAction::None;
        }
        let Some(spot) = self.spot_at(x, y) else {
            return UiAction::None;
        };
        if spot != Spot::Foot(Foot::Close) {
            self.warned = false;
        }
        match spot {
            Spot::Section(s) => self.open_section(s),
            Spot::Row(i) => {
                self.row = i;
                self.part = 0;
                if let Some(row) = self.current() {
                    match row.kind() {
                        Kind::Toggle | Kind::Button => self.activate(),
                        _ => {}
                    }
                }
            }
            Spot::Part(i, p) => {
                self.row = i;
                self.part = p;
                self.activate();
            }
            Spot::Step(i, dir) => {
                self.row = i;
                self.part = 0;
                if let Some(row) = self.current() {
                    self.step(row, dir, false);
                }
            }
            Spot::Scroll(dir) => self.scroll(dir * self.visible.saturating_sub(1).max(1) as i32),
            Spot::Foot(Foot::Section) => self.open_section(self.section.next(false)),
            Spot::Foot(Foot::Change) => self.activate(),
            Spot::Foot(Foot::Save) => return UiAction::Save,
            Spot::Foot(Foot::Close) => return self.close(),
        }
        UiAction::None
    }

    fn spot_at(&self, x: f32, y: f32) -> Option<Spot> {
        self.spots
            .iter()
            .rev()
            .find(|(r, _)| r.holds(x, y))
            .map(|(_, s)| *s)
    }

    fn label(&self, row: Row) -> String {
        match row {
            Row::Scale => "Pixel scale".into(),
            Row::Fullscreen => "Fullscreen".into(),
            Row::Vsync => "Vsync".into(),
            Row::Mute => "Mute".into(),
            Row::Volume => "Volume".into(),
            Row::Palette => "Colour palette".into(),
            Row::InterfaceSize => "Interface size".into(),
            Row::Key(a) | Row::Pad(a) => action_title(a),
            Row::Macro(i) => self
                .cfg
                .macros
                .get(i)
                .map_or_else(String::new, |(b, _)| binding_label(b)),
            Row::RestoreKeys => "Restore Continuum defaults".into(),
            Row::RestorePad => "Restore default gamepad layout".into(),
            Row::AddMacro => "Add a macro key".into(),
        }
    }

    /// What a line shows in the value column.
    fn value(&self, row: Row) -> String {
        let on = |b: bool| if b { "On" } else { "Off" }.to_string();
        match row {
            Row::Scale => match self.cfg.scale {
                Some(n) if n > 0 => n.to_string(),
                _ => "from display".into(),
            },
            Row::Fullscreen => on(self.cfg.fullscreen),
            Row::Vsync => on(self.cfg.vsync),
            Row::Mute => on(self.cfg.mute),
            Row::Volume => self.cfg.volume.to_string(),
            Row::Palette => self.cfg.palette.name().replace('_', " "),
            Row::InterfaceSize => match self.cfg.ui_scale {
                0 => "normal".into(),
                n => format!("+{n}"),
            },
            Row::Key(a) => {
                // Each name once: the left and right Shift are two bindings
                // that a player would only ever call "Shift".
                let mut keys: Vec<String> = Vec::new();
                for b in self.cfg.bindings.get(a) {
                    let name = binding_label(b);
                    if !keys.contains(&name) {
                        keys.push(name);
                    }
                }
                if keys.is_empty() {
                    "(none)".into()
                } else {
                    keys.join(", ")
                }
            }
            Row::Pad(a) => {
                let buttons: Vec<&str> = self.cfg.pad.get(a).iter().map(|b| b.name()).collect();
                if buttons.is_empty() {
                    "(none)".into()
                } else {
                    buttons.join(", ")
                }
            }
            Row::Macro(i) => self
                .cfg
                .macros
                .get(i)
                .map_or_else(String::new, |(_, line)| line.clone()),
            Row::RestoreKeys | Row::RestorePad | Row::AddMacro => String::new(),
        }
    }

    /// One line on what a line does, for the foot -- the job Continuum's
    /// `infobox` does on its start-up screen.
    fn hint(&self, row: Row) -> String {
        match row {
            Row::Scale => {
                "Screen pixels to each game pixel. The zoom keys change it in game.".into()
            }
            Row::Fullscreen => "Borderless fullscreen. Applies when saved.".into(),
            Row::Vsync => "Caps the frame rate to the display's. Applies at the next start.".into(),
            Row::Mute => "No sound at all. Applies at the next start.".into(),
            Row::Volume => "How loud, 0 to 100. Applies when saved.".into(),
            Row::Palette => match self.cfg.palette {
                PaletteKind::Standard => "Continuum's own colours.",
                PaletteKind::Deuteranopia => "For red-green colour blindness (green-weak).",
                PaletteKind::Protanopia => "For red-green colour blindness (red-weak).",
                PaletteKind::Tritanopia => "For blue-yellow colour blindness.",
                PaletteKind::HighContrast => "Strong colours on the text and the radar.",
            }
            .into(),
            Row::InterfaceSize => "Draws chat, the HUD and menus larger than the world.".into(),
            Row::Key(_) | Row::Pad(_) => format!("{}: {}", self.label(row), self.value(row)),
            Row::RestoreKeys => "Puts every key back as Continuum has it.".into(),
            Row::RestorePad => "Puts every button back to the default layout.".into(),
            Row::Macro(_) => "Type the line it sends; Backspace takes from it.".into(),
            Row::AddMacro => "Then press the key the new macro goes on.".into(),
        }
    }

    /// The question while a key is being waited for.
    fn prompt(&self) -> Option<String> {
        let what = match self.capture? {
            Capture::Replace(a) | Capture::Add(a) | Capture::Pad(a) => action_title(a),
            Capture::Macro => "a new macro".into(),
        };
        let device = if self.capturing() {
            "a key"
        } else {
            "a gamepad button"
        };
        Some(format!("Press {device} for {what} (Esc cancels)"))
    }

    /// Draws the window in the middle of the screen and records where
    /// everything went for [`SettingsUi::click`].
    pub fn draw(&mut self, c: &Canvas, r: &mut Renderer, cursor: Option<(f32, f32)>) {
        self.spots.clear();
        let hovered = |rect: Rect| cursor.is_some_and(|(mx, my)| rect.holds(mx, my));
        let w = COLS * 8.0;
        let left_w = LEFT_COLS * 8.0;
        // As many lines as the screen has room for, so a small window scrolls
        // rather than running off the bottom.
        let room = ((c.view_h - 8.0 - TITLE_H - HEAD_H - FOOT_H) / PITCH).floor();
        let lines = (room.max(0.0) as usize).clamp(MIN_LINES, MAX_LINES);
        let body_h = HEAD_H + lines as f32 * PITCH;
        let h = TITLE_H + body_h + FOOT_H;
        let x = ((c.view_w - w) / 2.0).floor().max(4.0);
        let top = ((c.view_h - h) / 2.0).floor().max(4.0);
        c.panel(r, x, top, w, h);
        c.text_center(r, x + w / 2.0, top + 1.0, color::GREEN, "-= Settings =-");
        c.separator(r, x, top + 13.0, w);
        let y0 = top + TITLE_H;

        // --- The sections, down the left ---------------------------------------
        c.text(r, x + 4.0, y0, color::DARK_RED, "Section");
        for (i, s) in Section::ALL.into_iter().enumerate() {
            let y = y0 + HEAD_H + i as f32 * PITCH;
            let rect = Rect::new(x + 2.0, y - 1.0, left_w - 4.0, 13.0);
            self.spots.push((rect, Spot::Section(s)));
            let current = s == self.section;
            if current || hovered(rect) {
                c.color_rect(r, ui_color::HIGHLIGHT, rect.x, rect.y, rect.w, rect.h);
            }
            let colour = if current { color::WHITE } else { color::GREEN };
            c.text(r, x + 4.0, y, colour, s.title());
        }
        c.color_rect(
            r,
            ui_color::BORDER1,
            x + left_w + 3.0,
            y0 - 3.0,
            1.0,
            body_h + 3.0,
        );

        // --- The section's lines, on the right ----------------------------------
        let px = x + left_w + 12.0;
        let pw = w - left_w - 16.0;
        let cols = (pw / 8.0) as usize;
        let vx = px + LABEL_COLS as f32 * 8.0;
        let (left_head, right_head) = self.section.columns();
        c.text(r, px, y0, color::DARK_RED, left_head);
        c.text(r, vx, y0, color::DARK_RED, right_head);
        let mut y = y0 + HEAD_H;
        for line in wrap(self.section.note(), cols, true) {
            if line.is_empty() {
                continue;
            }
            c.text(r, px, y, color::WHITE, &line);
            y += NOTE_PITCH;
        }
        if y > y0 + HEAD_H {
            y += 4.0;
        }
        let rows = self.rows();
        let list_end = y0 + body_h;
        self.visible = (((list_end - y) / PITCH).floor().max(1.0)) as usize;
        self.row = self.row.min(rows.len().saturating_sub(1));
        self.keep_in_view(rows.len());

        // Where the list is, when it is longer than the window: which lines are
        // on show, and the two arrows that move it, on the heading's line.
        if rows.len() > self.visible {
            let last = (self.scroll + self.visible).min(rows.len());
            let at = format!("{}-{} of {}", self.scroll + 1, last, rows.len());
            let right = px + pw;
            let arrows = [
                (right - 8.0, Spot::Scroll(1), "v"),
                (right - 24.0, Spot::Scroll(-1), "^"),
            ];
            for (ax, spot, glyph) in arrows {
                let rect = Rect::new(ax - 3.0, y0 - 1.0, 14.0, 13.0);
                self.spots.push((rect, spot));
                if hovered(rect) {
                    c.color_rect(r, ui_color::HIGHLIGHT, rect.x, rect.y, rect.w, rect.h);
                }
                c.text(r, ax, y0, color::YELLOW, glyph);
            }
            c.text_right(r, right - 40.0, y0, color::DARK_RED, &at);
        }

        for (v, &row) in rows.iter().enumerate().skip(self.scroll).take(self.visible) {
            let ry = y + (v - self.scroll) as f32 * PITCH;
            let rect = Rect::new(px - 2.0, ry - 1.0, pw + 4.0, 13.0);
            self.spots.push((rect, Spot::Row(v)));
            let focused = v == self.row;
            if focused || hovered(rect) {
                c.color_rect(r, ui_color::HIGHLIGHT, rect.x, rect.y, rect.w, rect.h);
            }
            let label_colour = if focused { color::YELLOW } else { color::GREEN };
            match row.kind() {
                Kind::Button => {
                    let text = if focused {
                        format!("[{}]", self.label(row))
                    } else {
                        format!(" {}", self.label(row))
                    };
                    c.text(r, px, ry, color::YELLOW, &text);
                }
                Kind::Toggle => {
                    c.text(r, px, ry, label_colour, &self.label(row));
                    c.text(r, vx, ry, color::WHITE, &self.value(row));
                    if matches!(row, Row::Vsync | Row::Mute) {
                        c.text(r, vx + 6.0 * 8.0, ry, color::GREEN, "(next start)");
                    }
                }
                Kind::Number | Kind::Choice => {
                    c.text(r, px, ry, label_colour, &self.label(row));
                    let more_x = vx + 16.0 + VALUE_COLS as f32 * 8.0;
                    for (ax, dir, glyph) in [(vx, -1, "<"), (more_x, 1, ">")] {
                        let arrow = Rect::new(ax - 3.0, ry - 1.0, 14.0, 13.0);
                        self.spots.push((arrow, Spot::Step(v, dir)));
                        if hovered(arrow) {
                            c.color_rect(r, ui_color::BORDER3, arrow.x, arrow.y, arrow.w, arrow.h);
                        }
                        c.text(r, ax, ry, color::YELLOW, glyph);
                    }
                    c.text(r, vx + 16.0, ry, color::WHITE, &self.value(row));
                }
                Kind::Parts => {
                    c.text(
                        r,
                        px,
                        ry,
                        label_colour,
                        &fit(&self.label(row), LABEL_COLS - 1),
                    );
                    // The buttons from the right edge in, as wide as their words
                    // and a bracket either side, which marks the picked one.
                    let buttons: Vec<(usize, Part)> = row
                        .parts()
                        .iter()
                        .copied()
                        .enumerate()
                        .filter(|(_, p)| *p != Part::Line)
                        .collect();
                    let total: f32 = buttons
                        .iter()
                        .map(|(_, p)| (p.label().len() + 2) as f32 * 8.0)
                        .sum();
                    let mut bx = px + pw - total;
                    let value_room = ((bx - 8.0 - vx) / 8.0).max(1.0) as usize;
                    for (i, part) in buttons {
                        let bw = (part.label().len() + 2) as f32 * 8.0;
                        let button = Rect::new(bx, ry - 1.0, bw, 13.0);
                        self.spots.push((button, Spot::Part(v, i)));
                        if hovered(button) {
                            c.color_rect(r, ui_color::BORDER3, button.x, button.y, button.w, 13.0);
                        }
                        let text = if focused && self.part == i {
                            format!("[{}]", part.label())
                        } else {
                            format!(" {} ", part.label())
                        };
                        c.text(r, bx, ry, color::YELLOW, &text);
                        bx += bw;
                    }
                    let value = self.value(row);
                    if let Row::Macro(_) = row {
                        // The line it sends, which is typed into: the end of it
                        // is what is shown, with the cursor after it.
                        let line_spot =
                            Rect::new(vx - 2.0, ry - 1.0, value_room as f32 * 8.0, 13.0);
                        self.spots.push((line_spot, Spot::Part(v, 0)));
                        let mut shown = value;
                        if focused && self.part == 0 {
                            shown.push('_');
                        }
                        let n = shown.chars().count();
                        let tail: String =
                            shown.chars().skip(n.saturating_sub(value_room)).collect();
                        c.text(r, vx, ry, color::WHITE, &tail);
                    } else {
                        c.text(r, vx, ry, color::WHITE, &fit(&value, value_room));
                    }
                }
            }
        }

        // --- The foot: what is happening, then the keys -------------------------
        let foot = y0 + body_h + 4.0;
        c.separator(r, x, foot, w);
        let width = (w / 8.0 - 1.0) as usize;
        let message = if let Some(p) = self.prompt() {
            Some((p, color::YELLOW))
        } else {
            self.status.clone().map(|s| {
                let colour = if self.warned || s.starts_with("Couldn't") {
                    color::RED
                } else {
                    color::WHITE
                };
                (s, colour)
            })
        };
        // What the pointer is over, or else the picked line: said under the
        // message, or in its place.
        let hint = match cursor.and_then(|(mx, my)| self.spot_at(mx, my)) {
            Some(Spot::Section(s)) => Some(s.hint().to_string()),
            Some(Spot::Row(i) | Spot::Part(i, _) | Spot::Step(i, _)) => {
                rows.get(i).map(|row| self.hint(*row))
            }
            _ => self.current().map(|row| self.hint(row)),
        };
        let mut said: Vec<(String, u32)> = Vec::new();
        if let Some((text, colour)) = message {
            for line in two_lines(&text, width) {
                said.push((line, colour));
            }
        }
        if said.len() < 2
            && let Some(hint) = hint
        {
            for line in wrap(&hint, width, true).into_iter().take(2 - said.len()) {
                said.push((line, color::GREEN));
            }
        }
        for (i, (line, colour)) in said.iter().enumerate() {
            c.text(r, x + 4.0, foot + 6.0 + i as f32 * 12.0, *colour, line);
        }

        // The keys, each a button as well, as on the start-up screen.
        let ky = foot + 36.0;
        let items = [
            ("Tab = Section", Foot::Section),
            ("Enter = Change", Foot::Change),
            ("Ctrl+S = Save", Foot::Save),
            ("Esc = Close", Foot::Close),
        ];
        let total: f32 = items.iter().map(|(t, _)| t.len() as f32 * 8.0).sum::<f32>()
            + 32.0 * (items.len() - 1) as f32;
        let mut kx = (x + w / 2.0 - total / 2.0).floor();
        for (text, foot) in items {
            let tw = text.len() as f32 * 8.0;
            let rect = Rect::new(kx - 2.0, ky - 1.0, tw + 4.0, 13.0);
            self.spots.push((rect, Spot::Foot(foot)));
            if hovered(rect) {
                c.color_rect(r, ui_color::HIGHLIGHT, rect.x, rect.y, rect.w, rect.h);
            }
            c.text(r, kx, ky, color::YELLOW, text);
            kx += tw + 32.0;
        }
    }
}

/// `text` wrapped onto at most two lines of `width`. What does not fit is cut
/// from the middle rather than the end, because the end is what matters in the
/// longest message there is: the file a save went to, whose name is last.
fn two_lines(text: &str, width: usize) -> Vec<String> {
    let mut lines = wrap(text, width, true);
    if lines.len() > 2 {
        let n = text.chars().count();
        let tail: String = text.chars().skip(n + 2 - width.max(3)).collect();
        lines.truncate(1);
        lines.push(format!("..{tail}"));
    }
    lines
}

/// How wide the window is, in characters: the start-up screen's width.
const COLS: f32 = 72.0;
/// The section list's share of it.
const LEFT_COLS: f32 = 15.0;
/// Where a line's value starts, in characters from the left of its pane.
const LABEL_COLS: usize = 15;
/// The room a number or a choice has between its arrows.
const VALUE_COLS: usize = 14;
/// Line spacing, as the start-up screen's profile lines have it.
const PITCH: f32 = 14.0;
/// The spacing of a section's note.
const NOTE_PITCH: f32 = 12.0;
/// The title and the rule under it.
const TITLE_H: f32 = 20.0;
/// A pane's column headings.
const HEAD_H: f32 = 16.0;
/// The rule, two lines of message and the keys.
const FOOT_H: f32 = 56.0;
/// The fewest and the most lines a section shows at once.
const MIN_LINES: usize = 6;
const MAX_LINES: usize = 18;

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(k: &[KeyCode]) -> Vec<Stroke> {
        k.iter().copied().map(Stroke::press).collect()
    }

    fn typed(s: &str) -> Vec<Stroke> {
        s.chars().map(Stroke::text).collect()
    }

    fn press(ui: &mut SettingsUi, k: &[KeyCode]) -> UiAction {
        ui.input(&keys(k), false, false)
    }

    #[test]
    fn capturing_keys() {
        let mut ui = SettingsUi::new(Config::default());
        ui.capture = Some(Capture::Replace(Action::Gun));
        ui.capture(KeyCode::Space, false, true);
        assert_eq!(
            ui.cfg.bindings.get(Action::Gun),
            &[Binding {
                key: KeyCode::Space,
                mods: Mods::Ctrl
            }]
        );
        assert!(!ui.capturing());
        // A modifier key on its own doesn't count as its own modifier.
        ui.capture = Some(Capture::Add(Action::Gun));
        ui.capture(KeyCode::ControlRight, false, true);
        assert_eq!(ui.cfg.bindings.get(Action::Gun).len(), 2);
        assert_eq!(ui.cfg.bindings.get(Action::Gun)[1].mods, Mods::Any);
        // Escape cancels; macros get an empty line to fill in.
        ui.capture = Some(Capture::Macro);
        ui.capture(KeyCode::Escape, false, false);
        assert!(ui.cfg.macros.is_empty());
        ui.capture = Some(Capture::Macro);
        ui.capture(KeyCode::F9, true, false);
        assert_eq!(ui.cfg.macros.len(), 1);
        assert_eq!(ui.cfg.macros[0].0.mods, Mods::Shift);
    }

    /// Tab walks the sections and comes round; Shift+Tab walks them back.
    #[test]
    fn tab_moves_between_the_sections() {
        let mut ui = SettingsUi::new(Config::default());
        assert_eq!(ui.section, Section::Video);
        for want in [
            Section::Audio,
            Section::Accessibility,
            Section::Keys,
            Section::Gamepad,
            Section::Macros,
            Section::Video,
        ] {
            press(&mut ui, &[KeyCode::Tab]);
            assert_eq!(ui.section, want);
        }
        ui.input(&keys(&[KeyCode::Tab]), true, false);
        assert_eq!(ui.section, Section::Macros);
    }

    /// Up and Down go round the lines; a number moves with Left and Right and
    /// stops at its ends, and the pixel scale's bottom is "from the display".
    #[test]
    fn the_arrows_move_and_change() {
        let mut ui = SettingsUi::new(Config::default());
        assert_eq!(ui.current(), Some(Row::Scale));
        press(&mut ui, &[KeyCode::ArrowUp]);
        assert_eq!(ui.current(), Some(Row::Vsync), "Up from the top goes round");
        press(&mut ui, &[KeyCode::ArrowDown]);
        assert_eq!(ui.current(), Some(Row::Scale));

        press(&mut ui, &[KeyCode::ArrowRight, KeyCode::ArrowRight]);
        assert_eq!(ui.cfg.scale, Some(2));
        press(
            &mut ui,
            &[KeyCode::ArrowLeft, KeyCode::ArrowLeft, KeyCode::ArrowLeft],
        );
        assert_eq!(ui.cfg.scale, None, "the bottom is from the display");
        press(&mut ui, &[KeyCode::ArrowRight; 12]);
        assert_eq!(ui.cfg.scale, Some(8), "and the top is the renderer's limit");
        // Enter steps up and goes round, so one key reaches every value.
        press(&mut ui, &[KeyCode::Enter]);
        assert_eq!(ui.cfg.scale, None);
    }

    /// The volume moves in fives and lands on them, whatever the old slider left.
    #[test]
    fn the_volume_moves_in_steps() {
        let mut ui = SettingsUi::new(Config {
            volume: 63,
            ..Config::default()
        });
        press(&mut ui, &[KeyCode::Tab, KeyCode::ArrowDown]);
        assert_eq!(ui.current(), Some(Row::Volume));
        press(&mut ui, &[KeyCode::ArrowRight]);
        assert_eq!(ui.cfg.volume, 65);
        press(&mut ui, &[KeyCode::ArrowLeft, KeyCode::ArrowLeft]);
        assert_eq!(ui.cfg.volume, 55);
        press(&mut ui, &[KeyCode::ArrowRight; 30]);
        assert_eq!(ui.cfg.volume, 100);
        press(&mut ui, &[KeyCode::ArrowLeft; 30]);
        assert_eq!(ui.cfg.volume, 0);
    }

    /// Enter and the arrows flip a switch; the palette cycles through every
    /// one there is.
    #[test]
    fn switches_flip_and_the_palette_cycles() {
        let mut ui = SettingsUi::new(Config::default());
        press(&mut ui, &[KeyCode::ArrowDown, KeyCode::Enter]);
        assert!(ui.cfg.fullscreen);
        press(&mut ui, &[KeyCode::ArrowRight]);
        assert!(!ui.cfg.fullscreen);

        press(&mut ui, &[KeyCode::Tab, KeyCode::Tab]);
        assert_eq!(ui.current(), Some(Row::Palette));
        let mut seen = vec![ui.cfg.palette];
        for _ in 1..PaletteKind::ALL.len() {
            press(&mut ui, &[KeyCode::Enter]);
            seen.push(ui.cfg.palette);
        }
        assert_eq!(seen, PaletteKind::ALL);
        press(&mut ui, &[KeyCode::Enter]);
        assert_eq!(ui.cfg.palette, PaletteKind::Standard, "and comes round");
        press(&mut ui, &[KeyCode::ArrowDown, KeyCode::ArrowRight]);
        assert_eq!(ui.cfg.ui_scale, 1);
    }

    /// A key is rebound from the keyboard alone: Enter on its line waits for a
    /// key, which becomes its only one; Right then Enter adds a second; Clear
    /// clears them.
    #[test]
    fn a_key_is_rebound_from_the_keyboard() {
        let mut ui = SettingsUi::new(Config::default());
        press(&mut ui, &[KeyCode::Tab, KeyCode::Tab, KeyCode::Tab]);
        let gun = Action::ALL.iter().position(|a| *a == Action::Gun).unwrap();
        for _ in 0..gun {
            press(&mut ui, &[KeyCode::ArrowDown]);
        }
        assert_eq!(ui.current(), Some(Row::Key(Action::Gun)));

        press(&mut ui, &[KeyCode::Enter]);
        assert!(ui.capturing());
        ui.input(&keys(&[KeyCode::KeyJ]), false, false);
        assert!(!ui.capturing());
        assert_eq!(ui.cfg.bindings.get(Action::Gun).len(), 1);
        assert_eq!(ui.cfg.bindings.get(Action::Gun)[0].key, KeyCode::KeyJ);
        assert_eq!(ui.status.as_deref(), Some("Bound J"));

        press(&mut ui, &[KeyCode::ArrowRight, KeyCode::Enter]);
        ui.input(&keys(&[KeyCode::KeyK]), true, false);
        assert_eq!(ui.cfg.bindings.get(Action::Gun).len(), 2);
        assert_eq!(ui.cfg.bindings.get(Action::Gun)[1].mods, Mods::Shift);

        press(&mut ui, &[KeyCode::ArrowRight, KeyCode::Enter]);
        assert!(ui.cfg.bindings.get(Action::Gun).is_empty());

        // The last line puts them all back.
        press(&mut ui, &[KeyCode::End, KeyCode::Enter]);
        assert_eq!(ui.cfg.bindings, Config::default().bindings);
    }

    /// Esc during a gamepad capture stops the capture, not the window.
    #[test]
    fn escape_cancels_a_gamepad_capture_first() {
        let mut ui = SettingsUi::new(Config::default());
        press(&mut ui, &[KeyCode::Tab; 4]);
        assert_eq!(ui.section, Section::Gamepad);
        press(&mut ui, &[KeyCode::Enter]);
        assert!(ui.capture.is_some());
        assert_eq!(press(&mut ui, &[KeyCode::Escape]), UiAction::None);
        assert!(ui.capture.is_none());
        assert!(!ui.capture_pad(PadButton::South));
        assert_eq!(press(&mut ui, &[KeyCode::Escape]), UiAction::Close);
    }

    /// A macro is made, typed, edited and removed with the keyboard alone, and
    /// the cursor is put on its line as soon as its key is pressed.
    #[test]
    fn a_macro_is_made_and_typed_from_the_keyboard() {
        let mut ui = SettingsUi::new(Config::default());
        ui.input(&keys(&[KeyCode::Tab]), true, false);
        assert_eq!(ui.current(), Some(Row::AddMacro));
        press(&mut ui, &[KeyCode::Enter]);
        assert!(ui.capturing());
        press(&mut ui, &[KeyCode::F9]);
        assert_eq!(ui.current(), Some(Row::Macro(0)));
        ui.input(&typed("'help at base!"), false, false);
        press(&mut ui, &[KeyCode::Backspace]);
        assert_eq!(ui.cfg.macros[0].1, "'help at base");
        // Ctrl with a letter is a command, not typing.
        let saved = ui.input(&[Stroke::press(KeyCode::KeyS).typing('s')], false, true);
        assert_eq!(saved, UiAction::Save);
        assert_eq!(ui.cfg.macros[0].1, "'help at base");

        press(&mut ui, &[KeyCode::ArrowRight, KeyCode::Enter]);
        assert!(ui.cfg.macros.is_empty());
        assert_eq!(ui.current(), Some(Row::AddMacro));
    }

    /// Typing does nothing on a line that does not take text.
    #[test]
    fn typing_changes_nothing_elsewhere() {
        let mut ui = SettingsUi::new(Config::default());
        let before = ui.cfg.clone();
        for _ in Section::ALL {
            ui.input(&typed("qwerty 123"), false, false);
            press(&mut ui, &[KeyCode::Tab]);
        }
        assert_eq!(ui.cfg, before);
    }

    /// Unsaved changes are not lost to one Esc: the first says so, the second
    /// closes. Saved, or unchanged, one Esc closes.
    #[test]
    fn closing_warns_once_about_unsaved_changes() {
        let mut ui = SettingsUi::new(Config::default());
        assert_eq!(press(&mut ui, &[KeyCode::Escape]), UiAction::Close);

        press(&mut ui, &[KeyCode::ArrowDown, KeyCode::Enter]);
        assert_eq!(press(&mut ui, &[KeyCode::Escape]), UiAction::None);
        assert!(
            ui.status
                .as_deref()
                .is_some_and(|s| s.contains("Not saved"))
        );
        assert_eq!(press(&mut ui, &[KeyCode::Escape]), UiAction::Close);

        ui.warned = false;
        ui.mark_saved();
        assert_eq!(press(&mut ui, &[KeyCode::Escape]), UiAction::Close);
    }

    /// A long list keeps the picked line in view, and the wheel carries the
    /// picked line with it rather than scrolling it out of the window.
    #[test]
    fn a_long_list_scrolls_with_the_picked_line() {
        let mut ui = SettingsUi::new(Config::default());
        press(&mut ui, &[KeyCode::Tab, KeyCode::Tab, KeyCode::Tab]);
        ui.visible = 10;
        let len = ui.rows().len();
        assert!(len > 30);
        press(&mut ui, &[KeyCode::End]);
        ui.keep_in_view(len);
        assert_eq!(ui.scroll, len - 10);
        press(&mut ui, &[KeyCode::Home]);
        ui.keep_in_view(len);
        assert_eq!(ui.scroll, 0);
        ui.scroll(3);
        assert_eq!((ui.scroll, ui.row), (3, 3), "the picked line came along");
        ui.scroll(1000);
        assert_eq!(ui.scroll, len - 10);
        ui.scroll(-1000);
        assert_eq!(ui.scroll, 0);
        assert_eq!(ui.row, 9, "still on the list");
    }

    /// Everything the keys do, a click does. The regions are the ones the
    /// window records as it draws, set here by hand because drawing needs a
    /// window.
    #[test]
    fn the_pointer_does_what_the_keys_do() {
        let mut ui = SettingsUi::new(Config::default());
        ui.spots = vec![
            (
                Rect::new(0.0, 0.0, 100.0, 13.0),
                Spot::Section(Section::Audio),
            ),
            (Rect::new(0.0, 20.0, 100.0, 13.0), Spot::Row(0)),
            (Rect::new(0.0, 34.0, 100.0, 13.0), Spot::Row(1)),
            (Rect::new(120.0, 34.0, 14.0, 13.0), Spot::Step(1, 1)),
            (Rect::new(0.0, 60.0, 40.0, 13.0), Spot::Foot(Foot::Save)),
            (Rect::new(50.0, 60.0, 40.0, 13.0), Spot::Foot(Foot::Close)),
            (
                Rect::new(100.0, 60.0, 40.0, 13.0),
                Spot::Foot(Foot::Section),
            ),
        ];
        ui.click(10.0, 5.0);
        assert_eq!(ui.section, Section::Audio);
        ui.click(10.0, 25.0);
        assert!(ui.cfg.mute, "a click on a switch flips it");
        ui.click(10.0, 40.0);
        assert_eq!(ui.current(), Some(Row::Volume));
        assert_eq!(
            ui.cfg.volume, 60,
            "a click on a number's line only picks it"
        );
        ui.click(125.0, 40.0);
        assert_eq!(ui.cfg.volume, 65, "its arrow moves it");
        assert_eq!(ui.click(10.0, 65.0), UiAction::Save);
        ui.mark_saved();
        ui.click(110.0, 65.0);
        assert_eq!(ui.section, Section::Accessibility);
        assert_eq!(ui.click(60.0, 65.0), UiAction::Close);
        assert_eq!(ui.click(500.0, 500.0), UiAction::None, "outside everything");
    }

    /// A button along a line is pressed by clicking it, and a click while a
    /// key is awaited stops the wait rather than doing anything else.
    #[test]
    fn a_click_presses_a_button_or_stops_a_capture() {
        let mut ui = SettingsUi::new(Config::default());
        press(&mut ui, &[KeyCode::Tab, KeyCode::Tab, KeyCode::Tab]);
        ui.spots = vec![
            (Rect::new(0.0, 0.0, 200.0, 13.0), Spot::Row(0)),
            (Rect::new(100.0, 0.0, 40.0, 13.0), Spot::Part(0, 0)),
            (Rect::new(150.0, 0.0, 40.0, 13.0), Spot::Part(0, 2)),
        ];
        ui.click(160.0, 5.0);
        assert!(ui.cfg.bindings.get(Action::ALL[0]).is_empty(), "Clear");
        ui.click(110.0, 5.0);
        assert!(ui.capturing(), "Change");
        ui.click(160.0, 5.0);
        assert!(!ui.capturing());
        assert_eq!(ui.status.as_deref(), Some("Cancelled"));
        assert!(ui.cfg.bindings.get(Action::ALL[0]).is_empty());
    }

    /// A message too long for two lines keeps its end, where a saved file's
    /// name is.
    #[test]
    fn a_long_message_keeps_its_end() {
        assert_eq!(two_lines("Saved", 20), vec!["Saved"]);
        let long = format!("Saved to /{}/oss-client.ini", "x".repeat(60));
        let lines = two_lines(&long, 30);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].starts_with(".."));
        assert!(lines[1].ends_with("oss-client.ini"));
        assert_eq!(lines[1].chars().count(), 30);
    }

    #[test]
    fn labels_read_as_words() {
        assert_eq!(action_title(Action::StatBoxUp), "Statbox up");
        assert_eq!(action_title(Action::Gun), "Gun");
        assert_eq!(fit("abcdef", 6), "abcdef");
        assert_eq!(fit("abcdefg", 6), "abcd..");
        let shift_up = Binding {
            key: KeyCode::ArrowUp,
            mods: Mods::Shift,
        };
        assert_eq!(binding_label(&shift_up), "Shift+Up");
    }
}
