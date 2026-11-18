//! The boxes a zone can ask a player to answer while logging in.
//!
//! A zone that does not know the name it was given offers to create it
//! ([`oss_session::SessionEvent::NewUserRequired`]). The real clients ask the
//! player before saying yes, and so does this one: a box over the connecting
//! screen, drawn the way the start-up screen is, that quotes the name back and
//! asks for the password a second time.
//!
//! Both halves of that are the guard against a typo becoming an account for
//! good. The name is quoted because a player who mistyped it believes they typed
//! it correctly until they are shown otherwise; the password is asked again
//! because a mistyped one is invisible -- the account would be made with it and
//! the player let in, and nothing would look wrong until the next visit.
//!
//! The box works fully from the keyboard and fully from the pointer: every
//! line that can be typed in can be clicked, and each key on its foot line is
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
    /// Create the account: the password was typed the same twice.
    Create,
    /// Do not create it; go back.
    Cancel,
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

/// The longest password a login carries: a 32-byte field with its NUL.
const PASSWORD_MAX: usize = 31;

/// The box, its fields and where the pointer can find them.
///
/// `Debug` is written out rather than derived so a password typed into it
/// cannot be printed.
pub struct Prompt {
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
    /// The offer to create `name`, which the zone said it does not know.
    /// `zone_text` is anything the zone said with the offer.
    pub fn new_account(name: &str, zone_text: Option<&str>) -> Self {
        let mut lines = vec![
            format!("No account named '{name}' on this zone."),
            "Create it, with the password you typed?".to_string(),
        ];
        if let Some(text) = zone_text.filter(|t| !t.trim().is_empty()) {
            lines.push(String::new());
            lines.push(format!("The zone says: {}", text.trim()));
        }
        lines.push(String::new());
        lines.push("Type the password again, to be sure of it.".to_string());
        Self {
            title: "-= New Account =-",
            heading: "Unknown name",
            lines,
            shown: vec![("Name", name.to_string())],
            fields: vec![Entry {
                label: "Password",
                value: String::new(),
                secret: true,
                max: PASSWORD_MAX,
            }],
            focus: 0,
            yes: "Enter = Create",
            no: "Esc = Cancel",
            error: None,
            spots: Vec::new(),
        }
    }

    /// Applies one frame of keystrokes. `password` is the one the login was
    /// sent with, which the second typing has to match.
    ///
    /// Typing goes into the focused line, Backspace (held or not) takes from it,
    /// Tab and the arrows move between lines, Enter answers yes and Esc no.
    pub fn input(&mut self, strokes: &[Stroke], shift: bool, password: &str) -> Option<Answer> {
        for stroke in strokes {
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
                    if let Some(answer) = self.yes(password) {
                        return Some(answer);
                    }
                }
                Some(KeyCode::Escape) => return Some(Answer::Cancel),
                _ => {}
            }
        }
        None
    }

    /// A click at (`x`, `y`) in UI pixels, against what the last frame drew.
    pub fn click(&mut self, x: f32, y: f32, password: &str) -> Option<Answer> {
        match self.spot_at(x, y)? {
            Spot::Field(i) => {
                self.focus = i;
                None
            }
            Spot::Yes => self.yes(password),
            Spot::No => Some(Answer::Cancel),
        }
    }

    /// The yes, if the box's fields allow it; otherwise says why not.
    fn yes(&mut self, password: &str) -> Option<Answer> {
        let again = self.fields.first().map_or("", |e| e.value.as_str());
        if again != password {
            // Cleared, so the second try starts from nothing rather than from
            // whatever was wrong, which the stars do not show.
            if let Some(e) = self.fields.first_mut() {
                e.value.clear();
            }
            self.focus = 0;
            self.error =
                Some("That is not the password you typed first. Type it again.".to_string());
            return None;
        }
        self.error = None;
        Some(Answer::Create)
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
            .chain(self.fields.iter().map(|e| LABEL_COLUMNS + e.max + 1))
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
        let rows = self.lines.len() + self.shown.len() + self.fields.len();
        // Title, rule, heading, the rows, a gap, the error line, a rule, the keys.
        let h = 20.0 + 14.0 + rows as f32 * 14.0 + 8.0 + 14.0 + 8.0 + 16.0;
        let x = ((c.view_w - w) / 2.0).floor();
        let top = ((c.view_h - h) / 2.0).floor().max(4.0);
        c.panel(r, x, top, w, h);
        c.text_center(r, x + w / 2.0, top + 1.0, color::GREEN, self.title);
        c.separator(r, x, top + 13.0, w);

        let mut y = top + 20.0;
        c.text(r, x + 4.0, y, color::DARK_RED, self.heading);
        y += 14.0;
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
        if let Some(err) = &self.error {
            let shown: String = err.chars().take(width).collect();
            c.text(r, x + 4.0, y, color::RED, &shown);
        }
        y += 14.0 + 4.0;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> Vec<Stroke> {
        s.chars().map(Stroke::text).collect()
    }

    fn keys(k: &[KeyCode]) -> Vec<Stroke> {
        k.iter().copied().map(Stroke::press).collect()
    }

    /// The box quotes the name back. That line is the guard against a typo in
    /// the name: without it the player has no reason to think the name is wrong.
    #[test]
    fn the_offer_quotes_the_name_back() {
        let p = Prompt::new_account("Acee", None);
        assert!(
            p.lines.iter().any(|l| l.contains("'Acee'")),
            "{:?}",
            p.lines
        );
        assert!(p.shown.iter().any(|(_, v)| v == "Acee"));
    }

    /// The same password twice creates; a different one is refused, said, and
    /// cleared for another go -- the second typing is what stops a mistyped
    /// password becoming the account's.
    #[test]
    fn a_mistyped_second_password_is_refused_and_a_matching_one_creates() {
        let mut p = Prompt::new_account("Acee", None);
        assert_eq!(p.input(&typed("hunter3"), false, "hunter2"), None);
        assert_eq!(p.input(&keys(&[KeyCode::Enter]), false, "hunter2"), None);
        assert!(p.error.is_some(), "a mismatch says so");
        assert_eq!(p.fields[0].value, "", "and starts the field again");

        p.input(&typed("hunter2"), false, "hunter2");
        assert_eq!(
            p.input(&keys(&[KeyCode::Enter]), false, "hunter2"),
            Some(Answer::Create)
        );
        assert_eq!(p.error, None);
    }

    /// Esc is no, whatever has been typed; Backspace edits.
    #[test]
    fn escape_cancels_and_backspace_edits() {
        let mut p = Prompt::new_account("Acee", None);
        p.input(&typed("abx"), false, "ab");
        p.input(&keys(&[KeyCode::Backspace]), false, "ab");
        assert_eq!(p.fields[0].value, "ab");
        assert_eq!(
            p.input(&keys(&[KeyCode::Escape]), false, "ab"),
            Some(Answer::Cancel)
        );
    }

    /// A zone that keeps passwords empty has to be answerable too: an empty
    /// first password is matched by an empty second one.
    #[test]
    fn an_empty_password_is_matched_by_an_empty_one() {
        let mut p = Prompt::new_account("Acee", None);
        assert_eq!(
            p.input(&keys(&[KeyCode::Enter]), false, ""),
            Some(Answer::Create)
        );
    }

    /// Everything the keys do, a click does: the field takes the focus and the
    /// two words on the foot line answer. The regions are the ones the box
    /// records as it draws, set here by hand because drawing needs a window.
    #[test]
    fn the_pointer_can_answer_both_ways() {
        let mut p = Prompt::new_account("Acee", None);
        p.spots = vec![
            (Rect::new(0.0, 0.0, 100.0, 13.0), Spot::Field(0)),
            (Rect::new(0.0, 20.0, 40.0, 13.0), Spot::Yes),
            (Rect::new(50.0, 20.0, 40.0, 13.0), Spot::No),
        ];
        assert_eq!(p.click(10.0, 5.0, "pw"), None, "a field only takes focus");
        assert_eq!(p.click(10.0, 25.0, "pw"), None, "nothing typed yet");
        assert!(p.error.is_some());
        p.input(&typed("pw"), false, "pw");
        assert_eq!(p.click(10.0, 25.0, "pw"), Some(Answer::Create));
        assert_eq!(p.click(60.0, 25.0, "pw"), Some(Answer::Cancel));
        assert_eq!(p.click(200.0, 200.0, "pw"), None, "outside everything");
    }

    /// A password typed into the box is never printed by a log line.
    #[test]
    fn the_box_cannot_print_what_was_typed_into_it() {
        let mut p = Prompt::new_account("Acee", None);
        p.input(&typed("hunter2"), false, "x");
        assert!(!format!("{p:?}").contains("hunter2"));
    }
}
