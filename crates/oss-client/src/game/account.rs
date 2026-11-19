//! The boxes a zone can ask a player to answer while logging in.
//!
//! A zone that does not know the name it was given offers to create it
//! ([`oss_session::SessionEvent::NewUserRequired`]). The real clients ask the
//! player before saying yes, and so does this one: a box over the connecting
//! screen, drawn the way the start-up screen is, asking Continuum's own
//! question in Continuum's own words, with Yes and No. Yes creates the account
//! with the name and password the login was sent with, as Continuum does; the
//! password is not asked for a second time, so a mistyped one becomes the
//! account's, as it does in Continuum.
//!
//! A zone whose billing server wants a registration form says so in its login
//! reply ([`oss_session::SessionEvent::RegistrationWanted`]). The box asks what
//! Continuum's *Player Information* dialog asks, in its order: the e-mail
//! address first and required, then real name, city, state or country and age,
//! all optional; where the player connects from, as three tick boxes; and sex,
//! as two round buttons with neither chosen until the player chooses. The
//! original clients sent a description of the computer with it. This one does
//! not, and the box says plainly that only what is typed into it is sent.
//!
//! The form has Continuum's two ways out and no third: Submit Information, or
//! Disconnect from server, which closes the connection and goes back to the
//! start-up screen. There is no way into the zone without answering, as there
//! was none in Continuum or in SubSpace before it.
//!
//! Both boxes work fully from the keyboard and fully from the pointer: every
//! line can be clicked, every tick box and round button can be clicked, and
//! each key on the foot line is a button as well, as the start-up screen's
//! are.

use std::fmt;

use oss_session::{RegistrationForm, Sex};
use winit::keyboard::KeyCode;

use super::input::Stroke;
use super::start_spots::Rect;
use super::view::{Canvas, color, ui_color};
use crate::gfx::Renderer;

/// What the player answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Create the account.
    Create,
    /// Do not create it; go back.
    Cancel,
    /// Send the registration form with these answers.
    Send(RegistrationForm),
    /// Do not send the form: leave the zone.
    Disconnect,
}

/// Which question the box is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    NewAccount,
    Registration,
}

/// Something in the box a pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    /// A line that can be picked, by its index.
    Line(usize),
    /// One tick box or round button: the line, and which along it.
    Option(usize, usize),
    /// The foot line's yes ("Enter = ...").
    Yes,
    /// The foot line's no ("Esc = ...").
    No,
}

/// One line of the box, top to bottom.
#[derive(Clone, PartialEq, Eq)]
enum Line {
    /// Words already wrapped to the box, in a colour; `indent` starts them
    /// under the values rather than under the labels.
    Note {
        text: String,
        color: u32,
        indent: bool,
    },
    /// Half a line of space.
    Gap,
    /// A line that takes text.
    Field(Field),
    /// A row of tick boxes or round buttons.
    Choice(Choice),
}

/// A line that takes text.
#[derive(Clone, PartialEq, Eq)]
struct Field {
    label: &'static str,
    value: String,
    /// The most characters the field on the wire holds.
    max: usize,
    /// Takes digits only.
    digits: bool,
    /// Marked "(required)", as Continuum marks its e-mail.
    required: bool,
}

/// A row of tick boxes, any number of which may be ticked, or of round
/// buttons, of which one at most is chosen.
#[derive(Clone, PartialEq, Eq)]
struct Choice {
    label: &'static str,
    options: &'static [&'static str],
    set: Vec<bool>,
    /// Round buttons: choosing one clears the rest, and once one is chosen
    /// there is always one, as with Windows' own.
    round: bool,
    /// The one the keyboard is on.
    at: usize,
}

impl Choice {
    fn new(label: &'static str, options: &'static [&'static str], round: bool) -> Self {
        Self {
            label,
            options,
            set: vec![false; options.len()],
            round,
            at: 0,
        }
    }

    /// Space, or a click, on option `i`.
    fn press(&mut self, i: usize) {
        if i >= self.set.len() {
            return;
        }
        self.at = i;
        if self.round {
            self.set.iter_mut().for_each(|s| *s = false);
            self.set[i] = true;
        } else {
            self.set[i] = !self.set[i];
        }
    }

    /// How an option reads: `[x] Home`, `( ) Male`.
    fn option_text(&self, i: usize) -> String {
        let on = self.set.get(i).copied().unwrap_or(false);
        let mark = match (self.round, on) {
            (false, true) => "[x]",
            (false, false) => "[ ]",
            (true, true) => "(*)",
            (true, false) => "( )",
        };
        format!("{mark} {}", self.options.get(i).copied().unwrap_or(""))
    }
}

/// Continuum's question, word for word.
const NEW_USER_QUESTION: &str =
    "Unknown name. Would you like to create a new user and enter the game using this name?";

/// How wide the new-account box is, in characters: Continuum's question on two
/// lines.
const NEW_USER_COLUMNS: usize = 52;

/// How wide the registration form is, in characters: a label, a field and the
/// "(required)" after the e-mail.
const FORM_COLUMNS: usize = 62;

/// The form's top paragraph. Continuum's says a hardware profile goes with
/// the answers; this client sends none, so this one says what is sent instead.
const FORM_INTRO: &str = "This zone's player database asks for the following information. \
     Only what you type or choose below is sent; nothing about this computer is.";

/// Continuum's line under its e-mail field, word for word.
const EMAIL_WHY: &str = "E-Mail address is required in case you lose your password or there \
     are any disputes about the ownership of the account.";

/// How the keys work, under the fields.
const FORM_KEYS: &str = "Tab, Up and Down move between lines. On a row of boxes, Left and \
     Right move along it and Space ticks or chooses.";

/// The form's labels, which are also how its answers are found.
const EMAIL: &str = "E-Mail";
const REAL_NAME: &str = "Real Name";
const CITY: &str = "City";
const STATE: &str = "State/Country";
const AGE: &str = "Age";
const CONNECT_FROM: &str = "Connect From";
const SEX: &str = "Sex";

/// The form's text fields' lengths on the wire, less their NULs.
const EMAIL_MAX: usize = 63;
const REAL_NAME_MAX: usize = 31;
const CITY_MAX: usize = 31;
const STATE_MAX: usize = 23;
/// Age is one byte: up to three digits, checked against 255 when sent.
const AGE_MAX: usize = 3;

/// The box, its lines and where the pointer can find them.
///
/// `Debug` is written out rather than derived so what a player typed into it
/// cannot be printed.
pub struct Prompt {
    kind: Kind,
    title: &'static str,
    /// How wide the box asks to be, in characters.
    columns: usize,
    lines: Vec<Line>,
    /// The picked line, by its index; always a field or a choice when the box
    /// has any.
    focus: usize,
    /// The two keys on the foot line, as they are drawn.
    yes: &'static str,
    no: &'static str,
    /// What went wrong with the last answer, in red under the lines.
    pub error: Option<String>,
    /// Where everything was drawn last frame, so a click finds it.
    spots: Vec<(Rect, Spot)>,
}

impl fmt::Debug for Prompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Prompt")
            .field("title", &self.title)
            .field("focus", &self.focus)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// Words wrapped to `width`, one [`Line::Note`] each.
fn notes(text: &str, width: usize, color: u32, indent: bool) -> Vec<Line> {
    super::chat::wrap(text, width, true)
        .into_iter()
        .map(|text| Line::Note {
            text,
            color,
            indent,
        })
        .collect()
}

impl Prompt {
    /// The offer to create the name the zone said it does not know, asked as
    /// Continuum asks it. `zone_text` is anything the zone said with the
    /// offer, shown under the question when there is some.
    pub fn new_account(zone_text: Option<&str>) -> Self {
        let width = NEW_USER_COLUMNS - 2;
        let mut lines = notes(NEW_USER_QUESTION, width, color::WHITE, false);
        if let Some(text) = zone_text.filter(|t| !t.trim().is_empty()) {
            lines.push(Line::Gap);
            lines.extend(notes(
                &format!("The zone says: {}", text.trim()),
                width,
                color::WHITE,
                false,
            ));
        }
        Self {
            kind: Kind::NewAccount,
            title: "-= Information =-",
            columns: NEW_USER_COLUMNS,
            lines,
            focus: 0,
            yes: "Enter = Yes",
            no: "Esc = No",
            error: None,
            spots: Vec::new(),
        }
    }

    /// The zone's registration form: Continuum's *Player Information*.
    pub fn registration() -> Self {
        let field = |label, max, digits, required| {
            Line::Field(Field {
                label,
                value: String::new(),
                max,
                digits,
                required,
            })
        };
        let width = FORM_COLUMNS - 2;
        let mut lines = notes(FORM_INTRO, width, color::WHITE, false);
        lines.push(Line::Gap);
        lines.push(field(EMAIL, EMAIL_MAX, false, true));
        lines.extend(notes(EMAIL_WHY, width - LABEL_COLUMNS, color::WHITE, true));
        lines.push(Line::Gap);
        lines.push(field(REAL_NAME, REAL_NAME_MAX, false, false));
        lines.push(field(CITY, CITY_MAX, false, false));
        lines.push(field(STATE, STATE_MAX, false, false));
        lines.push(field(AGE, AGE_MAX, true, false));
        lines.push(Line::Choice(Choice::new(
            CONNECT_FROM,
            &["Home", "Work", "School"],
            false,
        )));
        lines.push(Line::Choice(Choice::new(SEX, &["Male", "Female"], true)));
        lines.push(Line::Gap);
        lines.extend(notes(FORM_KEYS, width, color::GREEN, false));
        let mut p = Self {
            kind: Kind::Registration,
            title: "-= Player Information =-",
            columns: FORM_COLUMNS,
            lines,
            focus: 0,
            yes: "Enter = Submit Information",
            no: "Esc = Disconnect from server",
            error: None,
            spots: Vec::new(),
        };
        p.focus = p.find(EMAIL);
        p
    }

    /// The index of the field or choice labelled `label`.
    fn find(&self, label: &str) -> usize {
        self.lines
            .iter()
            .position(|l| match l {
                Line::Field(f) => f.label == label,
                Line::Choice(c) => c.label == label,
                _ => false,
            })
            .unwrap_or(0)
    }

    /// What was typed into the field labelled `label`, trimmed.
    fn text(&self, label: &str) -> String {
        match self.lines.get(self.find(label)) {
            Some(Line::Field(f)) => f.value.trim().to_string(),
            _ => String::new(),
        }
    }

    /// Which of the choice labelled `label`'s options are set.
    fn set(&self, label: &str) -> Vec<bool> {
        match self.lines.get(self.find(label)) {
            Some(Line::Choice(c)) => c.set.clone(),
            _ => Vec::new(),
        }
    }

    /// Whether line `i` can be picked.
    fn pickable(&self, i: usize) -> bool {
        matches!(self.lines.get(i), Some(Line::Field(_) | Line::Choice(_)))
    }

    /// Moves the pick to the next line that takes one, forwards or back,
    /// round from the end to the start.
    fn step_focus(&mut self, back: bool) {
        let n = self.lines.len();
        for k in 1..=n {
            let i = if back {
                (self.focus + n * 2 - k) % n
            } else {
                (self.focus + k) % n
            };
            if self.pickable(i) {
                self.focus = i;
                return;
            }
        }
    }

    /// Applies one frame of keystrokes.
    ///
    /// Typing goes into the picked field and Backspace (held or not) takes
    /// from it. On a row of tick boxes or round buttons, Left and Right move
    /// along it and Space ticks or chooses. Tab, Shift-Tab and Up and Down
    /// move between lines. Enter answers yes and Esc no. The new-account
    /// question, which has nothing to type in, also takes Y and N, as the
    /// buttons of the Windows message box it copies do.
    pub fn input(&mut self, strokes: &[Stroke], shift: bool) -> Option<Answer> {
        for stroke in strokes {
            if self.kind == Kind::NewAccount && !stroke.repeat {
                match stroke.text.map(|c| c.to_ascii_lowercase()) {
                    Some('y') => return self.yes(),
                    Some('n') => return Some(self.no()),
                    _ => {}
                }
            }
            match self.lines.get_mut(self.focus) {
                Some(Line::Field(f)) => {
                    if let Some(c) = stroke.text
                        && !c.is_control()
                        && (!f.digits || c.is_ascii_digit())
                        && f.value.chars().count() < f.max
                    {
                        f.value.push(c);
                    }
                    if stroke.key == Some(KeyCode::Backspace) {
                        f.value.pop();
                    }
                }
                Some(Line::Choice(c)) => {
                    let space = match stroke.key {
                        Some(KeyCode::Space) => !stroke.repeat,
                        Some(_) => false,
                        None => stroke.text == Some(' '),
                    };
                    if space {
                        c.press(c.at);
                    }
                    match stroke.key {
                        Some(KeyCode::ArrowLeft) => c.at = c.at.saturating_sub(1),
                        Some(KeyCode::ArrowRight) => {
                            c.at = (c.at + 1).min(c.options.len().saturating_sub(1));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            match stroke.key {
                Some(KeyCode::Tab) => self.step_focus(shift),
                Some(KeyCode::ArrowUp) => self.step_focus(true),
                Some(KeyCode::ArrowDown) => self.step_focus(false),
                _ => {}
            }
            match stroke.first_press() {
                Some(KeyCode::Enter | KeyCode::NumpadEnter) => {
                    if let Some(answer) = self.yes() {
                        return Some(answer);
                    }
                }
                Some(KeyCode::Escape) => return Some(self.no()),
                _ => {}
            }
        }
        None
    }

    /// A click at (`x`, `y`) in UI pixels, against what the last frame drew.
    pub fn click(&mut self, x: f32, y: f32) -> Option<Answer> {
        match self.spot_at(x, y)? {
            Spot::Line(i) => {
                if self.pickable(i) {
                    self.focus = i;
                }
                None
            }
            Spot::Option(i, k) => {
                if let Some(Line::Choice(c)) = self.lines.get_mut(i) {
                    c.press(k);
                    self.focus = i;
                }
                None
            }
            Spot::Yes => self.yes(),
            Spot::No => Some(self.no()),
        }
    }

    fn no(&self) -> Answer {
        match self.kind {
            Kind::NewAccount => Answer::Cancel,
            Kind::Registration => Answer::Disconnect,
        }
    }

    /// The yes, if the box's lines allow it; otherwise says why not.
    fn yes(&mut self) -> Option<Answer> {
        match self.kind {
            Kind::NewAccount => Some(Answer::Create),
            Kind::Registration => self.send(),
        }
    }

    /// Refuses the form: says why in red and picks the line to fix.
    fn refuse(&mut self, label: &str, why: &str) -> Option<Answer> {
        self.focus = self.find(label);
        self.error = Some(why.to_string());
        None
    }

    /// The registration form's yes. The e-mail is the one thing required, as
    /// it is the one thing Continuum marks required, and it has to look like
    /// an address. Everything else may be left empty or unchosen, Sex
    /// included. The age, when given, has to fit the one byte it is sent in.
    fn send(&mut self) -> Option<Answer> {
        let email = self.text(EMAIL);
        if email.is_empty() {
            return self.refuse(
                EMAIL,
                "E-Mail is required. Enter one, or press Esc to disconnect.",
            );
        }
        if !looks_like_email(&email) {
            return self.refuse(EMAIL, "That does not look like an e-mail address.");
        }
        let age = self.text(AGE);
        let age = if age.is_empty() {
            0
        } else {
            match age.parse::<u8>() {
                Ok(n) => n,
                Err(_) => return self.refuse(AGE, "Age is a number from 0 to 255."),
            }
        };
        let sex = match self.set(SEX).as_slice() {
            [true, _] => Some(Sex::Male),
            [_, true] => Some(Sex::Female),
            _ => None,
        };
        let from = self.set(CONNECT_FROM);
        let from = |i: usize| from.get(i).copied().unwrap_or(false);
        self.error = None;
        Some(Answer::Send(RegistrationForm {
            real_name: self.text(REAL_NAME),
            email,
            city: self.text(CITY),
            state: self.text(STATE),
            age,
            from_home: from(0),
            from_work: from(1),
            from_school: from(2),
            sex,
        }))
    }

    fn spot_at(&self, x: f32, y: f32) -> Option<Spot> {
        self.spots
            .iter()
            .rev()
            .find(|(r, _)| r.holds(x, y))
            .map(|(_, s)| *s)
    }

    /// The box's width in characters: what it asks for, or more if the title
    /// or the foot line need it.
    fn columns(&self) -> usize {
        let foot = self.yes.len() + 4 + self.no.len() + 2;
        self.columns.max(self.title.len() + 2).max(foot)
    }

    /// How tall a line is drawn, in UI pixels.
    fn pitch(line: &Line) -> f32 {
        match line {
            Line::Gap => 7.0,
            _ => 14.0,
        }
    }

    /// Draws the box in the middle of the screen, in the start-up screen's
    /// style: green labels, white values, the game's grey border, and the keys
    /// on a yellow foot line that are buttons as well. Records where
    /// everything went for [`Prompt::click`].
    pub fn draw(&mut self, c: &Canvas, r: &mut Renderer, cursor: Option<(f32, f32)>) {
        self.spots.clear();
        let hovered = |rect: Rect| cursor.is_some_and(|(mx, my)| rect.holds(mx, my));
        let cols = self
            .columns()
            .min(((c.view_w - 16.0) / 8.0).max(20.0) as usize);
        let w = cols as f32 * 8.0;
        let body: f32 = self.lines.iter().map(Self::pitch).sum();
        // Title and rule, the lines, a gap, two error lines, a rule, the keys.
        let h = 20.0 + body + 8.0 + 2.0 * ERROR_PITCH + 8.0 + 16.0;
        let x = ((c.view_w - w) / 2.0).floor();
        let top = ((c.view_h - h) / 2.0).floor().max(4.0);
        c.panel(r, x, top, w, h);
        c.text_center(r, x + w / 2.0, top + 1.0, color::GREEN, self.title);
        c.separator(r, x, top + 13.0, w);

        let mut y = top + 20.0;
        let width = cols.saturating_sub(1);
        let value_x = x + 4.0 + LABEL_COLUMNS as f32 * 8.0;
        let room = FIELD_COLUMNS.min(cols.saturating_sub(LABEL_COLUMNS + 1).max(1));
        for (i, line) in self.lines.iter().enumerate() {
            let row = Rect::new(x + 2.0, y - 1.0, w - 4.0, 13.0);
            let picked = self.focus == i;
            match line {
                Line::Note {
                    text,
                    color,
                    indent,
                } => {
                    let (lx, room) = if *indent {
                        (value_x, width.saturating_sub(LABEL_COLUMNS))
                    } else {
                        (x + 4.0, width)
                    };
                    let shown: String = text.chars().take(room).collect();
                    c.text(r, lx, y, *color, &shown);
                }
                Line::Gap => {}
                Line::Field(f) => {
                    self.spots.push((row, Spot::Line(i)));
                    if picked || hovered(row) {
                        c.color_rect(r, ui_color::HIGHLIGHT, row.x, row.y, row.w, row.h);
                    }
                    c.text(r, x + 4.0, y, color::GREEN, f.label);
                    let mut value = f.value.clone();
                    if picked {
                        value.push('_');
                    }
                    // A long value scrolls: its end, where the typing is, shows.
                    let n = value.chars().count();
                    let shown: String = value.chars().skip(n.saturating_sub(room)).collect();
                    c.text(r, value_x, y, color::WHITE, &shown);
                    if f.required {
                        let rx = value_x + (room + 1) as f32 * 8.0;
                        c.text(r, rx, y, color::DARK_RED, "(required)");
                    }
                }
                Line::Choice(ch) => {
                    self.spots.push((row, Spot::Line(i)));
                    if hovered(row) && !picked {
                        c.color_rect(r, ui_color::HIGHLIGHT, row.x, row.y, row.w, row.h);
                    }
                    c.text(r, x + 4.0, y, color::GREEN, ch.label);
                    let mut ox = value_x;
                    for k in 0..ch.options.len() {
                        let text = ch.option_text(k);
                        let tw = text.chars().count() as f32 * 8.0;
                        let rect = Rect::new(ox - 2.0, y - 1.0, tw + 4.0, 13.0);
                        self.spots.push((rect, Spot::Option(i, k)));
                        // The one the keyboard is on is lit while the row is
                        // picked, so Space visibly has a target.
                        if (picked && ch.at == k) || hovered(rect) {
                            c.color_rect(r, ui_color::HIGHLIGHT, rect.x, rect.y, rect.w, rect.h);
                        }
                        c.text(r, ox, y, color::WHITE, &text);
                        ox += tw + 16.0;
                    }
                }
            }
            y += Self::pitch(line);
        }
        y += 8.0;
        // Wrapped onto two lines rather than cut at the edge, which lost the
        // half of the sentence that says what to do.
        if let Some(err) = &self.error {
            for (i, line) in super::chat::wrap(err, width, true)
                .iter()
                .take(2)
                .enumerate()
            {
                c.text(r, x + 4.0, y + i as f32 * ERROR_PITCH, color::RED, line);
            }
        }
        y += 2.0 * ERROR_PITCH + 4.0;
        c.separator(r, x, y, w);

        // The foot line: both keys are buttons, as on the start-up screen.
        let ky = y + 6.0;
        let items = [(self.yes, Spot::Yes), (self.no, Spot::No)];
        let text_w: f32 = items.iter().map(|(t, _)| t.len() as f32 * 8.0).sum();
        let gap = ((w - text_w) / 3.0).clamp(16.0, 64.0).floor();
        let mut kx = (x + w / 2.0 - (text_w + gap) / 2.0).floor();
        for (text, spot) in items {
            let tw = text.len() as f32 * 8.0;
            let rect = Rect::new(kx - 2.0, ky - 1.0, tw + 4.0, 13.0);
            self.spots.push((rect, spot));
            if hovered(rect) {
                c.color_rect(r, ui_color::HIGHLIGHT, rect.x, rect.y, rect.w, rect.h);
            }
            c.text(r, kx, ky, color::YELLOW, text);
            kx += tw + gap;
        }
    }
}

/// Where the values start, in characters from the box's left edge: room for
/// the longest label, "State/Country", and two spaces.
const LABEL_COLUMNS: usize = 15;

/// The most columns a field is drawn across; longer values scroll.
const FIELD_COLUMNS: usize = 32;

/// The spacing of the error lines, which the start-up screen's foot uses too.
const ERROR_PITCH: f32 = 12.0;

/// Something, an `@`, something with a dot in it, and no spaces: enough to
/// catch a name typed into the wrong line, without claiming to validate an
/// address, which only sending to it can do.
fn looks_like_email(s: &str) -> bool {
    match s.split_once('@') {
        Some((user, host)) => {
            !user.is_empty()
                && !host.contains('@')
                && host.contains('.')
                && !host.starts_with('.')
                && !host.ends_with('.')
                && !s.chars().any(char::is_whitespace)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> Vec<Stroke> {
        s.chars().map(Stroke::text).collect()
    }

    fn keys(k: &[KeyCode]) -> Vec<Stroke> {
        k.iter().copied().map(Stroke::press).collect()
    }

    /// Space as a keyboard sends it: the key, with the character on it.
    fn space() -> Vec<Stroke> {
        vec![Stroke {
            key: Some(KeyCode::Space),
            text: Some(' '),
            repeat: false,
        }]
    }

    fn note_text(p: &Prompt) -> String {
        p.lines
            .iter()
            .filter_map(|l| match l {
                Line::Note { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The labels of the lines that can be picked, top to bottom.
    fn labels(p: &Prompt) -> Vec<&'static str> {
        p.lines
            .iter()
            .filter_map(|l| match l {
                Line::Field(f) => Some(f.label),
                Line::Choice(c) => Some(c.label),
                _ => None,
            })
            .collect()
    }

    fn focused(p: &Prompt) -> &'static str {
        match &p.lines[p.focus] {
            Line::Field(f) => f.label,
            Line::Choice(c) => c.label,
            _ => "",
        }
    }

    /// The offer is Continuum's question, word for word, and nothing to type:
    /// the account is made with the password the login was sent with.
    #[test]
    fn the_offer_asks_continuums_question_and_nothing_else() {
        let p = Prompt::new_account(None);
        assert_eq!(note_text(&p), NEW_USER_QUESTION);
        assert!(labels(&p).is_empty(), "no second password");
        assert_eq!((p.yes, p.no), ("Enter = Yes", "Esc = No"));

        let p = Prompt::new_account(Some("  Welcome!  "));
        assert!(note_text(&p).ends_with("The zone says: Welcome!"));
    }

    /// Enter or Y says yes; Esc or N says no. Nothing typed gets in the way.
    #[test]
    fn the_keys_answer_yes_or_no() {
        for (k, want) in [
            (KeyCode::Enter, Answer::Create),
            (KeyCode::NumpadEnter, Answer::Create),
            (KeyCode::Escape, Answer::Cancel),
        ] {
            let mut p = Prompt::new_account(None);
            assert_eq!(p.input(&keys(&[k]), false), Some(want));
        }
        for (s, want) in [
            ("y", Answer::Create),
            ("Y", Answer::Create),
            ("n", Answer::Cancel),
            ("N", Answer::Cancel),
        ] {
            let mut p = Prompt::new_account(None);
            assert_eq!(p.input(&typed(s), false), Some(want), "{s}");
        }
        let mut p = Prompt::new_account(None);
        assert_eq!(p.input(&typed("q"), false), None, "any other key waits");
    }

    /// Everything the keys do, a click does: the two words on the foot line
    /// answer. The regions are the ones the box records as it draws, set here
    /// by hand because drawing needs a window.
    #[test]
    fn the_pointer_can_answer_both_ways() {
        let mut p = Prompt::new_account(None);
        p.spots = vec![
            (Rect::new(0.0, 20.0, 40.0, 13.0), Spot::Yes),
            (Rect::new(50.0, 20.0, 40.0, 13.0), Spot::No),
        ];
        assert_eq!(p.click(10.0, 25.0), Some(Answer::Create));
        assert_eq!(p.click(60.0, 25.0), Some(Answer::Cancel));
        assert_eq!(p.click(200.0, 200.0), None, "outside everything");
    }

    /// Continuum's fields, in Continuum's order, with the e-mail first, marked
    /// required and explained; nothing ticked and no sex chosen to begin with.
    #[test]
    fn the_form_asks_continuums_questions_in_its_order() {
        let p = Prompt::registration();
        assert_eq!(
            labels(&p),
            [EMAIL, REAL_NAME, CITY, STATE, AGE, CONNECT_FROM, SEX]
        );
        assert_eq!(focused(&p), EMAIL, "the e-mail is picked first");
        assert!(matches!(
            &p.lines[p.find(EMAIL)],
            Line::Field(Field { required: true, .. })
        ));
        assert!(note_text(&p).contains(EMAIL_WHY));
        assert_eq!(p.set(CONNECT_FROM), [false, false, false]);
        assert_eq!(p.set(SEX), [false, false], "neither is chosen");
        assert_eq!(p.title, "-= Player Information =-");
        assert!(p.yes.ends_with("Submit Information"));
    }

    /// The top paragraph says what is sent, and does not claim a hardware
    /// profile goes with it, because none does.
    #[test]
    fn the_form_says_only_what_is_typed_is_sent() {
        let text = note_text(&Prompt::registration());
        assert!(text.contains("Only what you type or choose below is sent"));
        assert!(!text.to_lowercase().contains("hardware"));
    }

    /// The whole form from the keyboard: each field typed, Tab between them,
    /// Space to tick and choose, Left and Right along a row, Enter to send.
    /// Text is trimmed, and every answer lands in its own field.
    #[test]
    fn the_whole_form_fills_from_the_keyboard() {
        let mut p = Prompt::registration();
        let tab = keys(&[KeyCode::Tab]);
        let right = keys(&[KeyCode::ArrowRight]);
        p.input(&typed("ada@example.org "), false);
        p.input(&tab, false);
        p.input(&typed("Ada Lovelace"), false);
        p.input(&tab, false);
        p.input(&typed("London"), false);
        p.input(&tab, false);
        p.input(&typed("England"), false);
        p.input(&tab, false);
        p.input(&typed("36"), false);
        p.input(&tab, false);
        assert_eq!(focused(&p), CONNECT_FROM);
        p.input(&space(), false);
        p.input(&right, false);
        p.input(&right, false);
        p.input(&space(), false);
        p.input(&tab, false);
        assert_eq!(focused(&p), SEX);
        p.input(&right, false);
        p.input(&space(), false);
        assert_eq!(
            p.input(&keys(&[KeyCode::Enter]), false),
            Some(Answer::Send(RegistrationForm {
                real_name: "Ada Lovelace".into(),
                email: "ada@example.org".into(),
                city: "London".into(),
                state: "England".into(),
                age: 36,
                from_home: true,
                from_work: false,
                from_school: true,
                sex: Some(Sex::Female),
            }))
        );
    }

    /// Tab goes round, Shift-Tab and Up go back, and only the lines that take
    /// an answer are stopped at.
    #[test]
    fn tab_and_the_arrows_move_between_answers() {
        let mut p = Prompt::registration();
        p.input(&keys(&[KeyCode::ArrowUp]), false);
        assert_eq!(focused(&p), SEX, "back from the first goes to the last");
        p.input(&keys(&[KeyCode::Tab]), false);
        assert_eq!(focused(&p), EMAIL, "and on from the last to the first");
        p.input(&keys(&[KeyCode::ArrowDown]), false);
        assert_eq!(focused(&p), REAL_NAME);
        p.input(&keys(&[KeyCode::Tab]), true);
        assert_eq!(focused(&p), EMAIL);
    }

    /// The e-mail is required: without one, or with something plainly not
    /// one, the form is refused with the reason and the e-mail line picked.
    #[test]
    fn a_missing_or_odd_email_is_refused() {
        let mut p = Prompt::registration();
        p.input(&keys(&[KeyCode::Tab]), false);
        assert_eq!(p.input(&keys(&[KeyCode::Enter]), false), None);
        assert!(p.error.as_deref().is_some_and(|e| e.contains("required")));
        assert_eq!(focused(&p), EMAIL);
        p.input(&typed("Ada Lovelace"), false);
        assert_eq!(p.input(&keys(&[KeyCode::Enter]), false), None);
        assert!(
            p.error
                .as_deref()
                .is_some_and(|e| e.contains("does not look"))
        );
    }

    /// The e-mail is the only thing required, as in Continuum: a form with
    /// nothing else filled in, nothing ticked and no sex chosen is sent, with
    /// every other answer as not given.
    #[test]
    fn only_the_email_is_required() {
        let mut p = Prompt::registration();
        p.input(&typed("a@b.co"), false);
        assert_eq!(
            p.input(&keys(&[KeyCode::Enter]), false),
            Some(Answer::Send(RegistrationForm {
                real_name: String::new(),
                email: "a@b.co".into(),
                city: String::new(),
                state: String::new(),
                age: 0,
                from_home: false,
                from_work: false,
                from_school: false,
                sex: None,
            }))
        );
        assert_eq!(p.error, None);
        assert!(
            labels(&p).iter().all(|&l| l == EMAIL
                || !matches!(
                    &p.lines[p.find(l)],
                    Line::Field(Field { required: true, .. })
                )),
            "only the e-mail is marked required"
        );
    }

    /// Tick boxes tick and untick, any number at once; round buttons choose
    /// one, clear the other, and stay chosen.
    #[test]
    fn ticks_toggle_and_round_buttons_choose_one() {
        let mut p = Prompt::registration();
        p.focus = p.find(CONNECT_FROM);
        p.input(&space(), false);
        p.input(&keys(&[KeyCode::ArrowRight]), false);
        p.input(&space(), false);
        assert_eq!(p.set(CONNECT_FROM), [true, true, false]);
        p.input(&space(), false);
        assert_eq!(p.set(CONNECT_FROM), [true, false, false]);

        p.focus = p.find(SEX);
        p.input(&space(), false);
        assert_eq!(p.set(SEX), [true, false]);
        p.input(&keys(&[KeyCode::ArrowRight]), false);
        p.input(&space(), false);
        assert_eq!(p.set(SEX), [false, true]);
        p.input(&space(), false);
        assert_eq!(p.set(SEX), [false, true], "a chosen one stays chosen");
    }

    /// A held Space does not flicker a tick box on and off.
    #[test]
    fn a_held_space_ticks_once() {
        let mut p = Prompt::registration();
        p.focus = p.find(CONNECT_FROM);
        let held = Stroke {
            key: Some(KeyCode::Space),
            text: Some(' '),
            repeat: true,
        };
        p.input(&space(), false);
        p.input(&[held, held], false);
        assert_eq!(p.set(CONNECT_FROM), [true, false, false]);
    }

    /// Space is a space in a text field: real names have them.
    #[test]
    fn space_types_in_a_field() {
        let mut p = Prompt::registration();
        p.focus = p.find(REAL_NAME);
        p.input(&typed("Ada"), false);
        p.input(&space(), false);
        p.input(&typed("L"), false);
        assert_eq!(p.text(REAL_NAME), "Ada L");
    }

    /// Age takes digits only, and has to fit its one byte; left empty it is
    /// sent as 0, "not given".
    #[test]
    fn age_is_a_number_that_fits_a_byte() {
        let mut p = Prompt::registration();
        p.input(&typed("a@b.co"), false);
        p.focus = p.find(SEX);
        p.input(&space(), false);
        p.focus = p.find(AGE);
        p.input(&typed("3x0-0"), false);
        assert_eq!(p.text(AGE), "300", "digits only, three at most");
        assert_eq!(p.input(&keys(&[KeyCode::Enter]), false), None);
        assert!(p.error.as_deref().is_some_and(|e| e.contains("255")));
        assert_eq!(focused(&p), AGE);
        p.input(&keys(&[KeyCode::Backspace; 3]), false);
        assert!(matches!(
            p.input(&keys(&[KeyCode::Enter]), false),
            Some(Answer::Send(RegistrationForm { age: 0, .. }))
        ));
    }

    /// The fields stop where the form's do on the wire, so nothing typed is cut
    /// off unseen.
    #[test]
    fn the_form_fields_stop_at_the_wire_lengths() {
        let mut p = Prompt::registration();
        for (label, max) in [
            (EMAIL, EMAIL_MAX),
            (REAL_NAME, REAL_NAME_MAX),
            (CITY, CITY_MAX),
            (STATE, STATE_MAX),
        ] {
            p.focus = p.find(label);
            p.input(&typed(&"x".repeat(80)), false);
            assert_eq!(p.text(label).len(), max, "{label}");
        }
    }

    /// Esc disconnects, whatever is in the form, and the foot line says so in
    /// Continuum's words. There is no third way out.
    #[test]
    fn escape_disconnects() {
        let mut p = Prompt::registration();
        assert_eq!(p.no, "Esc = Disconnect from server");
        p.input(&typed("a@b.co"), false);
        assert_eq!(
            p.input(&keys(&[KeyCode::Escape]), false),
            Some(Answer::Disconnect)
        );
    }

    /// Everything the keys do on the form, a click does: a field takes the
    /// focus, a tick box or round button ticks or chooses, and the foot line
    /// answers.
    #[test]
    fn the_pointer_can_fill_and_answer_the_form() {
        let mut p = Prompt::registration();
        let (email, from, sex) = (p.find(EMAIL), p.find(CONNECT_FROM), p.find(SEX));
        p.spots = vec![
            (Rect::new(0.0, 0.0, 300.0, 13.0), Spot::Line(email)),
            (Rect::new(0.0, 20.0, 300.0, 13.0), Spot::Line(from)),
            (Rect::new(100.0, 20.0, 40.0, 13.0), Spot::Option(from, 0)),
            (Rect::new(150.0, 20.0, 40.0, 13.0), Spot::Option(from, 1)),
            (Rect::new(0.0, 40.0, 300.0, 13.0), Spot::Line(sex)),
            (Rect::new(100.0, 40.0, 40.0, 13.0), Spot::Option(sex, 0)),
            (Rect::new(150.0, 40.0, 40.0, 13.0), Spot::Option(sex, 1)),
            (Rect::new(0.0, 60.0, 40.0, 13.0), Spot::Yes),
            (Rect::new(50.0, 60.0, 40.0, 13.0), Spot::No),
        ];
        assert_eq!(p.click(10.0, 25.0), None);
        assert_eq!(focused(&p), CONNECT_FROM, "a row's label picks it");
        assert_eq!(p.set(CONNECT_FROM), [false, false, false]);
        p.click(160.0, 25.0);
        assert_eq!(p.set(CONNECT_FROM), [false, true, false]);
        p.click(160.0, 45.0);
        assert_eq!(p.set(SEX), [false, true]);
        p.click(110.0, 45.0);
        assert_eq!(p.set(SEX), [true, false]);
        p.click(10.0, 5.0);
        assert_eq!(focused(&p), EMAIL);
        p.input(&typed("a@b.co"), false);
        assert!(matches!(
            p.click(10.0, 65.0),
            Some(Answer::Send(RegistrationForm {
                from_work: true,
                sex: Some(Sex::Male),
                ..
            }))
        ));
        assert_eq!(p.click(60.0, 65.0), Some(Answer::Disconnect));
    }

    #[test]
    fn what_counts_as_an_email() {
        for good in ["a@b.co", "first.last@mail.example.org", "x+y@z.io"] {
            assert!(looks_like_email(good), "{good}");
        }
        for bad in [
            "", "ada", "@b.co", "a@", "a@b", "a@.co", "a@b.", "a b@c.d", "a@b@c.d",
        ] {
            assert!(!looks_like_email(bad), "{bad}");
        }
    }

    /// What a player types into the form is never printed by a log line.
    #[test]
    fn the_box_cannot_print_what_was_typed_into_it() {
        let mut p = Prompt::registration();
        p.input(&typed("ada@lovelace.org"), false);
        assert!(!format!("{p:?}").contains("lovelace"));
        let form = Answer::Send(RegistrationForm {
            real_name: "Ada Lovelace".into(),
            email: "ada@lovelace.org".into(),
            city: String::new(),
            state: String::new(),
            age: 0,
            from_home: false,
            from_work: false,
            from_school: false,
            sex: Some(Sex::Female),
        });
        assert!(!format!("{form:?}").contains("ovelace"));
    }
}
