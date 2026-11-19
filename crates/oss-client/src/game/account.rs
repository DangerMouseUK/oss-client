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
//! reply ([`oss_session::SessionEvent::RegistrationWanted`]). That form, from
//! 1998, has room for a real name, an e-mail address, a postal address, age,
//! sex, the Windows registered owner and thirteen lines describing the
//! computer. The box asks for the first two only -- what zones that ask are
//! seen to want is the e-mail -- and says plainly that nothing else is sent.
//! It can be skipped, and the zone is entered either way.
//!
//! Both boxes work fully from the keyboard and fully from the pointer: every
//! line that can be typed in can be clicked, and each key on the foot line is
//! a button as well, as the start-up screen's are.

use std::fmt;

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
    /// Send the registration form with these.
    Send { real_name: String, email: String },
    /// Do not send the form; enter the zone anyway.
    Skip,
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
    /// A line that takes text, by its index.
    Field(usize),
    /// The foot line's yes ("Enter = ...").
    Yes,
    /// The foot line's no ("Esc = ...").
    No,
}

/// One line that takes text.
#[derive(Clone, PartialEq, Eq)]
struct Entry {
    label: &'static str,
    value: String,
    /// Shown as stars.
    secret: bool,
    /// The most characters the field on the wire holds.
    max: usize,
}

/// Continuum's question, word for word.
const NEW_USER_QUESTION: &str =
    "Unknown name. Would you like to create a new user and enter the game using this name?";

/// How wide the new-account box is, in characters: Continuum's question on two
/// lines.
const NEW_USER_COLUMNS: usize = 50;

/// The registration form's real name and e-mail fields, less their NULs.
const REAL_NAME_MAX: usize = 31;
const EMAIL_MAX: usize = 63;

/// The box, its fields and where the pointer can find them.
///
/// `Debug` is written out rather than derived so what a player typed into it
/// cannot be printed.
pub struct Prompt {
    kind: Kind,
    title: &'static str,
    heading: &'static str,
    /// What the box says, one line each, before the fields.
    lines: Vec<String>,
    /// Lines shown under the fields, before the input: the label and what it says.
    shown: Vec<(&'static str, String)>,
    fields: Vec<Entry>,
    focus: usize,
    /// The two keys on the foot line, as they are drawn.
    yes: &'static str,
    no: &'static str,
    /// What went wrong with the last answer, in red under the fields.
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

impl Prompt {
    /// The offer to create the name the zone said it does not know, asked as
    /// Continuum asks it. `zone_text` is anything the zone said with the
    /// offer, shown under the question when there is some.
    pub fn new_account(zone_text: Option<&str>) -> Self {
        let wrap = |text: &str| super::chat::wrap(text, NEW_USER_COLUMNS, true);
        let mut lines = wrap(NEW_USER_QUESTION);
        if let Some(text) = zone_text.filter(|t| !t.trim().is_empty()) {
            lines.push(String::new());
            lines.extend(wrap(&format!("The zone says: {}", text.trim())));
        }
        Self {
            kind: Kind::NewAccount,
            title: "-= Information =-",
            heading: "",
            lines,
            shown: Vec::new(),
            fields: Vec::new(),
            focus: 0,
            yes: "Enter = Yes",
            no: "Esc = No",
            error: None,
            spots: Vec::new(),
        }
    }

    /// The zone's registration form: a real name and an e-mail address.
    pub fn registration() -> Self {
        let field = |label, max| Entry {
            label,
            value: String::new(),
            secret: false,
            max,
        };
        Self {
            kind: Kind::Registration,
            title: "-= Registration =-",
            heading: "This zone asks you to register",
            lines: vec![
                "This zone's billing server asks new players".to_string(),
                "for a real name and an e-mail address.".to_string(),
                String::new(),
                "Only these two are sent, nothing about this".to_string(),
                "computer. Esc enters the zone without them.".to_string(),
            ],
            shown: Vec::new(),
            fields: vec![
                field("Real name", REAL_NAME_MAX),
                field("E-mail", EMAIL_MAX),
            ],
            focus: 0,
            yes: "Enter = Send",
            no: "Esc = Skip",
            error: None,
            spots: Vec::new(),
        }
    }

    /// Applies one frame of keystrokes.
    ///
    /// Typing goes into the focused line, Backspace (held or not) takes from it,
    /// Tab and the arrows move between lines, Enter answers yes and Esc no. The
    /// new-account question, which has nothing to type in, also takes Y and N,
    /// as the buttons of the Windows message box it copies do.
    pub fn input(&mut self, strokes: &[Stroke], shift: bool) -> Option<Answer> {
        for stroke in strokes {
            if self.kind == Kind::NewAccount && !stroke.repeat {
                match stroke.text.map(|c| c.to_ascii_lowercase()) {
                    Some('y') => return self.yes(),
                    Some('n') => return Some(self.no()),
                    _ => {}
                }
            }
            let count = self.fields.len().max(1);
            if let Some(c) = stroke.text
                && !c.is_control()
                && let Some(e) = self.fields.get_mut(self.focus)
                && e.value.chars().count() < e.max
            {
                e.value.push(c);
            }
            match stroke.key {
                Some(KeyCode::Tab) if !shift => self.focus = (self.focus + 1) % count,
                Some(KeyCode::Tab | KeyCode::ArrowUp) => {
                    self.focus = (self.focus + count - 1) % count;
                }
                Some(KeyCode::ArrowDown) => self.focus = (self.focus + 1) % count,
                Some(KeyCode::Backspace) => {
                    if let Some(e) = self.fields.get_mut(self.focus) {
                        e.value.pop();
                    }
                }
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
            Spot::Field(i) => {
                self.focus = i;
                None
            }
            Spot::Yes => self.yes(),
            Spot::No => Some(self.no()),
        }
    }

    fn no(&self) -> Answer {
        match self.kind {
            Kind::NewAccount => Answer::Cancel,
            Kind::Registration => Answer::Skip,
        }
    }

    /// The yes, if the box's fields allow it; otherwise says why not.
    fn yes(&mut self) -> Option<Answer> {
        match self.kind {
            Kind::NewAccount => Some(Answer::Create),
            Kind::Registration => self.send(),
        }
    }

    /// The registration form's yes: an e-mail address is the one thing a zone
    /// that asks is after, so it has to be there and look like one. The real
    /// name may be left empty.
    fn send(&mut self) -> Option<Answer> {
        let value = |i: usize| {
            self.fields
                .get(i)
                .map_or_else(String::new, |e| e.value.trim().to_string())
        };
        let (real_name, email) = (value(0), value(1));
        let why = if email.is_empty() {
            Some("Enter an e-mail address, or press Esc to skip the form.")
        } else if !looks_like_email(&email) {
            Some("That does not look like an e-mail address.")
        } else {
            None
        };
        if let Some(why) = why {
            self.focus = 1;
            self.error = Some(why.to_string());
            return None;
        }
        self.error = None;
        Some(Answer::Send { real_name, email })
    }

    fn spot_at(&self, x: f32, y: f32) -> Option<Spot> {
        self.spots
            .iter()
            .rev()
            .find(|(r, _)| r.holds(x, y))
            .map(|(_, s)| *s)
    }

    /// The box's width in characters: wide enough for its longest line.
    fn columns(&self) -> usize {
        let foot = self.yes.len() + 8 + self.no.len();
        self.lines
            .iter()
            .map(|l| l.chars().count())
            .chain(
                self.shown
                    .iter()
                    .map(|(_, v)| LABEL_COLUMNS + v.chars().count()),
            )
            // A field as wide as it needs to be shown, not as long as it may get:
            // a long value scrolls, and sixty-three columns of e-mail made a box
            // wider than the screen.
            .chain(
                self.fields
                    .iter()
                    .map(|e| LABEL_COLUMNS + e.max.min(FIELD_COLUMNS) + 1),
            )
            .chain([self.title.len(), foot, 40])
            .max()
            .unwrap_or(40)
            + 2
    }

    /// Draws the box in the middle of the screen, in the start-up screen's
    /// style: green labels, red headings, the game's grey border, and the keys
    /// on a yellow foot line that are buttons as well. Records where everything
    /// went for [`Prompt::click`].
    pub fn draw(&mut self, c: &Canvas, r: &mut Renderer, cursor: Option<(f32, f32)>) {
        self.spots.clear();
        let hovered = |rect: Rect| cursor.is_some_and(|(mx, my)| rect.holds(mx, my));
        let cols = self
            .columns()
            .min(((c.view_w - 16.0) / 8.0).max(20.0) as usize);
        let w = cols as f32 * 8.0;
        let rows = usize::from(!self.heading.is_empty())
            + self.lines.len()
            + self.shown.len()
            + self.fields.len();
        // Title, rule, the heading and rows, a gap, two error lines, a rule, the keys.
        let h = 20.0 + rows as f32 * 14.0 + 8.0 + 2.0 * ERROR_PITCH + 8.0 + 16.0;
        let x = ((c.view_w - w) / 2.0).floor();
        let top = ((c.view_h - h) / 2.0).floor().max(4.0);
        c.panel(r, x, top, w, h);
        c.text_center(r, x + w / 2.0, top + 1.0, color::GREEN, self.title);
        c.separator(r, x, top + 13.0, w);

        let mut y = top + 20.0;
        if !self.heading.is_empty() {
            c.text(r, x + 4.0, y, color::DARK_RED, self.heading);
            y += 14.0;
        }
        let width = cols.saturating_sub(1);
        for line in &self.lines {
            let shown: String = line.chars().take(width).collect();
            c.text(r, x + 4.0, y, color::WHITE, &shown);
            y += 14.0;
        }
        let value_x = x + 4.0 + LABEL_COLUMNS as f32 * 8.0;
        for (label, value) in &self.shown {
            c.text(r, x + 4.0, y, color::GREEN, label);
            c.text(r, value_x, y, color::WHITE, value);
            y += 14.0;
        }
        for (i, e) in self.fields.iter().enumerate() {
            let row = Rect::new(x + 2.0, y - 1.0, w - 4.0, 13.0);
            self.spots.push((row, Spot::Field(i)));
            if self.focus == i || hovered(row) {
                c.color_rect(r, ui_color::HIGHLIGHT, row.x, row.y, row.w, row.h);
            }
            c.text(r, x + 4.0, y, color::GREEN, e.label);
            let mut value = if e.secret {
                "*".repeat(e.value.chars().count())
            } else {
                e.value.clone()
            };
            if self.focus == i {
                value.push('_');
            }
            let room = cols.saturating_sub(LABEL_COLUMNS + 1).max(1);
            let n = value.chars().count();
            let shown: String = value.chars().skip(n.saturating_sub(room)).collect();
            c.text(r, value_x, y, color::WHITE, &shown);
            y += 14.0;
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
        let total: f32 = items.iter().map(|(t, _)| t.len() as f32 * 8.0).sum::<f32>() + 64.0;
        let mut kx = (x + w / 2.0 - total / 2.0).floor();
        for (text, spot) in items {
            let tw = text.len() as f32 * 8.0;
            let rect = Rect::new(kx - 2.0, ky - 1.0, tw + 4.0, 13.0);
            self.spots.push((rect, spot));
            if hovered(rect) {
                c.color_rect(r, ui_color::HIGHLIGHT, rect.x, rect.y, rect.w, rect.h);
            }
            c.text(r, kx, ky, color::YELLOW, text);
            kx += tw + 64.0;
        }
    }
}

/// Where the values start, in characters from the box's left edge: the same
/// ten the start-up screen's profile lines use.
const LABEL_COLUMNS: usize = 10;

/// The most columns a field asks the box for; longer values scroll.
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

    /// The offer is Continuum's question, word for word, and nothing to type:
    /// the account is made with the password the login was sent with.
    #[test]
    fn the_offer_asks_continuums_question_and_nothing_else() {
        let p = Prompt::new_account(None);
        assert_eq!(p.lines.join(" "), NEW_USER_QUESTION);
        assert!(p.fields.is_empty(), "no second password");
        assert!(p.shown.is_empty());
        assert_eq!((p.yes, p.no), ("Enter = Yes", "Esc = No"));

        let p = Prompt::new_account(Some("  Welcome!  "));
        assert!(p.lines.iter().any(|l| l == "The zone says: Welcome!"));
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

    /// The form takes a real name and an e-mail, Tab moves between them, and
    /// Enter sends both -- trimmed, so a stray space is not sent to the zone.
    #[test]
    fn the_form_sends_a_real_name_and_an_email() {
        let mut p = Prompt::registration();
        p.input(&typed("Ada Lovelace "), false);
        p.input(&keys(&[KeyCode::Tab]), false);
        p.input(&typed("ada@example.org"), false);
        assert_eq!(
            p.input(&keys(&[KeyCode::Enter]), false),
            Some(Answer::Send {
                real_name: "Ada Lovelace".into(),
                email: "ada@example.org".into(),
            })
        );
    }

    /// No e-mail, or something that is plainly not one, is refused with the
    /// reason and the cursor on that line; Esc skips the form whatever is in it.
    #[test]
    fn the_form_wants_an_email_or_a_skip() {
        let mut p = Prompt::registration();
        assert_eq!(p.input(&keys(&[KeyCode::Enter]), false), None);
        assert!(p.error.as_deref().is_some_and(|e| e.contains("Esc")));
        assert_eq!(p.focus, 1);
        p.input(&typed("Ada Lovelace"), false);
        assert_eq!(p.input(&keys(&[KeyCode::Enter]), false), None);
        assert!(
            p.error
                .as_deref()
                .is_some_and(|e| e.contains("does not look"))
        );
        assert_eq!(
            p.input(&keys(&[KeyCode::Escape]), false),
            Some(Answer::Skip)
        );
    }

    /// The fields stop where the form's do on the wire, so nothing typed is cut
    /// off unseen.
    #[test]
    fn the_form_fields_stop_at_the_wire_lengths() {
        let mut p = Prompt::registration();
        p.input(&typed(&"x".repeat(80)), false);
        assert_eq!(p.fields[0].value.len(), REAL_NAME_MAX);
        p.input(&keys(&[KeyCode::ArrowDown]), false);
        p.input(&typed(&"y".repeat(80)), false);
        assert_eq!(p.fields[1].value.len(), EMAIL_MAX);
    }

    /// Clicks work on the form as on the offer: the no is a skip there.
    #[test]
    fn the_pointer_can_send_or_skip_the_form() {
        let mut p = Prompt::registration();
        p.spots = vec![
            (Rect::new(0.0, 0.0, 100.0, 13.0), Spot::Field(0)),
            (Rect::new(0.0, 14.0, 100.0, 13.0), Spot::Field(1)),
            (Rect::new(0.0, 30.0, 40.0, 13.0), Spot::Yes),
            (Rect::new(50.0, 30.0, 40.0, 13.0), Spot::No),
        ];
        assert_eq!(p.click(10.0, 20.0), None);
        assert_eq!(p.focus, 1, "clicking a line focuses it");
        p.input(&typed("a@b.co"), false);
        assert!(matches!(p.click(10.0, 35.0), Some(Answer::Send { .. })));
        assert_eq!(p.click(60.0, 35.0), Some(Answer::Skip));
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
        p.input(&typed("Ada Lovelace"), false);
        assert!(!format!("{p:?}").contains("Lovelace"));
    }
}
