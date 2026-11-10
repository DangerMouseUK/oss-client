//! What a typed chat line means, name completion, and the chat log on screen.
//!
//! Continuum's prefixes:
//!
//! | Typed            | Sent as                                                   |
//! |------------------|-----------------------------------------------------------|
//! | `text`           | public                                                    |
//! | `'text`, `//text`| your team                                                 |
//! | `"text`          | the team of the player selected in the stat box           |
//! | `/text`          | private to the player selected in the stat box            |
//! | `:name:text`     | private (remote private through the biller if `name` isn't in the arena; `:#squad:text` reaches a squad) |
//! | `::text`         | private reply to whoever last messaged you privately      |
//! | `;text`, `;N;text` | chat channel 1 or N (`?chat=a,b` picks channels)        |
//! | `=N`             | change to team N                                          |
//! | `?command`, `*command` | public text the server treats as a command          |
//!
//! [`ChatLog`] is the box those lines are typed into and the log they land in. What
//! it does that Continuum does, with nullspace (MIT, `src/null/ChatController.cpp`)
//! as the reference for the real client:
//!
//! * the sender's name sits right-aligned in a column `?namelen` wide (1-24,
//!   default 10) and is truncated to it;
//! * long lines wrap on spaces, at most 16 lines each ([`wrap`]);
//! * the log shows `view_h / 100 + 1` lines, or the whole buffer while the menu is
//!   open (nullspace `Game.cpp`: `chat.display_full = menu_open`);
//! * typing `:` on a line that already starts with `:` fills in the name of a
//!   player who messaged you privately, and cycles back through the last five;
//! * Ctrl+Backspace clears the line, and a line is 250 characters at most.
//!
//! Scrollback (Page Up/Down with the menu open) and recall of lines you typed
//! earlier (Up/Down while typing) are **ours**: Continuum's log stops at what fits
//! on screen and it has no recall, so there is no behaviour to match. They are kept
//! out of the way of anything Continuum binds.

use oss_session::chat_type;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatPlayer {
    pub pid: i16,
    pub name: String,
    pub freq: i16,
}

/// What the line refers to.
#[derive(Debug, Clone, Default)]
pub struct ChatContext<'a> {
    pub players: &'a [ChatPlayer],
    /// The stat box selection.
    pub selected: Option<i16>,
    /// Name of the last player who messaged you privately.
    pub last_private: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Chat {
        kind: u8,
        target: i16,
        text: String,
        /// Shown in your own chat log, which servers don't echo (None for commands).
        echo: Option<String>,
    },
    SetFreq(i16),
    /// The line can't be sent; tell the player why.
    Problem(String),
}

fn find<'a>(ctx: &ChatContext<'a>, name: &str) -> Option<&'a ChatPlayer> {
    ctx.players
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Interprets a typed line (already trimmed, not empty).
pub fn parse(line: &str, ctx: &ChatContext) -> Line {
    let chat = |kind: u8, target: i16, text: &str| Line::Chat {
        kind,
        target,
        text: text.to_string(),
        echo: Some(text.to_string()),
    };
    if let Some(freq) = line
        .strip_prefix('=')
        .and_then(|f| f.trim().parse::<i16>().ok())
    {
        return Line::SetFreq(freq);
    }
    if let Some(t) = line.strip_prefix("//").or_else(|| line.strip_prefix('\'')) {
        return chat(chat_type::FREQ, 0, t);
    }
    if let Some(t) = line.strip_prefix('"') {
        return match ctx.selected {
            Some(pid) => chat(chat_type::ENEMY_FREQ, pid, t),
            None => Line::Problem("Select a player in the stat box first (Page Up/Down).".into()),
        };
    }
    if let Some(t) = line.strip_prefix('/') {
        return match ctx.selected {
            Some(pid) => chat(chat_type::PRIVATE, pid, t),
            None => Line::Problem("Select a player in the stat box first (Page Up/Down).".into()),
        };
    }
    if let Some(t) = line.strip_prefix("::") {
        return match ctx.last_private {
            Some(name) => private_to(name, t, ctx),
            None => Line::Problem("Nobody has messaged you privately yet.".into()),
        };
    }
    if let Some(rest) = line.strip_prefix(':')
        && let Some((name, t)) = rest.split_once(':')
        && !name.is_empty()
    {
        return private_to(name, t, ctx);
    }
    if let Some(t) = line.strip_prefix(';') {
        // ASSS takes "text" (channel 1) or "N;text".
        return Line::Chat {
            kind: chat_type::CHANNEL,
            target: 0,
            text: t.to_string(),
            echo: Some(match t.split_once(';') {
                Some((n, msg)) if n.parse::<u8>().is_ok() => format!("{n}:{msg}"),
                _ => format!("1:{t}"),
            }),
        };
    }
    let command = line.starts_with('?') || line.starts_with('*');
    Line::Chat {
        kind: chat_type::PUBLIC,
        target: 0,
        text: line.to_string(),
        echo: (!command).then(|| line.to_string()),
    }
}

fn private_to(name: &str, text: &str, ctx: &ChatContext) -> Line {
    match find(ctx, name) {
        Some(p) => Line::Chat {
            kind: chat_type::PRIVATE,
            target: p.pid,
            text: text.to_string(),
            echo: Some(text.to_string()),
        },
        // Not here: the biller routes ":name:text" (and ":#squad:text").
        None => Line::Chat {
            kind: chat_type::REMOTE_PRIVATE,
            target: 0,
            text: format!(":{name}:{text}"),
            echo: Some(format!("(to {name}) {text}")),
        },
    }
}

/// Who sent a private message: the sender's name for an in-arena private, or the
/// `(name)>` prefix a remote private carries.
pub fn private_sender(kind: u8, name: Option<&str>, text: &str) -> Option<String> {
    match kind {
        chat_type::PRIVATE => name.map(str::to_string),
        chat_type::REMOTE_PRIVATE => text
            .strip_prefix('(')
            .and_then(|t| t.split_once(")>"))
            .map(|(n, _)| n.to_string()),
        _ => None,
    }
}

/// Tab completion of the player name in `:partial` (or `::`-less `:name:` lines):
/// the `cycle`-th match, alphabetically, as a complete `:name:` prefix. `None` when
/// the line isn't a name being typed or nothing matches.
pub fn complete(line: &str, names: &[&str], cycle: usize) -> Option<String> {
    let partial = line.strip_prefix(':')?;
    if partial.contains(':') || partial.is_empty() && line.starts_with("::") {
        return None;
    }
    let lower = partial.to_ascii_lowercase();
    let mut matches: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| n.to_ascii_lowercase().starts_with(&lower))
        .collect();
    matches.sort_by_key(|n| n.to_ascii_lowercase());
    matches.dedup();
    let pick = matches.get(cycle % matches.len().max(1))?;
    Some(format!(":{pick}:"))
}

/// Longest line the chat box takes. nullspace: Continuum's own box holds 256 but
/// sends only 250, so 250 is what a line can actually carry.
pub const MAX_CHAT: usize = 250;
/// Messages kept. Continuum keeps 64; we keep more so the scrollback is worth
/// having (the extra lines are only ever reachable by scrolling).
pub const MAX_MESSAGES: usize = 200;
/// Default width of the sender column (`?namelen`).
pub const DEFAULT_NAMELEN: usize = 10;
/// Most lines one message wraps to (nullspace `WrapChat`).
const MAX_WRAP: usize = 16;
/// Lines shown with the menu open, which is when Continuum shows the whole buffer.
const FULL_LINES: usize = 64;
/// Private senders remembered for `:` cycling (nullspace `PrivateHistory`).
const PRIVATE_HISTORY: usize = 5;
/// Lines you typed that are kept for Up/Down recall (ours, not Continuum's).
const SENT_HISTORY: usize = 32;

/// Chat colours by message type, as `(sender column, text)`; the font's colour rows
/// are the same eight in the same order as nullspace's `TextColor`, so these are
/// nullspace's `ChatController::Render` choices directly.
pub fn colors(kind: u8) -> (u32, u32) {
    use super::view::color::{BLUE, DARK_RED, FUCHSIA, GREEN, RED, WHITE, YELLOW};
    match kind {
        chat_type::PUBLIC | chat_type::PUBLIC_MACRO => (BLUE, BLUE),
        chat_type::FREQ => (YELLOW, YELLOW),
        // The only type drawn in two colours: a green name, then blue text.
        chat_type::ENEMY_FREQ => (GREEN, BLUE),
        chat_type::PRIVATE | chat_type::ARENA | chat_type::REMOTE_PRIVATE => (GREEN, GREEN),
        chat_type::WARNING | chat_type::SERVER_ERROR => (DARK_RED, DARK_RED),
        chat_type::CHANNEL => (RED, RED),
        // Type 79 is the fuchsia line some zones use; oss-proto has no name for it.
        79 => (FUCHSIA, FUCHSIA),
        _ => (WHITE, WHITE),
    }
}

/// Whether this type is drawn with a sender column. Arena messages, both red
/// warnings, channel lines and remote privates run the full width in Continuum
/// (a remote private carries its own `(name)>` inside the text).
fn has_sender(kind: u8) -> bool {
    matches!(
        kind,
        chat_type::PUBLIC
            | chat_type::PUBLIC_MACRO
            | chat_type::FREQ
            | chat_type::ENEMY_FREQ
            | chat_type::PRIVATE
    )
}

/// A line in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Who sent it, drawn in the sender column; `None` runs the full width.
    pub sender: Option<String>,
    pub text: String,
    pub color: u32,
    pub sender_color: u32,
}

impl Message {
    /// A line the client itself writes (no sender column).
    pub fn local(text: impl Into<String>, color: u32) -> Self {
        Self {
            sender: None,
            text: text.into(),
            color,
            sender_color: color,
        }
    }

    /// A line that arrived (or was echoed) as chat type `kind`.
    pub fn received(kind: u8, sender: Option<String>, text: impl Into<String>) -> Self {
        let (sender_color, color) = colors(kind);
        Self {
            sender: sender.filter(|_| has_sender(kind)),
            text: text.into(),
            color,
            sender_color,
        }
    }
}

/// One drawn row: the sender column (already padded, empty when there is none)
/// followed by the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub prefix: String,
    pub prefix_color: u32,
    pub text: String,
    pub color: u32,
}

/// Wraps `text` to `width` characters the way Continuum does (nullspace
/// `WrapChat`): break at the last space that fits, drop the spaces broken on, give
/// up after 16 lines. With `skip_spaces` false — the line being typed — it cuts at
/// the limit instead, so the cursor stays with the characters.
pub fn wrap(text: &str, width: usize, skip_spaces: bool) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if width == 0 {
        return vec![String::new()];
    }
    let mut start = 0;
    if skip_spaces {
        while start < chars.len() && chars[start] == ' ' {
            start += 1;
        }
    }
    let body = &chars[start..];
    let size = body.len();
    let take = |a: usize, b: usize| body[a..b].iter().collect::<String>();
    if size < width {
        return vec![take(0, size)];
    }
    let mut lines = Vec::new();
    let mut last_end = 0;
    for _ in 0..MAX_WRAP {
        let mut end = last_end + width;
        if end >= size {
            lines.push(take(last_end, size));
            break;
        }
        if skip_spaces {
            // Back up to a space: past the last one in a run, or onto the previous
            // space. Running off the front leaves the hard cut below.
            let mut e = end as isize;
            if body[end] == ' ' {
                while e >= 0 && body[e as usize] == ' ' {
                    e -= 1;
                }
                e += 1;
            } else {
                while e >= 0 && body[e as usize] != ' ' {
                    e -= 1;
                }
            }
            end = e.max(0) as usize;
        }
        if end <= last_end {
            // No space to break on: cut at the limit.
            end = last_end + width;
        }
        lines.push(take(last_end, end));
        last_end = end;
        if skip_spaces {
            while last_end < size && body[last_end] == ' ' {
                last_end += 1;
            }
        }
    }
    lines
}

/// The chat type the line being typed would be sent as (nullspace
/// `ChatController::GetInputType`): `'` and `//` team, `/` and `:name:` private,
/// `;` channel, `"` the selected player's team, anything else public.
pub fn input_kind(line: &str) -> u8 {
    let mut cs = line.chars();
    match cs.next() {
        None => chat_type::PUBLIC,
        Some('\'') => chat_type::FREQ,
        Some('/') if line.starts_with("//") => chat_type::FREQ,
        Some('/') => chat_type::PRIVATE,
        Some(':') if line[1..].contains(':') => chat_type::PRIVATE,
        Some(';') => chat_type::CHANNEL,
        Some('"') => chat_type::ENEMY_FREQ,
        _ => chat_type::PUBLIC,
    }
}

/// The colour the line being typed is drawn in. Continuum colours fewer of these
/// than it colours the log: only team, private and channel lines stand out
/// (nullspace `ChatController::Render`).
pub fn input_color(line: &str) -> u32 {
    use super::view::color::{GREEN, RED, WHITE, YELLOW};
    match input_kind(line) {
        chat_type::FREQ => YELLOW,
        chat_type::PRIVATE | chat_type::REMOTE_PRIVATE => GREEN,
        chat_type::CHANNEL => RED,
        _ => WHITE,
    }
}

/// The chat box and the log above it.
#[derive(Debug, Clone)]
pub struct ChatLog {
    entries: Vec<Message>,
    /// The line being typed, if the box is open.
    pub input: Option<String>,
    /// Rows scrolled back from the newest (ours; Continuum cannot scroll).
    scroll: usize,
    /// Lines sent earlier, oldest first.
    sent: Vec<String>,
    /// How far back Up/Down has walked `sent`.
    recall: Option<usize>,
    /// Names that messaged you privately, newest first.
    privates: Vec<String>,
    /// Width of the sender column (`?namelen`).
    pub namelen: usize,
    /// Tab completion in progress: the line as typed and how many Tabs so far.
    completing: Option<(String, usize)>,
}

impl Default for ChatLog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            input: None,
            scroll: 0,
            sent: Vec::new(),
            recall: None,
            privates: Vec::new(),
            namelen: DEFAULT_NAMELEN,
            completing: None,
        }
    }
}

impl ChatLog {
    pub fn push(&mut self, m: Message) {
        self.entries.push(m);
        if self.entries.len() > MAX_MESSAGES {
            self.entries.remove(0);
        }
        // Keep the view where the reader put it while older lines age out.
        if self.scroll > 0 {
            self.scroll += 1;
        }
    }

    /// Remembers a private sender for `::` and for `:` cycling.
    pub fn saw_private(&mut self, name: &str) {
        self.privates.retain(|n| !n.eq_ignore_ascii_case(name));
        self.privates.insert(0, name.to_string());
        self.privates.truncate(PRIVATE_HISTORY);
    }

    /// Whoever last messaged you privately (`::text`).
    pub fn last_private(&self) -> Option<&str> {
        self.privates.first().map(String::as_str)
    }

    /// `?namelen N`: sets the sender column width, as Continuum does (1-24).
    /// Returns the line to show in reply.
    pub fn set_namelen(&mut self, arg: &str) -> String {
        if let Ok(n) = arg.trim().parse::<usize>()
            && n > 0
        {
            self.namelen = n.min(24);
        }
        format!("Message Name Length: {}", self.namelen)
    }

    /// Rows the log shows: Continuum's `view_h / 100 + 1`, or the whole buffer
    /// while the menu is open.
    pub fn visible_rows(view_h: f32, menu_open: bool) -> usize {
        if menu_open {
            FULL_LINES
        } else {
            (view_h.max(0.0) as usize / 100) + 1
        }
    }

    /// The rows to draw, oldest first, for a log `width` characters wide.
    pub fn lines(&self, width: usize, rows: usize) -> Vec<LogLine> {
        let name_col = self.namelen + 2;
        let mut out: Vec<LogLine> = Vec::new();
        let wanted = rows + self.scroll;
        for m in self.entries.iter().rev() {
            if out.len() >= wanted {
                break;
            }
            let (prefix, text_width) = match &m.sender {
                Some(n) => {
                    let name: String = n.chars().take(self.namelen).collect();
                    (
                        format!("{name:>0$}> ", self.namelen),
                        width.saturating_sub(name_col).max(1),
                    )
                }
                None => (String::new(), width.max(1)),
            };
            for text in wrap(&m.text, text_width, true).into_iter().rev() {
                out.push(LogLine {
                    prefix: prefix.clone(),
                    prefix_color: m.sender_color,
                    text,
                    color: m.color,
                });
            }
        }
        // `out` is newest-first; drop what is scrolled past and put it back in order.
        let scroll = self.scroll.min(out.len().saturating_sub(1));
        let end = (scroll + rows).min(out.len());
        let mut window: Vec<LogLine> = out[scroll..end].to_vec();
        window.reverse();
        window
    }

    /// Page Up/Down with the menu open (ours, not Continuum's).
    pub fn scroll_by(&mut self, rows: isize) {
        let max = self.entries.len().saturating_sub(1);
        self.scroll = (self.scroll as isize + rows).clamp(0, max as isize) as usize;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
    }

    /// Opens the chat box.
    pub fn start_typing(&mut self) {
        self.input = Some(String::new());
        self.recall = None;
        self.completing = None;
    }

    /// Escape: closes the box, keeping nothing.
    pub fn cancel(&mut self) {
        self.input = None;
        self.recall = None;
        self.completing = None;
    }

    /// A typed character. `:` on a line that already starts with `:` fills in a
    /// recent private sender instead, as Continuum does.
    pub fn type_char(&mut self, c: char) {
        let Some(line) = self.input.as_mut() else {
            return;
        };
        if c.is_control() || line.chars().count() >= MAX_CHAT {
            return;
        }
        if c == ':' && line.starts_with(':') {
            let current: String = line[1..].split(':').next().unwrap_or("").to_string();
            if let Some(name) = cycle_private(&self.privates, &current) {
                *line = format!(":{name}:");
                self.completing = None;
                return;
            }
        }
        line.push(c);
        self.completing = None;
    }

    /// Backspace; with Ctrl the whole line goes (nullspace).
    pub fn backspace(&mut self, ctrl: bool) {
        if let Some(line) = self.input.as_mut() {
            if ctrl {
                line.clear();
            } else {
                line.pop();
            }
            self.completing = None;
        }
    }

    /// Up/Down while typing: walks the lines you sent earlier (ours).
    pub fn recall(&mut self, back: bool) {
        if self.input.is_none() || self.sent.is_empty() {
            return;
        }
        let next = match (self.recall, back) {
            (None, true) => Some(self.sent.len() - 1),
            (None, false) => None,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < self.sent.len() => Some(i + 1),
            (Some(_), false) => None,
        };
        self.recall = next;
        self.input = Some(match next {
            Some(i) => self.sent[i].clone(),
            None => String::new(),
        });
        self.completing = None;
    }

    /// Enter: takes the line, closes the box and remembers it for recall.
    pub fn submit(&mut self) -> String {
        let line = self.input.take().unwrap_or_default();
        self.recall = None;
        self.completing = None;
        let line = line.trim().to_string();
        if !line.is_empty() && self.sent.last() != Some(&line) {
            self.sent.push(line.clone());
            if self.sent.len() > SENT_HISTORY {
                self.sent.remove(0);
            }
        }
        line
    }

    /// Tab: completes `:partial` to a player name, cycling on repeats.
    pub fn complete_name(&mut self, names: &[&str]) {
        let Some(line) = self.input.clone() else {
            return;
        };
        let (base, n) = match self.completing.take() {
            Some((base, n)) => (base, n + 1),
            None => (line, 0),
        };
        if let Some(done) = complete(&base, names, n) {
            self.input = Some(done);
            self.completing = Some((base, n));
        }
    }
}

/// The private sender before `current` in the history, newest first and wrapping;
/// with no `current` (the line is just `:`) the newest one. nullspace
/// `PrivateHistory::GetPrevious`.
fn cycle_private(privates: &[String], current: &str) -> Option<String> {
    if privates.is_empty() {
        return None;
    }
    if current.is_empty() {
        return privates.first().cloned();
    }
    let at = privates
        .iter()
        .position(|n| n.eq_ignore_ascii_case(current))?;
    privates.get((at + 1) % privates.len()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn players() -> Vec<ChatPlayer> {
        [(1, "Alpha", 0), (2, "alfred", 1), (3, "Bravo", 1)]
            .into_iter()
            .map(|(pid, name, freq)| ChatPlayer {
                pid,
                name: name.into(),
                freq,
            })
            .collect()
    }

    fn sent(line: &str, ctx: &ChatContext) -> (u8, i16, String) {
        match parse(line, ctx) {
            Line::Chat {
                kind, target, text, ..
            } => (kind, target, text),
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn every_prefix() {
        let ps = players();
        let ctx = ChatContext {
            players: &ps,
            selected: Some(3),
            last_private: Some("alfred"),
        };
        assert_eq!(sent("hello", &ctx), (chat_type::PUBLIC, 0, "hello".into()));
        assert_eq!(sent("'go", &ctx), (chat_type::FREQ, 0, "go".into()));
        assert_eq!(sent("//go", &ctx), (chat_type::FREQ, 0, "go".into()));
        assert_eq!(sent("\"hi", &ctx), (chat_type::ENEMY_FREQ, 3, "hi".into()));
        assert_eq!(sent("/psst", &ctx), (chat_type::PRIVATE, 3, "psst".into()));
        assert_eq!(
            sent(":ALPHA:yo", &ctx),
            (chat_type::PRIVATE, 1, "yo".into())
        );
        assert_eq!(sent("::back", &ctx), (chat_type::PRIVATE, 2, "back".into()));
        assert_eq!(
            sent(":Zed:far", &ctx),
            (chat_type::REMOTE_PRIVATE, 0, ":Zed:far".into())
        );
        assert_eq!(
            sent(":#squad:all", &ctx),
            (chat_type::REMOTE_PRIVATE, 0, ":#squad:all".into())
        );
        assert_eq!(sent(";hi", &ctx), (chat_type::CHANNEL, 0, "hi".into()));
        assert_eq!(sent(";2;hi", &ctx), (chat_type::CHANNEL, 0, "2;hi".into()));
        assert_eq!(parse("=3", &ctx), Line::SetFreq(3));
        assert!(matches!(
            parse("?help", &ctx),
            Line::Chat {
                kind: chat_type::PUBLIC,
                echo: None,
                ..
            }
        ));
        let nobody = ChatContext {
            players: &ps,
            ..Default::default()
        };
        assert!(matches!(parse("/x", &nobody), Line::Problem(_)));
        assert!(matches!(parse("::x", &nobody), Line::Problem(_)));
    }

    #[test]
    fn senders_and_completion() {
        assert_eq!(
            private_sender(chat_type::PRIVATE, Some("Bravo"), "hi"),
            Some("Bravo".into())
        );
        assert_eq!(
            private_sender(chat_type::REMOTE_PRIVATE, None, "(Far Away)> hi"),
            Some("Far Away".into())
        );
        assert_eq!(private_sender(chat_type::PUBLIC, Some("x"), "hi"), None);

        let names = ["Alpha", "alfred", "Bravo"];
        assert_eq!(complete(":al", &names, 0).as_deref(), Some(":alfred:"));
        assert_eq!(complete(":al", &names, 1).as_deref(), Some(":Alpha:"));
        assert_eq!(complete(":al", &names, 2).as_deref(), Some(":alfred:"));
        assert_eq!(complete(":", &names, 2).as_deref(), Some(":Bravo:"));
        assert_eq!(complete(":zz", &names, 0), None);
        assert_eq!(complete(":Alpha:hi", &names, 0), None);
        assert_eq!(complete("hello", &names, 0), None);
    }

    #[test]
    fn wrapping_follows_continuum() {
        // Short lines are left alone, and a line exactly the width still fits.
        assert_eq!(wrap("hello", 10, true), vec!["hello"]);
        assert_eq!(wrap("0123456789", 10, true), vec!["0123456789"]);
        assert_eq!(wrap("01234567890", 10, true), vec!["0123456789", "0"]);
        // Breaks on the last space that fits, and the space itself goes.
        assert_eq!(
            wrap("the quick brown fox jumps", 10, true),
            vec!["the quick", "brown fox", "jumps"]
        );
        // A word longer than the line is cut at the limit rather than lost.
        assert_eq!(
            wrap("aaaaaaaaaaaaaaaaaa", 8, true),
            vec!["aaaaaaaa", "aaaaaaaa", "aa"]
        );
        // The line being typed cuts at the edge instead of at a space.
        assert_eq!(
            wrap("the quick brown", 8, false),
            vec!["the quic", "k brown"]
        );
        // Leading spaces are trimmed for a message, kept for the typed line.
        assert_eq!(wrap("   hi", 4, true), vec!["hi"]);
    }

    #[test]
    fn sender_column_and_colours() {
        let mut log = ChatLog::default();
        log.push(Message::received(
            chat_type::PUBLIC,
            Some("Bob".into()),
            "hello there",
        ));
        let lines = log.lines(40, 4);
        assert_eq!(lines.len(), 1);
        // Right-aligned in a 10-wide column, as Continuum draws it.
        assert_eq!(lines[0].prefix, "       Bob> ");
        assert_eq!(lines[0].color, super::super::view::color::BLUE);
        // A long name is cut to the column.
        log.push(Message::received(
            chat_type::PRIVATE,
            Some("AVeryLongPlayerName".into()),
            "psst",
        ));
        let lines = log.lines(40, 4);
        assert_eq!(lines[1].prefix, "AVeryLongP> ");
        // Arena, channel and remote-private lines run the full width.
        log.push(Message::received(
            chat_type::CHANNEL,
            Some("Bob".into()),
            "1:hi",
        ));
        let lines = log.lines(40, 4);
        assert_eq!(lines[2].prefix, "");
        assert_eq!(lines[2].color, super::super::view::color::RED);
        // ?namelen changes the column; out-of-range values are clamped.
        assert_eq!(log.set_namelen("3"), "Message Name Length: 3");
        assert_eq!(log.set_namelen("99"), "Message Name Length: 24");
        assert_eq!(log.set_namelen("x"), "Message Name Length: 24");
    }

    #[test]
    fn log_window_and_scrollback() {
        let mut log = ChatLog::default();
        for i in 0..20 {
            log.push(Message::local(format!("line {i}"), 0));
        }
        // Continuum's count: one row per 100 pixels of height, plus one.
        assert_eq!(ChatLog::visible_rows(480.0, false), 5);
        assert_eq!(ChatLog::visible_rows(1080.0, false), 11);
        assert_eq!(ChatLog::visible_rows(480.0, true), 64);
        let shown = log.lines(40, 3);
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[2].text, "line 19");
        // Scrolling back moves the window, and a new line keeps it put.
        log.scroll_by(2);
        let shown = log.lines(40, 3);
        assert_eq!(shown[2].text, "line 17");
        log.push(Message::local("line 20", 0));
        let shown = log.lines(40, 3);
        assert_eq!(shown[2].text, "line 17");
        log.scroll_to_bottom();
        assert_eq!(log.lines(40, 3)[2].text, "line 20");
    }

    #[test]
    fn typing_history_and_private_cycling() {
        let mut log = ChatLog::default();
        log.start_typing();
        for c in "hello".chars() {
            log.type_char(c);
        }
        log.backspace(false);
        assert_eq!(log.input.as_deref(), Some("hell"));
        // Ctrl+Backspace clears the line (nullspace).
        log.backspace(true);
        assert_eq!(log.input.as_deref(), Some(""));
        for c in "first".chars() {
            log.type_char(c);
        }
        assert_eq!(log.submit(), "first");
        log.start_typing();
        for c in "second".chars() {
            log.type_char(c);
        }
        assert_eq!(log.submit(), "second");
        // Ours: Up walks back through what was sent, Down returns.
        log.start_typing();
        log.recall(true);
        assert_eq!(log.input.as_deref(), Some("second"));
        log.recall(true);
        assert_eq!(log.input.as_deref(), Some("first"));
        log.recall(false);
        assert_eq!(log.input.as_deref(), Some("second"));
        log.recall(false);
        assert_eq!(log.input.as_deref(), Some(""));
        log.cancel();

        // Continuum: ':' on a line that starts with ':' fills in a recent private
        // sender, and cycles back through the last five.
        log.saw_private("Alpha");
        log.saw_private("Bravo");
        assert_eq!(log.last_private(), Some("Bravo"));
        log.start_typing();
        log.type_char(':');
        assert_eq!(log.input.as_deref(), Some(":"));
        log.type_char(':');
        assert_eq!(log.input.as_deref(), Some(":Bravo:"));
        log.type_char(':');
        assert_eq!(log.input.as_deref(), Some(":Alpha:"));
        log.type_char(':');
        assert_eq!(log.input.as_deref(), Some(":Bravo:"));
    }

    #[test]
    fn typed_line_kinds() {
        assert_eq!(input_kind(""), chat_type::PUBLIC);
        assert_eq!(input_kind("'go"), chat_type::FREQ);
        assert_eq!(input_kind("//go"), chat_type::FREQ);
        assert_eq!(input_kind("/psst"), chat_type::PRIVATE);
        assert_eq!(input_kind(":Bob:hi"), chat_type::PRIVATE);
        // A name still being typed is not a private line yet.
        assert_eq!(input_kind(":Bo"), chat_type::PUBLIC);
        assert_eq!(input_kind(";5;hi"), chat_type::CHANNEL);
        assert_eq!(input_kind("\"hi"), chat_type::ENEMY_FREQ);
        assert_eq!(input_kind("?help"), chat_type::PUBLIC);
    }
}
