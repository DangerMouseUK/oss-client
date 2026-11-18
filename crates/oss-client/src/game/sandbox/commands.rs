//! The commands this client answers itself instead of sending them to the zone,
//! and the sandbox's own offline commands.

use oss_session::chat_type;
use oss_sim::Prize;

use super::Sandbox;
use crate::game::view::color;

impl Sandbox {
    /// Commands Continuum answers itself instead of sending on (nullspace
    /// `ChatController::HandleInputCommands`): `?go`, `?namelen`, `?lines` and
    /// `?status`.
    /// `?news` is ours — Continuum opens the news in a window of its own, which we
    /// have no equivalent of, so it gets a command and a menu key. Returns true
    /// when the line was handled here.
    pub(super) fn client_command(&mut self, line: &str) -> bool {
        let (word, rest) = match line.split_once(' ') {
            Some((w, r)) => (w, r.trim()),
            None => (line, ""),
        };
        // Both forms, because both originals taught one: nullspace takes
        // "?namelen x" with a space, and SubSpace's own F1 help documents
        // "?namelen=x". A player who types the one we did not take got no reply at
        // all, because the line went to the zone, which has no such command.
        let (word, rest) = match word.split_once('=') {
            Some((w, r)) if rest.is_empty() => (w, r.trim()),
            _ => (word, rest),
        };
        match word.to_ascii_lowercase().as_str() {
            // Offline there is no arena and no ship to report on, so these fall
            // through to the sandbox's own command list instead of being swallowed.
            "?go" if self.online() => {
                if let Some(net) = self.net.as_mut() {
                    net.session.change_arena(rest);
                    net.flush();
                }
                self.message(
                    if rest.is_empty() {
                        "Entering a public arena...".to_string()
                    } else {
                        format!("Entering arena {rest}...")
                    },
                    color::GREEN,
                );
                true
            }
            "?namelen" if !rest.is_empty() => {
                let reply = self.chat.set_namelen(rest);
                self.message(reply, color::GREEN);
                true
            }
            // Continuum's own: "Changes number of displayed message lines". With
            // no number it says what it is, so a player who guesses the command
            // learns the setting rather than having the line go to the zone,
            // which has no such command and answers nothing.
            "?lines" => {
                let reply = self.chat.set_lines(rest);
                self.message(reply, color::GREEN);
                true
            }
            "?status" if rest.is_empty() && self.online() => {
                self.show_status();
                true
            }
            // Ours, and the same thing in a zone as offline: leave for the front
            // end, exactly as Q on the menu does. Neither the original client nor
            // ASSS has a `?quit`, so no zone is waiting for one.
            "?quit" if rest.is_empty() => {
                self.request_leave();
                true
            }
            "?news" if rest.is_empty() => {
                if self.news.is_empty() {
                    self.message("This zone has no news.", color::YELLOW);
                } else {
                    self.news_open = true;
                    self.news_scroll = 0;
                }
                true
            }
            _ => false,
        }
    }

    /// How this session is encrypted, for a player who wants to know.
    ///
    /// A session that offered OpenSec and got no answer in time falls back to
    /// VIE, which is no encryption worth the name and pins no server key. That
    /// is said once in the connecting status and once in a log line, and both
    /// are gone the moment login finishes -- so a player it happened to could
    /// not find out, and nor could anyone diagnosing it afterwards.
    pub(super) fn connection_line(&self) -> Option<String> {
        let (server, cipher) = self.net.as_ref()?.connection();
        Some(format!("Connection: {server} ({cipher})"))
    }

    /// `?status`: how far each upgradeable stat is toward its maximum, counted in
    /// whole upgrades as Continuum counts them (nullspace `GetShipStatusPercent`),
    /// and how the session is encrypted.
    fn show_status(&mut self) {
        if let Some(line) = self.connection_line() {
            self.chat_message(chat_type::ARENA, None, line);
        }
        let me = self.me();
        if me.kind as usize >= self.world.settings.ships.len() {
            return;
        }
        let s = &self.world.settings.ships[me.kind as usize];
        let stats = me.stats;
        let shrapnel = me.shrapnel;
        let pct = |upgrade: u32, maximum: u32, current: u32| {
            if upgrade == 0 {
                return 100;
            }
            let max_upgrades = (maximum / upgrade).max(1);
            (current / upgrade) * 100 / max_upgrades
        };
        let line = format!(
            "Recharge:{}%  Thruster:{}%  Speed:{}%  Rotation:{}%  Shrapnel:{shrapnel}",
            pct(s.upgrade_recharge, s.maximum_recharge, stats.recharge),
            pct(s.upgrade_thrust, s.maximum_thrust, stats.thrust),
            pct(s.upgrade_speed, s.maximum_speed, stats.speed),
            pct(s.upgrade_rotation, s.maximum_rotation, stats.rotation),
        );
        self.chat_message(chat_type::ARENA, None, line);
    }

    /// Offline `?commands`.
    pub(super) fn local_command(&mut self, line: &str) {
        let mut words = line.split_whitespace();
        match words.next().map(str::to_ascii_lowercase).as_deref() {
            Some("?help") => {
                self.message(
                    "Commands: ?ship <1-8>, ?prize <id|name>, ?quit",
                    color::GREEN,
                );
            }
            Some("?ship") => match words
                .next()
                .and_then(|w| w.parse::<u8>().ok())
                .filter(|n| (1..=8).contains(n))
            {
                Some(n) => self.request_ship(n - 1),
                None => self.message("Usage: ?ship <1-8>", color::YELLOW),
            },
            Some("?prize") => {
                let arg = words.next().unwrap_or("");
                let prize = arg.parse::<i32>().ok().or_else(|| {
                    Prize::ALL
                        .iter()
                        .find(|p| format!("{p:?}").eq_ignore_ascii_case(arg))
                        .map(|p| *p as i32)
                });
                match prize {
                    Some(id) if (-28..=28).contains(&id) => {
                        let mut ev = Vec::new();
                        let me = self.me_index();
                        self.world.apply_prize(me, id, &mut ev);
                        self.handle_events(&ev);
                    }
                    _ => self.message("Usage: ?prize <1-28 or name, e.g. Thor>", color::YELLOW),
                }
            }
            Some("?quit") => self.request_leave(),
            Some(c) if c.starts_with('?') => {
                self.message(format!("Unknown command {c}; try ?help"), color::YELLOW)
            }
            _ => self.message(format!("> {line}"), color::WHITE),
        }
    }
}
