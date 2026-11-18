//! The Esc menu: its rows, its keys, and leaving the game from it.

use oss_session::SPECTATOR;
use winit::keyboard::KeyCode;

use super::Sandbox;
use crate::game::input::{Action, Stroke};
use crate::game::start_spots::Rect;
use crate::game::view::{Canvas, color};
use crate::gfx::Renderer;

impl Sandbox {
    /// Keys while the menu is up.
    pub(super) fn menu_input(&mut self, edges: &[Action], strokes: &[Stroke]) {
        // The menu lists F2 and F12, so they work while it is up -- they used to
        // be thrown away unread here -- and it stays up, because in Continuum the
        // menu is where display switches are flipped. F11 changes ship, so it
        // puts the menu away as choosing a ship does.
        for e in edges {
            match e {
                Action::StatBox => self.statbox.cycle(),
                Action::ShowFps => self.fps_view = self.fps_view.next(),
                Action::Spectate => {
                    self.toggle_spectator();
                    self.menu = false;
                }
                _ => {}
            }
        }
        for stroke in strokes {
            match stroke.key {
                // Keys that type nothing never reach Continuum's menu, so they
                // leave it up: F1 opens the help, and PgUp/PgDn scroll the log
                // the menu has put on screen (ours; Continuum cannot scroll).
                Some(KeyCode::PageUp) => self.chat.scroll_by(1),
                Some(KeyCode::PageDown) => self.chat.scroll_by(-1),
                _ => {}
            }
            if stroke.first_press() == Some(KeyCode::F1) {
                self.help_open = true;
                self.help_page = 0;
                self.menu = false;
            }
            // The menu answers to typed characters, and whichever one comes is
            // the last: "Any other key to resume game", which then types as
            // usual.
            if let Some(c) = stroke.text {
                let handled = self.menu_key(c);
                self.menu = false;
                if !handled {
                    self.chat.type_char(c);
                }
            }
        }
        if !self.menu {
            self.chat.scroll_to_bottom();
        }
    }

    /// Q on the menu, or `?quit`: leave the game and go back to the front end.
    ///
    /// Not out of the program -- that is the front end's own Quit. nullspace does
    /// the same, returning its `Run` loop to `GameScreen::MainMenu` when `Game`
    /// ends, and only the main menu breaks the loop.
    pub(super) fn request_leave(&mut self) {
        if self.leaving {
            return;
        }
        self.leaving = true;
        if !self.can_leave() {
            // The original client's own wording, shown while it waits
            // (`SUBSPACE.EXE`, and nullspace's `Game::Update`).
            self.message("Exiting game...", color::YELLOW);
        }
    }

    /// Whether leaving is allowed yet: half your energy, as both originals ask, so
    /// that a losing fight cannot be quit out of. Spectators and a dead ship may go
    /// at once, and offline there is nobody to dodge.
    pub(super) fn can_leave(&self) -> bool {
        let me = self.me();
        may_leave(
            self.online() && !self.spectating && me.is_alive(),
            me.energy_points(),
            me.stats.max_energy as i32,
        )
    }

    /// The keystroke a click at this point means, where it lands on a menu row.
    ///
    /// `None` when the menu is down or the click missed every row -- and a miss
    /// inside the menu is still a miss, not "any other key", because a pointer
    /// that dismisses the menu wherever it lands would make the ships
    /// unclickable at the edges.
    pub fn click(&mut self, x: f32, y: f32) -> Option<Stroke> {
        if self.click_arena_list(x, y) {
            return None;
        }
        if !self.menu {
            return None;
        }
        self.menu_spots
            .iter()
            .find(|(rect, _)| rect.holds(x, y))
            .map(|(_, stroke)| *stroke)
    }

    /// One of the menu's own keys, or `false` for "any other key".
    fn menu_key(&mut self, ch: char) -> bool {
        let Some(choice) = menu_choice(ch, !self.news.is_empty(), self.online()) else {
            return false;
        };
        match choice {
            MenuChoice::Quit => self.request_leave(),
            MenuChoice::Options => self.open_settings = true,
            MenuChoice::Spectate => self.request_ship(SPECTATOR),
            MenuChoice::News => {
                self.news_open = true;
                self.news_scroll = 0;
            }
            MenuChoice::Ship(kind) => self.request_ship(kind),
            MenuChoice::FrameRate => self.fps_view = self.fps_view.next(),
            MenuChoice::Arenas => self.ask_for_arenas(),
        }
        true
    }

    /// The digit keys the menu's eight ships answer to, in order, so a click on
    /// a ship row is the same keystroke as typing its number.
    const SHIP_DIGITS: [KeyCode; 8] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
    ];

    /// The menu, in the box SubSpace and Continuum draw it in: keys down the left,
    /// ships down the right from column 26, and the line that says any other key
    /// puts it away. The wording and the columns are the original client's own
    /// (`SUBSPACE.EXE`, the block of strings beginning `-= Menu =-`); the frame,
    /// the fill and the rule under the title are Continuum's, out of `colors.bm2`.
    /// Only the entries this client actually has are listed.
    ///
    /// The rows are clickable, and a click on one **is** that key: the row
    /// records the keystroke it answers to and the click puts it on the input
    /// queue, so the menu's own logic never learns there was a pointer. That is
    /// only possible because a keystroke is one thing carrying its character --
    /// the menu reads characters, so a click that could only press a key code
    /// would do nothing here, exactly as `--press` could not.
    pub(super) fn draw_menu(&mut self, c: &Canvas, r: &mut Renderer) {
        let ships = [
            "Ships",
            "1 = Warbird",
            "2 = Javelin",
            "3 = Spider",
            "4 = Leviathan",
            "5 = Terrier",
            "6 = Weasel",
            "7 = Lancaster",
            "8 = Shark",
        ];
        let mut keys = vec![
            ("Q  = Quit", Stroke::press(KeyCode::KeyQ).typing('q')),
            ("F1 = Help", Stroke::press(KeyCode::F1)),
            ("F2 = Stat box", Stroke::press(KeyCode::F2)),
        ];
        if self.online() {
            keys.push((" A = Arena list", Stroke::press(KeyCode::KeyA).typing('a')));
        }
        if !self.news.is_empty() {
            keys.push((" N = Zone news", Stroke::press(KeyCode::KeyN).typing('n')));
        }
        keys.push((" O = Options", Stroke::press(KeyCode::KeyO).typing('o')));
        keys.push(("F12= Frame rate", Stroke::press(KeyCode::F12)));
        self.menu_spots.clear();
        // Spectator is a row under the eight ships, and it is there whether or
        // not there is a zone: "there's no spectator mode anywhere in the game
        // options menu" was the row being hidden offline, which is where the
        // lead was.
        let rows = keys.len().max(ships.len() + 1);

        // 40 characters across, as the original's longest line is, and up at the
        // top of the screen: the menu puts the whole message log on screen beneath
        // it, which is the point of it being out of the way (nullspace keeps the
        // same `(centred, y = 3)`).
        let w = 40.0 * 8.0;
        let h = 20.0 + (rows as f32 + 2.0) * 12.0;
        let x = ((c.view_w - w) / 2.0).floor();
        let y = 4.0;
        c.panel(r, x, y, w, h);
        c.text_center(r, x + w / 2.0, y + 1.0, color::GREEN, "-= Menu =-");
        c.separator(r, x, y + 13.0, w);

        let row_y = |i: usize| y + 18.0 + i as f32 * 12.0;
        let half = 26.0 * 8.0;
        for (i, (line, stroke)) in keys.iter().enumerate() {
            self.menu_spots
                .push((Rect::new(x, row_y(i) - 1.0, half, 12.0), *stroke));
            c.text(r, x + 2.0, row_y(i), color::WHITE, line);
        }
        // The ships sit at column 26 of the box, where the original puts them, with
        // their heading in dark red and spectator under the eighth.
        let ship_x = x + 2.0 + half;
        for (i, line) in ships.iter().enumerate() {
            let colour = if i == 0 {
                color::DARK_RED
            } else {
                color::WHITE
            };
            // The heading is a heading; the eight under it are the ships, by the
            // digit the menu already answers to.
            if let Some(digit) = char::from_digit(i as u32, 10).filter(|_| i > 0) {
                let key = Self::SHIP_DIGITS[i - 1];
                self.menu_spots.push((
                    Rect::new(ship_x - 2.0, row_y(i) - 1.0, w - half, 12.0),
                    Stroke::press(key).typing(digit),
                ));
            }
            c.text(r, ship_x, row_y(i), colour, line);
        }
        self.menu_spots.push((
            Rect::new(ship_x - 2.0, row_y(ships.len()) - 1.0, w - half, 12.0),
            Stroke::press(KeyCode::KeyS).typing('s'),
        ));
        c.text(r, ship_x, row_y(ships.len()), color::WHITE, "S = Spectator");
        c.text(
            r,
            x + 2.0,
            row_y(rows),
            color::WHITE,
            "PgUp/PgDn = Scroll messages",
        );
        c.text_center(
            r,
            x + w / 2.0,
            row_y(rows + 1),
            color::YELLOW,
            "Any other key to resume game",
        );
    }
}

/// Whether a player may leave the game yet.
///
/// Both originals make you wait until half your energy is back before the menu's Q
/// takes effect, so that a fight being lost cannot be quit out of: nullspace's
/// `Game::Update` tests `self->energy >= half_energy` with `half_energy` half the
/// ship's maximum, and shows "Exiting game..." until then. `in_a_fight` is false
/// for a spectator, a dead ship and an offline game, where there is nobody to dodge.
fn may_leave(in_a_fight: bool, energy: i32, max_energy: i32) -> bool {
    !in_a_fight || energy * 2 >= max_energy
}

/// What one of the menu's own keys means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuChoice {
    Quit,
    Options,
    Spectate,
    News,
    Ship(u8),
    /// Esc then F: Continuum's frame-rate key, which its menu does not list.
    FrameRate,
    /// Esc then A: the zone's arena list.
    Arenas,
}

/// What a character typed on the menu does, or `None` for "any other key to
/// resume game".
///
/// Pulled out of the menu itself so that which rows the menu offers can be
/// checked without a window and a renderer. The only thing that varies is the
/// news row, which is there when the zone sent any news: spectator used to vary
/// too -- it was answered only when there was a zone -- and that is what left
/// the offline sandbox's menu with no spectator on it at all.
fn menu_choice(ch: char, has_news: bool, online: bool) -> Option<MenuChoice> {
    match ch.to_ascii_lowercase() {
        'q' => Some(MenuChoice::Quit),
        'o' => Some(MenuChoice::Options),
        's' => Some(MenuChoice::Spectate),
        'n' if has_news => Some(MenuChoice::News),
        'f' => Some(MenuChoice::FrameRate),
        'a' if online => Some(MenuChoice::Arenas),
        d @ '1'..='8' => Some(MenuChoice::Ship(d as u8 - b'1')),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::help::PanelLines;
    use super::{MenuChoice, may_leave, menu_choice};

    #[test]
    fn quitting_waits_for_half_your_energy() {
        // In a fight: half the ship's maximum, which is nullspace's test.
        assert!(!may_leave(true, 499, 1000));
        assert!(may_leave(true, 500, 1000));
        assert!(may_leave(true, 1000, 1000));
        // Spectating, dead or offline: nothing to dodge, so go at once.
        assert!(may_leave(false, 0, 1000));
        // A ship with no maximum at all must not trap the player in the game.
        assert!(may_leave(true, 0, 0));
    }

    /// The menu is the other bordered box on this screen, and its two columns
    /// are the kind of thing that fits until someone writes a longer ship name.
    #[test]
    fn the_menu_columns_hold_what_is_put_in_them() {
        // The box, and the column the ships start at, as `draw_menu` sets them.
        let w = 40.0 * 8.0;
        let half = 26.0 * 8.0;
        let p = PanelLines::new(w);
        let ships = [
            "Ships",
            "1 = Warbird",
            "2 = Javelin",
            "3 = Spider",
            "4 = Leviathan",
            "5 = Terrier",
            "6 = Weasel",
            "7 = Lancaster",
            "8 = Shark",
            "S = Spectator",
        ];
        let room = p.cols(2.0 + half);
        for line in ships {
            assert!(
                line.chars().count() <= room,
                "{line:?} is {} characters in a column of {room}",
                line.chars().count()
            );
        }
        // And the widest line of the left-hand column.
        assert!("PgUp/PgDn = Scroll messages".chars().count() <= p.cols(2.0));
    }

    /// "There's no spectator mode anywhere in the game options menu." Offline,
    /// that was exactly true: the menu hid the row and the ship change refused
    /// spectator outright, so the sandbox -- where the lead was -- could not
    /// stop flying.
    #[test]
    fn the_menu_offers_spectator_with_or_without_a_zone() {
        // `false` here is "no news", which offline always is. S used to be
        // answered only when there was a zone, so this is the case that failed.
        assert_eq!(menu_choice('s', false, false), Some(MenuChoice::Spectate));
        assert_eq!(menu_choice('S', true, false), Some(MenuChoice::Spectate));
        // The news row is the one that genuinely varies, and still does.
        assert_eq!(menu_choice('n', false, false), None, "no news, no row");
        assert_eq!(menu_choice('n', true, false), Some(MenuChoice::News));
        // The rest of the menu, so that pulling this out changed nothing else.
        assert_eq!(menu_choice('q', false, false), Some(MenuChoice::Quit));
        assert_eq!(menu_choice('o', false, false), Some(MenuChoice::Options));
        assert_eq!(menu_choice('1', false, false), Some(MenuChoice::Ship(0)));
        assert_eq!(menu_choice('8', false, false), Some(MenuChoice::Ship(7)));
        assert_eq!(menu_choice('f', false, false), Some(MenuChoice::FrameRate));
        assert_eq!(
            menu_choice('9', false, false),
            None,
            "any other key resumes"
        );
        assert_eq!(menu_choice('x', true, false), None);
        // The arena list is the zone's, so the row is there only in a zone;
        // offline, A is any other key.
        assert_eq!(menu_choice('a', false, true), Some(MenuChoice::Arenas));
        assert_eq!(menu_choice('A', false, true), Some(MenuChoice::Arenas));
        assert_eq!(menu_choice('a', false, false), None);
    }
}
