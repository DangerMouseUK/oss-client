//! The F1 help: its pages, and the box they are laid out in.

use winit::keyboard::KeyCode;

use super::Sandbox;
use crate::game::input::{Action, Bindings, Stroke};
use crate::game::view::{Canvas, color};
use crate::gfx::Renderer;

impl Sandbox {
    /// Keys while the help is up: F1 turns the page, Esc or Enter closes it.
    pub(super) fn help_input(&mut self, strokes: &[Stroke]) {
        for stroke in strokes {
            match stroke.first_press() {
                // "F1 = More help": the pages turn, then the help goes away.
                Some(KeyCode::F1) => self.help_page += 1,
                Some(KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter) => {
                    self.help_open = false
                }
                _ => {}
            }
        }
        if self.help_page >= help_pages(&self.bindings, self.online()).len() {
            self.help_open = false;
            self.help_page = 0;
        }
    }

    /// The F1 help, which is where the key list lives in SubSpace and Continuum
    /// rather than in the menu. F1 turns the page and the last page closes it.
    pub(super) fn draw_help(&self, c: &Canvas, r: &mut Renderer) {
        let pages = help_pages(&self.bindings, self.online());
        let Some((title, lines)) = pages.get(self.help_page) else {
            return;
        };
        let w = HELP_COLS * 8.0;
        // Laid out against the box before the box is sized, so a line too long
        // for it becomes two lines and a taller box rather than text out through
        // the border. See [`PanelLines`].
        let page = help_layout(lines, w);
        let h = 20.0 + (page.len() as f32 + 2.0) * 12.0;
        let x = ((c.view_w - w) / 2.0).floor();
        let y = ((c.view_h - h) / 2.0).floor().max(4.0);
        c.panel(r, x, y, w, h);
        c.text_center(r, x + w / 2.0, y + 1.0, color::GREEN, title);
        c.separator(r, x, y + 13.0, w);
        page.draw(c, r, x, y + 18.0, 12.0);
        let footer = if self.help_page + 1 < pages.len() {
            "F1 = More help        Esc = Back to game"
        } else {
            "F1 or Esc = Back to game"
        };
        c.text_center(
            r,
            x + w / 2.0,
            y + 18.0 + (page.len() as f32 + 1.0) * 12.0,
            color::YELLOW,
            footer,
        );
    }
}

/// How many characters wide the help box is.
const HELP_COLS: f32 = 54.0;
/// How far in from the box's left edge a line of help text starts.
const HELP_INSET: f32 = 2.0;
/// Where a key's own name starts, and where its description does: the key gets
/// the first 21 characters of the line, as `Commands.hlp` lays it out.
const HELP_KEY_INSET: f32 = 4.0;
const HELP_TEXT_INSET: f32 = HELP_KEY_INSET + 21.0 * 8.0;

/// Text laid out for a bordered box, measured against the box as it goes in.
///
/// [`Canvas::panel`] draws a border and knows nothing about what will be put
/// inside it; the drawing code puts text at an offset from the box's left edge
/// and never asked how wide the box was. So the help pages ran seven of their
/// lines out through the right-hand border -- by up to eighteen pixels, on the
/// two pages the lead happened to read. Editing those seven strings would have
/// left the next panel free to do the same thing, which is why the measuring
/// lives here rather than in the strings.
///
/// Content is laid out first, wrapped to what the box can actually hold, and the
/// box is then made tall enough for the result -- rather than the box being
/// sized first and the text hoping to fit. [`PanelLines::over_the_edge`] is the
/// instrument the tests use, and it reports on content as laid out, so a line
/// added with a bad inset is caught even though wrapping would have saved a
/// line added with a long string.
#[derive(Default)]
pub(super) struct PanelLines {
    /// The inside width of the box, in pixels.
    w: f32,
    /// One entry per drawn line; each carries the pieces on that line, by their
    /// distance in from the box's left edge.
    rows: Vec<Vec<(f32, u32, String)>>,
}

impl PanelLines {
    pub(super) fn new(w: f32) -> Self {
        Self {
            w,
            rows: Vec::new(),
        }
    }

    /// How many characters of the 8x12 font fit between `inset` and the border.
    pub(super) fn cols(&self, inset: f32) -> usize {
        (((self.w - inset) / 8.0).floor()).max(0.0) as usize
    }

    fn len(&self) -> usize {
        self.rows.len()
    }

    /// A line of text, wrapped to whatever is left before the border.
    fn push(&mut self, inset: f32, colour: u32, text: &str) {
        for line in wrap_to(text, self.cols(inset)) {
            self.rows.push(vec![(inset, colour, line)]);
        }
    }

    /// A key and what it does, the key in its own column and the description
    /// wrapped into the rest of the line with its continuations lined up under
    /// itself.
    fn push_key(&mut self, key: &str, key_colour: u32, text: &str, colour: u32) {
        let mut wrapped = wrap_to(text, self.cols(HELP_TEXT_INSET)).into_iter();
        let first = wrapped.next().unwrap_or_default();
        self.rows.push(vec![
            (HELP_KEY_INSET, key_colour, key.to_string()),
            (HELP_TEXT_INSET, colour, first),
        ]);
        for line in wrapped {
            self.rows.push(vec![(HELP_TEXT_INSET, colour, line)]);
        }
    }

    /// Anything that would be drawn past the box's right-hand border, as the
    /// text and how many pixels past it goes.
    ///
    /// Always empty for content that came through [`PanelLines::push`], which
    /// wraps. It is not therefore a check that can never fail: a piece placed at
    /// an inset that leaves no room, or a key name longer than its column, is
    /// reported here, and the tests feed it both.
    #[cfg(test)]
    fn over_the_edge(&self) -> Vec<(&str, f32)> {
        self.rows
            .iter()
            .flatten()
            .filter_map(|(inset, _, text)| {
                let past = inset + text.chars().count() as f32 * 8.0 - self.w;
                (past > 0.0).then_some((text.as_str(), past))
            })
            .collect()
    }

    /// Draws every line, the first at `y` and the rest `line_h` apart.
    fn draw(&self, c: &Canvas, r: &mut Renderer, x: f32, y: f32, line_h: f32) {
        for (i, row) in self.rows.iter().enumerate() {
            for (inset, colour, text) in row {
                c.text(r, x + inset, y + i as f32 * line_h, *colour, text);
            }
        }
    }
}

/// Wraps `text` to `cols` characters, keeping what it is indented by on the
/// lines after the first.
///
/// Breaks at the last space that fits, and cuts mid-word where there is no space
/// to break at -- a word too long for the box still may not cross the border.
/// Deliberately not [`crate::game::chat::wrap`]: that one strips the leading spaces,
/// and the indent is what lines these pages' tables up.
fn wrap_to(text: &str, cols: usize) -> Vec<String> {
    if cols == 0 {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= cols {
        return vec![text.to_string()];
    }
    // Continuations keep the original's indent, unless that leaves so little
    // room that wrapping would never finish.
    let indent: String = chars.iter().take_while(|c| **c == ' ').collect();
    let indent = if indent.len() + 4 <= cols {
        indent
    } else {
        String::new()
    };
    let mut out = Vec::new();
    let mut rest: Vec<char> = chars;
    loop {
        let room = cols - if out.is_empty() { 0 } else { indent.len() };
        if rest.len() <= room {
            out.push(line_with(&indent, &rest, out.is_empty()));
            return out;
        }
        // The last space that still fits; none means a hard cut, which is the
        // only thing left that keeps the line inside the box.
        let cut = rest[..=room]
            .iter()
            .rposition(|c| *c == ' ')
            .filter(|p| *p > 0)
            .unwrap_or(room);
        out.push(line_with(&indent, &rest[..cut], out.is_empty()));
        rest = rest[cut..].to_vec();
        while rest.first() == Some(&' ') {
            rest.remove(0);
        }
        if rest.is_empty() {
            return out;
        }
    }
}

/// One wrapped line: the first keeps the text's own indent because it is still
/// in it, and the rest are given it back.
fn line_with(indent: &str, body: &[char], first: bool) -> String {
    let body: String = body.iter().collect();
    if first {
        body
    } else {
        format!("{indent}{body}")
    }
}

/// One line of a help page.
pub(super) enum Help {
    /// A group heading, as Continuum's `%p` marks.
    Heading(&'static str),
    /// A key and what it does. The key is read from the table, never written here.
    Key(String, &'static str),
    Text(&'static str),
}

/// The keyboard pages, in Continuum's groups and mostly its wording.
///
/// `Commands.hlp` carries these as `%Y %KB_BULLET %WFire guns`, where `%K` plus a
/// binding name is replaced with the player's own key when the page is drawn. So the
/// descriptions are written down and **the keys never are**: rebind something and
/// this follows. SubSpace's own help could not do that — its list is literal text
/// compiled into the binary — but SubSpace had no rebinding either.
const KEY_GROUPS: &[(&str, &[(Action, &str)])] = &[
    (
        "Movement",
        &[
            (Action::Left, "Turn left"),
            (Action::Right, "Turn right"),
            (Action::Forward, "Forward thrust"),
            (Action::Backward, "Reverse thrust"),
            (Action::Afterburner, "Afterburners"),
        ],
    ),
    (
        "Weapons",
        &[
            (Action::Gun, "Fire guns"),
            (Action::Bomb, "Fire bomb"),
            (Action::Mine, "Drop mine"),
            (Action::Thor, "Thor's hammer"),
            (Action::Burst, "Burst"),
        ],
    ),
    (
        "Toggle equipment",
        &[
            (Action::Stealth, "Stealth"),
            (Action::Cloak, "Cloak"),
            (Action::XRadar, "X-radar"),
            (Action::Antiwarp, "AntiWarp"),
            (Action::Multifire, "Multi-fire guns"),
        ],
    ),
    (
        "Actions",
        &[
            (Action::Repel, "Repel"),
            (Action::Warp, "Warp"),
            (Action::Portal, "Set portal"),
            (Action::Decoy, "Drop decoy"),
            (Action::Rocket, "Fire rocket"),
            (Action::Brick, "Drop brick"),
            (Action::Attach, "Attach/detach"),
            (Action::BigRadar, "Show map (while held)"),
        ],
    ),
    (
        "Miscellaneous",
        &[
            (Action::StatBox, "Stat box"),
            (Action::StatBoxUp, "Prev ticked player"),
            (Action::StatBoxDown, "Next ticked player"),
            (Action::ZoomIn, "Zoom in"),
            (Action::ZoomOut, "Zoom out"),
            (Action::ShowFps, "Frame rate"),
            (Action::Screenshot, "Take screenshot"),
        ],
    ),
];

/// Keys that cannot be rebound, so they are written out — as `Commands.hlp` writes
/// `%KEsc` and `%KPrintScrn` rather than a binding name.
const FIXED_KEYS: &[(&str, &str)] = &[
    ("Esc", "Menu display"),
    ("Enter", "Send message"),
    ("F1", "More help"),
];

/// One help page, laid out for a box `w` pixels wide.
///
/// Keys in yellow against their description in white, under a heading in dark
/// red: Continuum's own `%Y`/`%W`/`%p` colours for this page. Everything goes
/// through [`PanelLines`], so nothing can be put on a page without being
/// measured against the box it will be drawn in.
fn help_layout(lines: &[Help], w: f32) -> PanelLines {
    let mut page = PanelLines::new(w);
    for line in lines {
        match line {
            Help::Heading(text) => page.push(HELP_INSET, color::DARK_RED, text),
            Help::Key(key, text) => page.push_key(key, color::YELLOW, text, color::WHITE),
            Help::Text(text) => page.push(HELP_INSET, color::WHITE, text),
        }
    }
    page
}

/// The help pages. The first two are the keys, read out of `bindings`; the rest is
/// this client and its commands.
pub(super) fn help_pages(bindings: &Bindings, online: bool) -> Vec<(&'static str, Vec<Help>)> {
    let group = |name: &'static str| -> Vec<Help> {
        let (_, items) = KEY_GROUPS.iter().find(|(n, _)| *n == name).expect("group");
        let mut out = vec![Help::Heading(name)];
        for (action, text) in *items {
            // Attaching needs somebody to attach to, so it is not offered offline.
            if !online && matches!(action, Action::Attach) {
                continue;
            }
            out.push(Help::Key(bindings.label(*action), text));
        }
        out
    };

    let mut first: Vec<Help> = FIXED_KEYS
        .iter()
        .map(|(k, t)| Help::Key((*k).to_string(), t))
        .collect();
    first.extend(group("Movement"));
    first.extend(group("Weapons"));

    let mut second = group("Toggle equipment");
    second.extend(group("Actions"));
    second.extend(group("Miscellaneous"));

    let ours = vec![
        Help::Heading("Messages"),
        Help::Text("Type a message and press Enter to send it. There is"),
        Help::Text("no key to open the chat line: the characters go there"),
        Help::Text("as you type, as they do in SubSpace and Continuum."),
        Help::Text("  'message       = to your team (// also works)"),
        Help::Text("  /message       = to the ticked player"),
        Help::Text("  :name:message  = to that player by name. Tab"),
        Help::Text("                   finishes a name, :: repeats it."),
        Help::Text("  ;channel;msg   = to a chat channel"),
        Help::Text("  \"message       = to the enemy team"),
    ];

    let commands = vec![
        Help::Heading("This client answers these itself"),
        Help::Text("  ?go [arena]  = Switch arena (blank: next public)"),
        Help::Text("  ?namelen x   = Letters shown of a name (=x works)"),
        Help::Text("  ?lines x     = Message lines shown (=x also works)"),
        Help::Text("  ?status      = Your ship's status"),
        Help::Text("  ?news        = The zone's news"),
        Help::Text("  ?quit        = Back to the login screen, as Q does"),
        Help::Text(""),
        Help::Text("Anything else starting with ? goes to the zone, and"),
        Help::Text("the zone decides. ?help there lists what yours"),
        Help::Text("offers, which differs from zone to zone."),
        Help::Text(""),
        Help::Text("Macros are set on the settings screen (menu: O), and"),
        Help::Text("%selfname, %tickname, %freq, %bounty, %flags,"),
        Help::Text("%energy, %killer, %killed, %coord and %squad are"),
        Help::Text("replaced in anything you send."),
    ];

    vec![
        ("-= Help: keyboard =-", first),
        ("-= Help: keyboard =-", second),
        ("-= Help: this client =-", ours),
        ("-= Help: commands =-", commands),
    ]
}

#[cfg(test)]
mod tests {
    use super::{HELP_COLS, HELP_INSET, PanelLines, color, help_layout, help_pages};

    /// Nothing on any help page is drawn past the box's right-hand border.
    ///
    /// **This one cannot catch a long string, and saying otherwise was the first
    /// thing written here.** Laying a page out wraps it, so by the time this
    /// looks, an over-long line has already become two that fit: restoring one
    /// of the seven original strings leaves this test green, which is how that
    /// was found rather than assumed. What it does guard is the insets -- a
    /// piece placed where there is no room, which wrapping cannot save --
    /// and [`the_border_check_can_say_no`] shows it refusing both ways that can
    /// happen. The check that would have caught the seven is the next one.
    #[test]
    fn no_help_page_draws_outside_its_box() {
        let w = HELP_COLS * 8.0;
        for online in [false, true] {
            for (title, lines) in help_pages(&crate::game::input::Bindings::default(), online) {
                let page = help_layout(&lines, w);
                let over = page.over_the_edge();
                assert!(
                    over.is_empty(),
                    "{title} (online: {online}) draws past the border: {over:?}"
                );
            }
        }
    }

    /// "Help text runs outside the help window's border", on both the commands
    /// page and this client's page. Seven lines did, by up to eighteen pixels.
    ///
    /// This is the one that would have caught them: it asks whether any line
    /// had to be wrapped to fit, rather than whether the wrapped result fits.
    /// Wrapping is the safety net and keeps text inside the border whatever
    /// anyone writes; this says the pages do not need the net, which is what
    /// keeps the command tables lined up on their `=` signs.
    #[test]
    fn no_help_line_has_to_be_wrapped_to_fit() {
        let w = HELP_COLS * 8.0;
        for online in [false, true] {
            for (title, lines) in help_pages(&crate::game::input::Bindings::default(), online) {
                let page = help_layout(&lines, w);
                assert_eq!(
                    page.len(),
                    lines.len(),
                    "{title} (online: {online}) has a line too long for the box, \
                     so it wrapped: shorten it to {} characters",
                    PanelLines::new(w).cols(HELP_INSET)
                );
            }
        }
    }

    /// The control for the border check, which otherwise could not come back
    /// negative: a check that has never refused anything has not been tested.
    ///
    /// A piece placed at an inset with no room left is reported, and so is a key
    /// name longer than the column it is given -- the two ways content can still
    /// cross the border once the wrapping is doing its job.
    #[test]
    fn the_border_check_can_say_no() {
        let w = HELP_COLS * 8.0;
        let mut p = PanelLines::new(w);
        // Straight past the edge: pushed at an inset that leaves nothing.
        p.rows
            .push(vec![(w - 8.0, color::WHITE, "far too long".to_string())]);
        let over = p.over_the_edge();
        assert_eq!(over.len(), 1, "an over-wide piece is reported");
        assert_eq!(over[0].1, 11.0 * 8.0, "and by how much it is over");

        // A key name wider than its 21-character column.
        let mut p = PanelLines::new(w);
        p.push_key(&"K".repeat(60), color::YELLOW, "does a thing", color::WHITE);
        assert_eq!(p.over_the_edge().len(), 1, "a key too long for its column");
    }

    /// The safety net itself: a line too long for the box becomes more lines,
    /// each of which fits, rather than one line that does not.
    #[test]
    fn a_line_too_long_for_the_box_is_wrapped_rather_than_spilled() {
        let w = HELP_COLS * 8.0;
        let mut p = PanelLines::new(w);
        let budget = p.cols(HELP_INSET);
        assert_eq!(budget, 53, "53 characters fit beside a 2-pixel inset");
        let long =
            "Anything else starting with ? goes to the zone, and the zone decides what it does";
        assert!(long.len() > budget, "the line this test is about is over");
        p.push(HELP_INSET, color::WHITE, long);
        assert!(p.len() > 1, "it became more than one line");
        assert!(p.over_the_edge().is_empty(), "and none of them is over");

        // An indent is kept on the lines after the first, which is what holds
        // the command tables lined up when one of them does have to wrap.
        let mut p = PanelLines::new(w);
        p.push(
            HELP_INSET,
            color::WHITE,
            &format!("    {}", "word ".repeat(20)),
        );
        assert!(p.rows.len() > 1);
        for row in p.rows.iter().skip(1) {
            assert!(
                row[0].2.starts_with("    "),
                "continuations keep the indent"
            );
        }

        // A word with nowhere to break is cut rather than allowed out of the
        // box: staying inside the border wins over keeping the word whole.
        let mut p = PanelLines::new(w);
        p.push(HELP_INSET, color::WHITE, &"x".repeat(200));
        assert!(p.over_the_edge().is_empty(), "an unbreakable word is cut");
    }
}
