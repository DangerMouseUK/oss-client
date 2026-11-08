//! What a typed chat line means, and player-name completion.
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
}
