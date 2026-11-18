//! The F1 help: pages read from a help file, and the box they are drawn in.
//!
//! The pages are written in the format of Continuum's own help file, because a
//! zone can send its own help in that format, packed in one of its LVZ files, and
//! a client that reads the format shows every zone's help for nothing. Trench
//! Wars does exactly that. So this client's pages are files too, written in the
//! same format and built into the program: the general ones (keys, chat, the
//! commands zones answer) give way to a zone's own, and the page about this
//! client itself always follows whichever came first.
//!
//! The format, as far as any file we have uses it:
//!
//! * a line starting `#` starts a page, and the rest of it is the page's title;
//! * `%Y`, `%W`, `%G`, `%B`, `%R`, `%P` switch to yellow, white, green, blue,
//!   red and pink, and `%p` and `%b` to the dark red used for headings;
//! * `%K` and a key name is a key: `%KB_BULLET` is whatever the player has bound
//!   to guns, so one file shows every player their own keys, and `%KEsc` is
//!   just the word, for keys nobody can rebind. A key takes a fixed-width
//!   column, which is what lines up the key pages;
//! * `%%` is a percent sign. The files always follow it with a colour code
//!   (`%%Ytickname`), which reads as the sign and then the colour;
//! * a `%` before anything else is itself.

use winit::keyboard::KeyCode;

use super::Sandbox;
use crate::game::input::{Action, Bindings, Stroke};
use crate::game::view::{Canvas, color};
use crate::gfx::Renderer;

/// This client's general pages: keys, chat, and the commands zones answer. A
/// zone that sends its own help is shown instead.
const GENERAL: &str = include_str!("help/general.hlp");
/// The page about this client itself, after whichever pages come first.
const CLIENT: &str = include_str!("help/client.hlp");
/// What a zone's help file is called inside its LVZ (compared in lower case).
const ZONE_HELP_FILE: &str = "commands.hlp";

/// How many characters a key takes, so that the descriptions after the keys
/// line up whatever the keys are called.
const KEY_COLS: usize = 18;
/// How far in from the box's edge the text starts, in pixels.
const INSET: f32 = 2.0;
/// The widest our own pages are written to be, in characters. A zone's pages
/// may be wider; the box grows for them, up to the screen.
#[cfg(test)]
const OWN_COLS: usize = 60;

/// One character and the colour it is drawn in.
type Cell = (char, u32);

/// One page of a help file, read but not yet fitted to a screen.
#[derive(Debug, Clone, PartialEq)]
struct Page {
    title: String,
    lines: Vec<Vec<Cell>>,
}

/// A page as it is drawn: wrapped to the box, and cut to what fits on the
/// screen, so that one page of the file may be more than one of these.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    title: String,
    lines: Vec<Vec<Cell>>,
    /// The widest line, in characters.
    cols: usize,
}

impl Sandbox {
    /// Keys while the help is up: F1 or Page Down turns the page, Page Up turns
    /// back, Esc or Enter closes it. F1 past the last page closes it too, which
    /// is what "F1 = More help" has always done.
    pub(super) fn help_input(&mut self, strokes: &[Stroke]) {
        let last = self.help_count.saturating_sub(1);
        for stroke in strokes {
            match stroke.first_press() {
                Some(KeyCode::F1) => self.help_page += 1,
                Some(KeyCode::PageDown) => self.help_page = (self.help_page + 1).min(last),
                Some(KeyCode::PageUp) => self.help_page = self.help_page.saturating_sub(1),
                Some(KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter) => {
                    self.help_open = false
                }
                _ => {}
            }
        }
        // Until the pages have been drawn once there is no count to go past.
        if self.help_count > 0 && self.help_page >= self.help_count {
            self.help_open = false;
            self.help_page = 0;
        }
    }

    /// The F1 help, in a box at the top left sized to the page, as Continuum
    /// draws it, with the page number in the title.
    pub(super) fn draw_help(&mut self, c: &Canvas, r: &mut Renderer) {
        let zone = self.lvz.file(ZONE_HELP_FILE);
        let pages = book(&self.bindings, zone);
        let max_cols = (((c.view_w - 8.0 - 2.0 * INSET) / 8.0).floor() as usize).max(16);
        // The title, its rule, the footer and the margins take about four rows.
        let max_rows = (((c.view_h - 8.0) / 12.0).floor() as usize)
            .saturating_sub(4)
            .max(3);
        let shown = lay_out(&pages, max_cols, max_rows);
        self.help_count = shown.len();
        let at = self.help_page.min(shown.len().saturating_sub(1));
        let Some(page) = shown.get(at) else { return };

        let counter = format!("{}/{}", at + 1, shown.len());
        let footer = footer(at + 1 < shown.len(), max_cols);
        let cols = page
            .cols
            .max(footer.len())
            .max(page.title.chars().count() + 2 + counter.len())
            .min(max_cols);
        let title: String = page
            .title
            .chars()
            .take(cols.saturating_sub(counter.len() + 1))
            .collect();

        let w = cols as f32 * 8.0 + 2.0 * INSET;
        let h = 18.0 + (page.lines.len() as f32 + 1.0) * 12.0 + 4.0;
        let (x, y) = (4.0, 4.0);
        c.panel(r, x, y, w, h);
        c.text(r, x + INSET, y + 1.0, color::GREEN, &title);
        c.text_right(r, x + w - INSET, y + 1.0, color::GREEN, &counter);
        c.separator(r, x, y + 13.0, w);
        for (i, line) in page.lines.iter().enumerate() {
            draw_cells(c, r, x + INSET, y + 18.0 + i as f32 * 12.0, line);
        }
        c.text_center(
            r,
            x + w / 2.0,
            y + 18.0 + page.lines.len() as f32 * 12.0 + 2.0,
            color::YELLOW,
            footer,
        );
    }
}

/// The footer, in the shorter form when the long one would not fit.
fn footer(more: bool, max_cols: usize) -> &'static str {
    let long = if more {
        "F1 = More help    Esc = Back to game"
    } else {
        "F1 or Esc = Back to game"
    };
    if long.len() <= max_cols {
        long
    } else if more {
        "F1: more  Esc: back"
    } else {
        "Esc: back"
    }
}

/// Draws a line of cells, one run of text per colour.
fn draw_cells(c: &Canvas, r: &mut Renderer, x: f32, y: f32, line: &[Cell]) {
    let mut start = 0;
    while start < line.len() {
        let colour = line[start].1;
        let end = line[start..]
            .iter()
            .position(|cell| cell.1 != colour)
            .map_or(line.len(), |n| start + n);
        let text: String = line[start..end].iter().map(|cell| cell.0).collect();
        c.text(r, x + start as f32 * 8.0, y, colour, &text);
        start = end;
    }
}

/// The pages to show: the zone's own help if it sent one that reads as any
/// pages at all, otherwise ours, and then the page about this client.
fn book(bindings: &Bindings, zone: Option<&[u8]>) -> Vec<Page> {
    let key = |name: &str| key_text(bindings, name);
    // Help files are 8-bit text from the 1990s, not UTF-8: one byte, one
    // character.
    let zone: Option<String> = zone.map(|b| b.iter().map(|&b| b as char).collect());
    let mut pages = zone
        .as_deref()
        .map(|t| parse(t, &key))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| parse(GENERAL, &key));
    pages.extend(parse(CLIENT, &key));
    pages
}

/// The colour a code letter switches to.
fn colour_code(c: char) -> Option<u32> {
    Some(match c {
        'Y' => color::YELLOW,
        'W' => color::WHITE,
        'G' => color::GREEN,
        'B' => color::BLUE,
        'R' => color::RED,
        'P' => color::PINK,
        'p' | 'b' => color::DARK_RED,
        _ => return None,
    })
}

/// Continuum's names for the keys a help file can ask for, against ours.
const CONTINUUM_KEYS: &[(&str, Action)] = &[
    ("LEFT", Action::Left),
    ("RIGHT", Action::Right),
    ("FORWARD", Action::Forward),
    ("REVERSE", Action::Backward),
    ("AFTERBURNERS", Action::Afterburner),
    ("BULLET", Action::Gun),
    ("BOMB", Action::Bomb),
    ("MINE", Action::Mine),
    ("THOR", Action::Thor),
    ("BURST", Action::Burst),
    ("STEALTH", Action::Stealth),
    ("CLOAK", Action::Cloak),
    ("XRADAR", Action::XRadar),
    ("ANTIWARP", Action::Antiwarp),
    ("MULTI", Action::Multifire),
    ("REPEL", Action::Repel),
    ("WARP", Action::Warp),
    ("PORTAL", Action::Portal),
    ("DECOY", Action::Decoy),
    ("ROCKET", Action::Rocket),
    ("BRICK", Action::Brick),
    ("ATTACH", Action::Attach),
    ("VIEWMAP", Action::BigRadar),
    ("CYCLE", Action::StatBox),
    ("PREV", Action::StatBoxUp),
    ("NEXT", Action::StatBoxDown),
    ("PLAY", Action::Spectate),
];

/// What `%K` and `name` shows. `B_` and a name is a binding, by Continuum's name
/// for it or by this client's own (the names in the settings file, which is how
/// our pages reach the keys Continuum does not have); anything else is the
/// key's own name, written out.
fn key_text(bindings: &Bindings, name: &str) -> String {
    let Some(binding) = name.strip_prefix("B_") else {
        return match name {
            "PrintScrn" => "Print Screen".to_string(),
            other => other.to_string(),
        };
    };
    CONTINUUM_KEYS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(binding))
        .map(|(_, a)| *a)
        .or_else(|| Action::from_name(binding))
        .map_or_else(|| "(no key)".to_string(), |a| bindings.label(a))
}

/// Reads a help file into pages. Lines before the first `#` are a page of their
/// own with a plain title, so a file that forgets one still shows.
fn parse(text: &str, key: &dyn Fn(&str) -> String) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut current: Option<Page> = None;
    for raw in text.lines() {
        if let Some(title) = raw.strip_prefix('#') {
            pages.extend(current.take());
            current = Some(Page {
                title: title.trim().to_string(),
                lines: Vec::new(),
            });
            continue;
        }
        current
            .get_or_insert_with(|| Page {
                title: "Help".to_string(),
                lines: Vec::new(),
            })
            .lines
            .push(parse_line(raw, key));
    }
    pages.extend(current);
    pages
}

/// One line of a help file, its codes turned into colours and keys.
fn parse_line(raw: &str, key: &dyn Fn(&str) -> String) -> Vec<Cell> {
    let mut out = Vec::new();
    let mut colour = color::WHITE;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            // A tab is the one control character these files might hold.
            out.push((if c == '\t' { ' ' } else { c }, colour));
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push(('%', colour));
                if let Some(next) = chars.peek().and_then(|&n| colour_code(n)) {
                    chars.next();
                    colour = next;
                }
            }
            Some('K') => {
                chars.next();
                let mut name = String::new();
                while let Some(&n) = chars.peek().filter(|n| !n.is_whitespace()) {
                    name.push(n);
                    chars.next();
                }
                let label = key(&name);
                let pad = KEY_COLS.saturating_sub(label.chars().count());
                out.extend(label.chars().map(|ch| (ch, colour)));
                out.extend(std::iter::repeat_n((' ', colour), pad));
            }
            Some(code) if colour_code(code).is_some() => {
                chars.next();
                colour = colour_code(code).unwrap_or(colour);
            }
            _ => out.push(('%', colour)),
        }
    }
    out
}

/// Fits pages to a box at most `max_cols` wide and `max_rows` tall: long lines
/// wrap, and a page too tall for the screen goes on over as many pages as it
/// needs, so the page count is the count of what is actually seen.
fn lay_out(pages: &[Page], max_cols: usize, max_rows: usize) -> Vec<Shown> {
    let max_rows = max_rows.max(1);
    let mut shown = Vec::new();
    for page in pages {
        let lines: Vec<Vec<Cell>> = page
            .lines
            .iter()
            .flat_map(|l| wrap_cells(l, max_cols))
            .collect();
        if lines.is_empty() {
            shown.push(Shown {
                title: page.title.clone(),
                lines,
                cols: 0,
            });
            continue;
        }
        for (i, chunk) in lines.chunks(max_rows).enumerate() {
            let title = if i == 0 {
                page.title.clone()
            } else {
                format!("{} (continued)", page.title)
            };
            shown.push(Shown {
                title,
                cols: chunk.iter().map(Vec::len).max().unwrap_or(0),
                lines: chunk.to_vec(),
            });
        }
    }
    shown
}

/// Where a wrapped line's continuations start: under the description where the
/// line has one lined up after a run of spaces (a key, a command), otherwise
/// under its own first character.
fn hang(line: &[Cell], cols: usize) -> usize {
    let lead = line.iter().take_while(|c| c.0 == ' ').count();
    let column = (lead + 1..line.len())
        .find(|&i| line[i].0 != ' ' && line[i - 1].0 == ' ' && i >= 2 && line[i - 2].0 == ' ');
    let at = column.unwrap_or(lead);
    // Leave room for some text on every continuation, or wrapping never ends.
    if at + 8 <= cols { at } else { 0 }
}

/// Wraps a line to `cols` characters, at the last space that fits, cutting a
/// word with nowhere to break: staying inside the box wins over keeping a word
/// whole.
fn wrap_cells(line: &[Cell], cols: usize) -> Vec<Vec<Cell>> {
    if cols == 0 {
        return Vec::new();
    }
    if line.len() <= cols {
        return vec![line.to_vec()];
    }
    let indent = hang(line, cols);
    let mut out: Vec<Vec<Cell>> = Vec::new();
    let mut rest: &[Cell] = line;
    loop {
        let first = out.is_empty();
        let room = cols - if first { 0 } else { indent };
        let mut row: Vec<Cell> = if first {
            Vec::new()
        } else {
            vec![(' ', rest.first().map_or(color::WHITE, |c| c.1)); indent]
        };
        if rest.len() <= room {
            row.extend_from_slice(rest);
            out.push(row);
            return out;
        }
        let cut = rest[..=room]
            .iter()
            .rposition(|c| c.0 == ' ')
            .filter(|&p| p > 0)
            .unwrap_or(room);
        row.extend_from_slice(&rest[..cut]);
        out.push(row);
        rest = &rest[cut..];
        while rest.first().is_some_and(|c| c.0 == ' ') {
            rest = &rest[1..];
        }
        if rest.is_empty() {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CLIENT, Cell, GENERAL, KEY_COLS, OWN_COLS, Page, book, color, key_text, lay_out, parse,
        parse_line, wrap_cells,
    };
    use crate::game::input::{Action, Bindings};

    fn text(line: &[Cell]) -> String {
        line.iter().map(|c| c.0).collect()
    }

    fn plain(name: &str) -> String {
        format!("<{name}>")
    }

    /// The codes, one at a time, in made-up lines shaped like the real files'.
    #[test]
    fn the_codes_read_as_the_format_says() {
        // Colours switch where the code is, and the code itself is not drawn.
        let line = parse_line("%Yone %Wtwo", &plain);
        assert_eq!(text(&line), "one two");
        assert_eq!(line[0].1, color::YELLOW);
        assert_eq!(line[4].1, color::WHITE);

        // A key fills its column, so what follows it lines up.
        let line = parse_line("%Y %KB_BULLET %WGuns", &plain);
        assert_eq!(text(&line), format!(" {:<KEY_COLS$} Guns", "<B_BULLET>"));

        // `%%` and a colour is a percent sign in that colour: how the files
        // write the `%tickname` substitutions.
        let line = parse_line("%W%%Ytickname", &plain);
        assert_eq!(text(&line), "%tickname");
        assert_eq!(line[1].1, color::YELLOW);

        // A percent before anything else is just a percent.
        assert_eq!(
            text(&parse_line("Returns % of packets", &plain)),
            "Returns % of packets"
        );
        assert_eq!(text(&parse_line("100%", &plain)), "100%");
        assert_eq!(text(&parse_line("%z", &plain)), "%z");
    }

    /// `#` lines start pages, titled with the rest of the line; lines before the
    /// first title still show.
    #[test]
    fn titles_start_pages() {
        let pages = parse("stray\n# One\na\nb\n# Two (F1 for more)\r\nc\n", &plain);
        let titles: Vec<_> = pages.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Help", "One", "Two (F1 for more)"]);
        assert_eq!(pages[1].lines.len(), 2);
    }

    /// Keys come from the player's own table, by Continuum's names or ours.
    #[test]
    fn keys_are_the_players_own() {
        let mut b = Bindings::default();
        assert_eq!(key_text(&b, "B_BULLET"), b.label(Action::Gun));
        assert_eq!(key_text(&b, "B_PLAY"), "F11");
        assert_eq!(key_text(&b, "B_show_fps"), b.label(Action::ShowFps));
        b.set(Action::Gun, Vec::new());
        assert_eq!(key_text(&b, "B_BULLET"), "(unbound)");
        // A key no table has, and a key named in words.
        assert_eq!(key_text(&b, "B_NOSUCHTHING"), "(no key)");
        assert_eq!(key_text(&b, "Esc"), "Esc");
        assert_eq!(key_text(&b, "PrintScrn"), "Print Screen");
    }

    /// Every key our own pages name is a key we have: a misspelt name would show
    /// "(no key)" to every player.
    #[test]
    fn our_pages_name_only_real_keys() {
        let b = Bindings::default();
        for file in [GENERAL, CLIENT] {
            for page in parse(file, &|n| key_text(&b, n)) {
                for line in &page.lines {
                    let t = text(line);
                    assert!(!t.contains("(no key)"), "{}: {t}", page.title);
                }
            }
        }
    }

    /// "Help text runs outside the help window's border." Our own pages are
    /// written to fit a box of `OWN_COLS`, so none of them needs the wrapping
    /// that keeps a zone's pages inside the box -- which is what keeps their
    /// columns lined up.
    #[test]
    fn our_pages_fit_without_wrapping() {
        let b = Bindings::default();
        for page in book(&b, None) {
            for line in &page.lines {
                assert!(
                    line.len() <= OWN_COLS,
                    "{}: {:?} is {} characters, over {OWN_COLS}",
                    page.title,
                    text(line),
                    line.len()
                );
            }
        }
    }

    /// A zone's help replaces our general pages, and our page about this client
    /// still follows it. A zone file that holds no pages at all is ignored.
    #[test]
    fn a_zones_help_comes_first_and_ours_follows() {
        let b = Bindings::default();
        let ours = book(&b, None);
        let last = ours.last().expect("pages").title.clone();
        assert_eq!(last, "Help: this client");

        let zone = book(
            &b,
            Some(b"# HELP: A Zone\n%WWelcome\n# HELP: Rules\nBe nice\n"),
        );
        let titles: Vec<_> = zone.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["HELP: A Zone", "HELP: Rules", "Help: this client"]);

        let empty = book(&b, Some(b""));
        assert_eq!(empty.len(), ours.len(), "an empty file is not a help file");
        // Help files are 8-bit text: a byte above 127 is one character.
        let latin = book(&b, Some(b"# Caf\xe9\nx\n"));
        assert_eq!(latin[0].title, "Caf\u{e9}");
    }

    /// Nothing is ever laid out wider than the box, however long a zone's line
    /// is, and a page taller than the screen goes on over more pages.
    #[test]
    fn a_zones_page_stays_inside_its_box() {
        let long = Page {
            title: "Long".into(),
            lines: (0..30)
                .map(|i| parse_line(&format!(" %Y?cmd{i}   %W{}", "word ".repeat(i + 3)), &plain))
                .chain([parse_line(&"x".repeat(200), &plain)])
                .collect(),
        };
        for (cols, rows) in [(40, 10), (60, 25), (16, 3)] {
            let shown = lay_out(std::slice::from_ref(&long), cols, rows);
            assert!(shown.len() > 1, "{cols}x{rows}: split over pages");
            for page in &shown {
                assert!(page.lines.len() <= rows, "{cols}x{rows}: too tall");
                for line in &page.lines {
                    assert!(line.len() <= cols, "{cols}x{rows}: {:?}", text(line));
                }
            }
            assert!(shown[1].title.ends_with("(continued)"));
        }
    }

    /// A wrapped command keeps its description lined up: the continuation starts
    /// under the description, not under the command.
    #[test]
    fn a_wrapped_line_hangs_under_its_description() {
        let line = parse_line(
            " %Y?go name       %WGo to that arena, or a public one if no name is given",
            &plain,
        );
        let rows = wrap_cells(&line, 40);
        assert!(rows.len() > 1);
        let column = text(&rows[0]).find("Go").expect("description");
        for row in &rows[1..] {
            let t = text(row);
            assert_eq!(t.len() - t.trim_start().len(), column, "{t:?}");
        }
    }
}
