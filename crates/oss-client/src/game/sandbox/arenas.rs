//! The arena list: Esc then A, or `?arena`.
//!
//! Continuum shows the zone's arenas in a box at the top centre of the screen,
//! headed with the arena name and its player count, your own arena first and
//! marked `>`. Its own tips file says how it is used: Esc then A (or `?arena`) to
//! see the list, Page Up and Page Down to choose, Enter to go. The rest of the
//! screen stays live underneath, so you can keep flying and typing; the list
//! takes only the keys it needs.
//!
//! The list comes from the zone, not from us: `?arena` goes to the zone as any
//! other command does, and its answer (S2C `0x2F`) opens the box, whichever of
//! the two asked.

use oss_session::ArenaListed;
use winit::keyboard::KeyCode;

use super::Sandbox;
use crate::game::input::{Action, Stroke};
use crate::game::start_spots::Rect;
use crate::game::view::{Canvas, color};
use crate::gfx::Renderer;

/// The open arena list.
#[derive(Debug, Default)]
pub(super) struct ArenaList {
    /// Your own arena first, then the rest in the order the zone sent them.
    pub(super) arenas: Vec<ArenaListed>,
    /// The row Page Up and Page Down have chosen.
    pub(super) selected: usize,
    /// Where each row was drawn last frame, by its index in `arenas`, so a click
    /// lands on the row that was seen.
    pub(super) spots: Vec<(Rect, usize)>,
}

impl ArenaList {
    /// The list as Continuum shows it: yours on top. A list that arrives while
    /// one is already open keeps the same arena chosen, if it is still there.
    pub(super) fn new(mut arenas: Vec<ArenaListed>, keep: Option<&str>) -> Self {
        // A stable sort, so the zone's own order survives under your arena.
        arenas.sort_by_key(|a| !a.yours);
        let selected = keep
            .and_then(|name| {
                arenas
                    .iter()
                    .position(|a| a.name.eq_ignore_ascii_case(name))
            })
            .unwrap_or(0);
        Self {
            arenas,
            selected,
            spots: Vec::new(),
        }
    }

    /// Page Up (`up`) or Page Down, stopping at either end.
    pub(super) fn step(&mut self, up: bool) {
        self.selected = if up {
            self.selected.saturating_sub(1)
        } else {
            (self.selected + 1).min(self.arenas.len().saturating_sub(1))
        };
    }

    /// The arena Enter would go to.
    pub(super) fn chosen(&self) -> Option<&ArenaListed> {
        self.arenas.get(self.selected)
    }
}

/// How many characters wide the list's box is, as Continuum's is about.
const LIST_COLS: usize = 30;
/// Where the count's column ends, from the box's left edge, in characters.
const COUNT_END: usize = LIST_COLS - 1;
const _: () = assert!(COUNT_END < LIST_COLS, "the count must be inside the box");
/// The most rows drawn at once. A zone with more arenas than this scrolls, with
/// the chosen one kept in view.
const MAX_ROWS: usize = 20;

/// What one row of the list says: the marker, the name cut to fit, and the count
/// right-aligned. The name is cut rather than wrapped because the count has to
/// stay on the same line as its arena.
fn row_text(a: &ArenaListed) -> (String, String) {
    let marker = if a.yours { ">" } else { " " };
    let count = a.players.to_string();
    let room = COUNT_END.saturating_sub(count.len() + 3);
    let name: String = shown_name(&a.name).chars().take(room).collect();
    (format!("{marker}{name}"), count)
}

/// A public arena's name is its number (`0`, `1`, ...), which Continuum's list
/// shows as `(Public 0)`. Going there still uses the number, which is what the
/// zone knows it by.
fn shown_name(name: &str) -> String {
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
        format!("(Public {name})")
    } else {
        name.to_string()
    }
}

/// Which rows are drawn, as a range of indices: all of them when they fit, and
/// otherwise a window that keeps `selected` on screen.
fn visible(len: usize, selected: usize) -> std::ops::Range<usize> {
    if len <= MAX_ROWS {
        return 0..len;
    }
    let start = selected.saturating_sub(MAX_ROWS / 2).min(len - MAX_ROWS);
    start..start + MAX_ROWS
}

impl Sandbox {
    /// Esc then A: asks the zone for its arenas. The box opens when the answer
    /// arrives, exactly as it does for a typed `?arena`.
    pub(super) fn ask_for_arenas(&mut self) {
        let Some(net) = self.net.as_mut() else {
            self.message("The arena list needs a zone.", color::YELLOW);
            return;
        };
        net.session
            .send_chat(oss_session::chat_type::PUBLIC, 0, "?arena");
        net.flush();
    }

    /// The zone's answer to `?arena`.
    pub(super) fn arenas_arrived(&mut self, arenas: Vec<ArenaListed>) {
        if arenas.is_empty() {
            // SubSpace's own words for an empty list.
            self.message("Could not find any arenas.", color::YELLOW);
            self.arena_list = None;
            return;
        }
        let keep = self
            .arena_list
            .as_ref()
            .and_then(|l| l.chosen())
            .map(|a| a.name.clone());
        self.arena_list = Some(ArenaList::new(arenas, keep.as_deref()));
    }

    /// The keys the open list takes, before anything else sees them: Page Up and
    /// Page Down choose, Enter goes (unless there is a message on the chat line,
    /// which Enter still sends), and Esc puts the list away instead of opening
    /// the menu. Everything else passes through, so the game goes on.
    ///
    /// Returns the keystrokes left for the rest of the screen.
    pub(super) fn arena_list_input(
        &mut self,
        edges: &mut Vec<Action>,
        strokes: &[Stroke],
    ) -> Vec<Stroke> {
        if self.arena_list.is_none() {
            return strokes.to_vec();
        }
        let typing = !self.chat.input.is_empty();
        let mut rest = Vec::with_capacity(strokes.len());
        let mut go = false;
        for stroke in strokes {
            let Some(list) = self.arena_list.as_mut() else {
                rest.push(*stroke);
                continue;
            };
            match stroke.key {
                // Held keys repeat through the list, as they do in the chat log.
                Some(KeyCode::PageUp) => list.step(true),
                Some(KeyCode::PageDown) => list.step(false),
                Some(KeyCode::Escape) if !stroke.repeat => self.arena_list = None,
                Some(KeyCode::Enter | KeyCode::NumpadEnter) if !typing && !stroke.repeat => {
                    go = true
                }
                _ => rest.push(*stroke),
            }
        }
        // The same keys reach the game as actions too; while the list is up,
        // Page Up and Page Down are its and not the stat box's, and Esc closes
        // the list rather than opening the menu.
        edges.retain(|e| !matches!(e, Action::StatBoxUp | Action::StatBoxDown | Action::Menu));
        if go {
            self.go_to_chosen_arena();
        }
        rest
    }

    /// Enter on the list, or a click on one of its rows.
    fn go_to_chosen_arena(&mut self) {
        let Some(list) = self.arena_list.take() else {
            return;
        };
        let Some(arena) = list.chosen() else { return };
        if arena.yours {
            self.message(format!("You are already in {}.", arena.name), color::YELLOW);
            return;
        }
        let name = arena.name.clone();
        self.go_to_arena(&name);
    }

    /// A click on a row of the list chooses that arena and goes there, as our
    /// menu's rows already do. `true` when the click landed on the list.
    pub(super) fn click_arena_list(&mut self, x: f32, y: f32) -> bool {
        let Some(list) = self.arena_list.as_mut() else {
            return false;
        };
        let Some(&(_, i)) = list.spots.iter().find(|(rect, _)| rect.holds(x, y)) else {
            return false;
        };
        list.selected = i;
        self.go_to_chosen_arena();
        true
    }

    /// The box at the top centre, your arena first and marked, the chosen row
    /// lit, and the keys that work underneath.
    pub(super) fn draw_arena_list(&mut self, c: &Canvas, r: &mut Renderer) {
        let Some(list) = self.arena_list.as_mut() else {
            return;
        };
        let rows = visible(list.arenas.len(), list.selected);
        let w = LIST_COLS as f32 * 8.0;
        let h = 20.0 + (rows.len() as f32 + 1.0) * 12.0 + 4.0;
        let x = ((c.view_w - w) / 2.0).floor();
        let y = 4.0;
        c.panel(r, x, y, w, h);
        c.text(r, x + 2.0, y + 1.0, color::GREEN, " Arena-name");
        c.text_right(
            r,
            x + COUNT_END as f32 * 8.0,
            y + 1.0,
            color::GREEN,
            "Count",
        );
        c.separator(r, x, y + 13.0, w);
        list.spots.clear();
        for (row, i) in rows.clone().enumerate() {
            let a = &list.arenas[i];
            let ry = y + 18.0 + row as f32 * 12.0;
            if i == list.selected {
                c.rect(r, x + 1.0, ry - 1.0, w - 2.0, 12.0, [0.25, 0.25, 0.45, 1.0]);
            }
            let colour = if a.yours { color::YELLOW } else { color::WHITE };
            let (name, count) = row_text(a);
            c.text(r, x + 2.0, ry, colour, &name);
            c.text_right(r, x + COUNT_END as f32 * 8.0, ry, colour, &count);
            list.spots.push((Rect::new(x, ry - 1.0, w, 12.0), i));
        }
        let foot = y + 18.0 + rows.len() as f32 * 12.0 + 2.0;
        c.text_center(r, x + w / 2.0, foot, color::YELLOW, "PgUp/PgDn, Enter = go");
    }
}

#[cfg(test)]
mod tests {
    use super::{ArenaList, COUNT_END, MAX_ROWS, row_text, shown_name, visible};
    use oss_session::ArenaListed;

    fn arena(name: &str, players: u16, yours: bool) -> ArenaListed {
        ArenaListed {
            name: name.into(),
            players,
            yours,
        }
    }

    /// Your arena first, the zone's order kept under it, and it is the one
    /// chosen when the list opens.
    #[test]
    fn your_arena_comes_first_and_is_chosen() {
        let l = ArenaList::new(
            vec![
                arena("duel", 2, false),
                arena("elim", 3, false),
                arena("0", 9, true),
                arena("twd", 2, false),
            ],
            None,
        );
        let names: Vec<_> = l.arenas.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["0", "duel", "elim", "twd"]);
        assert_eq!(l.chosen().map(|a| a.name.as_str()), Some("0"));
    }

    /// Page Up and Page Down stop at the ends rather than wrapping or running off.
    #[test]
    fn choosing_stops_at_either_end() {
        let mut l = ArenaList::new(vec![arena("a", 1, true), arena("b", 1, false)], None);
        l.step(true);
        assert_eq!(l.selected, 0);
        l.step(false);
        l.step(false);
        assert_eq!(l.chosen().map(|a| a.name.as_str()), Some("b"));
    }

    /// A list that comes again while one is open keeps the choice by name, even
    /// though the rows may have moved.
    #[test]
    fn a_fresh_list_keeps_the_choice() {
        let l = ArenaList::new(
            vec![
                arena("a", 1, true),
                arena("new", 1, false),
                arena("b", 1, false),
            ],
            Some("B"),
        );
        assert_eq!(l.chosen().map(|a| a.name.as_str()), Some("b"));
    }

    /// Every row fits the box, however long the arena's name, and the count is
    /// never pushed out by it.
    #[test]
    fn a_long_name_is_cut_and_the_count_stays() {
        let (name, count) = row_text(&arena(&"x".repeat(80), 1234, true));
        assert!(name.starts_with('>'));
        assert_eq!(count, "1234");
        assert!(
            name.chars().count() + 1 + count.len() <= COUNT_END,
            "{name} {count} runs into the count column"
        );
    }

    /// Public arenas are named by number on the wire and in words on screen.
    #[test]
    fn a_public_arena_is_shown_as_public() {
        assert_eq!(shown_name("0"), "(Public 0)");
        assert_eq!(shown_name("12"), "(Public 12)");
        assert_eq!(shown_name("duel2"), "duel2");
        assert_eq!(shown_name(""), "");
    }

    /// A zone with more arenas than the box holds scrolls, keeping the chosen
    /// row in view.
    #[test]
    fn a_long_list_scrolls_to_the_choice() {
        assert_eq!(visible(5, 4), 0..5);
        for selected in [0, MAX_ROWS, 49] {
            let rows = visible(50, selected);
            assert_eq!(rows.len(), MAX_ROWS);
            assert!(rows.contains(&selected), "{selected} not in {rows:?}");
        }
    }
}
