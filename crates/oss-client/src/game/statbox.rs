//! The stat box: the player list in the top-left corner, in Continuum's views.
//!
//! F2 cycles Names → Points → Point Sort → Team Sort → Full → Frequency → hidden;
//! Page Up/Down move the selection (the selected player is the target for private
//! messages and, later, spectating). Layout follows Continuum's: an 8×12 font, a
//! header line with the player count, then one 12-pixel row per player. This module
//! only lays the box out; drawing is in [`super::sandbox`].

use super::view::color;

/// Padding around the box.
const BORDER: f32 = 3.0;
const HEADER_HEIGHT: f32 = 14.0;
/// Room for the selection/spectator marker before each name.
const MARKER_WIDTH: f32 = 8.0;
const ROW: f32 = 12.0;
/// Rows shown at once.
const ROWS: usize = 20;
/// Names are cut to this many characters.
const NAME_CHARS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Names,
    Points,
    PointSort,
    TeamSort,
    Full,
    Frequency,
    Hidden,
}

impl View {
    fn next(self) -> View {
        match self {
            View::Names => View::Points,
            View::Points => View::PointSort,
            View::PointSort => View::TeamSort,
            View::TeamSort => View::Full,
            View::Full => View::Frequency,
            View::Frequency => View::Hidden,
            View::Hidden => View::Names,
        }
    }
}

/// One player as the stat box sees them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    pub pid: i16,
    pub name: String,
    pub squad: String,
    pub spectator: bool,
    pub freq: i16,
    pub kill_points: i32,
    pub flag_points: i32,
    pub wins: i16,
    pub losses: i16,
    pub flags: i16,
}

impl Entry {
    fn points(&self) -> i64 {
        self.kill_points as i64 + self.flag_points as i64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub x: f32,
    pub y: f32,
    pub align: Align,
    pub color: u32,
    pub text: String,
}

/// The small square before a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Selected,
    Spectator,
    SelectedSpectator,
}

/// A laid-out box in game pixels from the top-left of the screen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// Background rectangle (x, y, w, h).
    pub rect: (f32, f32, f32, f32),
    /// The line under the header (y).
    pub separator: f32,
    pub texts: Vec<Text>,
    pub markers: Vec<(f32, f32, Marker)>,
}

impl Layout {
    fn text(&mut self, x: f32, y: f32, align: Align, color: u32, text: impl Into<String>) {
        self.texts.push(Text {
            x,
            y,
            align,
            color,
            text: text.into(),
        });
    }

    /// The name column, with its marker.
    fn name(&mut self, e: &Entry, y: f32, selected: bool, same_freq: bool) {
        let marker = match (selected, e.spectator) {
            (true, false) => Some(Marker::Selected),
            (true, true) => Some(Marker::SelectedSpectator),
            (false, true) => Some(Marker::Spectator),
            (false, false) => None,
        };
        if let Some(m) = marker {
            self.markers.push((BORDER, y + 3.0, m));
        }
        let name: String = e.name.chars().take(NAME_CHARS).collect();
        self.text(
            BORDER + MARKER_WIDTH + 2.0,
            y,
            Align::Left,
            row_color(same_freq),
            name,
        );
    }
}

fn row_color(same_freq: bool) -> u32 {
    if same_freq {
        color::YELLOW
    } else {
        color::WHITE
    }
}

fn digits(n: i64) -> f32 {
    n.to_string().len() as f32
}

#[derive(Debug, Clone, Default)]
pub struct StatBox {
    pub view: View,
    /// Index into the sorted list.
    selected: usize,
    /// First row shown.
    top: usize,
    /// Player at the selection, from the last layout.
    selected_pid: Option<i16>,
}

impl StatBox {
    /// F2: next view.
    pub fn cycle(&mut self) {
        self.view = self.view.next();
    }

    /// Page Up/Down: moves the selection; shows the box if hidden.
    pub fn move_selection(&mut self, down: bool) {
        if self.view == View::Hidden {
            self.view = View::Names;
        } else if down {
            self.selected += 1;
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    /// The selected player, as of the last layout.
    pub fn selected(&self) -> Option<i16> {
        self.selected_pid
    }

    /// Sorts `players` for the current view and lays the box out. `me` is your
    /// player id. Returns `None` when hidden.
    pub fn layout(&mut self, me: i16, players: &[Entry]) -> Option<Layout> {
        let me_entry = players.iter().find(|p| p.pid == me)?;
        if self.view == View::Hidden {
            return None;
        }
        let my_freq = me_entry.freq;
        let order = sorted(self.view, me, my_freq, players);
        self.selected = self.selected.min(order.len().saturating_sub(1));
        self.selected_pid = order.get(self.selected).map(|e| e.pid);
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + ROWS {
            self.top = self.selected + 1 - ROWS;
        }
        self.top = self.top.min(order.len().saturating_sub(ROWS));
        let mut l = Layout::default();
        let count = players.len().to_string();
        let width = match self.view {
            View::Names => {
                l.text(49.0, BORDER + 1.0, Align::Center, color::GREEN, count);
                self.rows(&mut l, &order, my_freq, |_, _, _| {});
                108.0
            }
            View::Points | View::PointSort => {
                let width = 124.0 + points_width(players);
                l.text(49.0, BORDER + 1.0, Align::Center, color::GREEN, count);
                let header = if self.view == View::PointSort {
                    "Point Sort"
                } else {
                    "Points"
                };
                l.text(width, BORDER + 1.0, Align::Right, color::GREEN, header);
                self.rows(&mut l, &order, my_freq, |l, e, y| {
                    let c = row_color(e.freq == my_freq);
                    l.text(width, y, Align::Right, c, e.points().to_string());
                });
                width
            }
            View::TeamSort => {
                let width = 124.0 + points_width(players);
                l.text(49.0, BORDER + 1.0, Align::Center, color::GREEN, count);
                l.text(width, BORDER + 1.0, Align::Right, color::GREEN, "Team Sort");
                self.team_rows(&mut l, &order, my_freq, width);
                width
            }
            View::Full => {
                self.full(&mut l, &order, my_freq, &count);
                452.0
            }
            View::Frequency => frequency(&mut l, &order, my_freq, self.selected_pid),
            View::Hidden => unreachable!(),
        };
        let bottom = l
            .texts
            .iter()
            .map(|t| t.y + ROW)
            .fold(BORDER + HEADER_HEIGHT, f32::max);
        l.rect = (BORDER, BORDER, width, bottom - BORDER + 1.0);
        l.separator = BORDER + 13.0;
        Some(l)
    }

    /// One row per visible player: name, then `extra` columns.
    fn rows(
        &self,
        l: &mut Layout,
        order: &[&Entry],
        my_freq: i16,
        mut extra: impl FnMut(&mut Layout, &Entry, f32),
    ) {
        for (row, (i, e)) in order
            .iter()
            .enumerate()
            .skip(self.top)
            .take(ROWS)
            .enumerate()
        {
            let y = BORDER + HEADER_HEIGHT + 1.0 + row as f32 * ROW;
            l.name(e, y, i == self.selected, e.freq == my_freq);
            extra(l, e, y);
        }
    }

    /// Team Sort: a "0001-------------  3" line before each team.
    fn team_rows(&self, l: &mut Layout, order: &[&Entry], my_freq: i16, width: f32) {
        let mut y = BORDER + HEADER_HEIGHT + 1.0;
        let mut lines = 0;
        let mut previous: Option<i16> = order.get(self.top.wrapping_sub(1)).map(|e| e.freq);
        for (i, e) in order.iter().enumerate().skip(self.top) {
            if lines >= ROWS {
                break;
            }
            if previous != Some(e.freq) {
                let members = order.iter().filter(|p| p.freq == e.freq).count();
                l.text(
                    BORDER + 1.0,
                    y,
                    Align::Left,
                    color::DARK_RED,
                    format!("{:04}-------------", e.freq),
                );
                l.text(
                    BORDER + 1.0 + 18.0 * 8.0,
                    y,
                    Align::Left,
                    color::DARK_RED,
                    members.to_string(),
                );
                previous = Some(e.freq);
                y += ROW;
                lines += 1;
                if lines >= ROWS {
                    break;
                }
            }
            l.name(e, y, i == self.selected, e.freq == my_freq);
            let c = row_color(e.freq == my_freq);
            l.text(width, y, Align::Right, c, e.points().to_string());
            y += ROW;
            lines += 1;
        }
    }

    /// Full: squad, wins, losses, rating and average points per kill.
    fn full(&self, l: &mut Layout, order: &[&Entry], my_freq: i16, count: &str) {
        const SQUAD_X: f32 = 133.0;
        const W_X: f32 = SQUAD_X + 17.0 * 8.0;
        const L_X: f32 = W_X + 7.0 * 8.0;
        const R_X: f32 = L_X + 7.0 * 8.0;
        const WIDTH: f32 = 452.0;
        let y0 = BORDER + 1.0;
        l.text(49.0, y0, Align::Center, color::GREEN, count);
        l.text(SQUAD_X, y0, Align::Left, color::GREEN, "Squad");
        l.text(W_X, y0, Align::Left, color::GREEN, "W");
        l.text(L_X, y0, Align::Left, color::GREEN, "L");
        l.text(R_X, y0, Align::Left, color::GREEN, "R");
        l.text(WIDTH, y0, Align::Right, color::GREEN, "Ave");
        self.rows(l, order, my_freq, |l, e, y| {
            let c = row_color(e.freq == my_freq);
            let squad: String = e.squad.chars().take(10).collect();
            l.text(SQUAD_X, y, Align::Left, c, squad);
            l.text(W_X + 8.0, y, Align::Right, c, e.wins.to_string());
            l.text(L_X + 8.0, y, Align::Right, c, e.losses.to_string());
            l.text(R_X + 8.0, y, Align::Right, c, rating(e).to_string());
            l.text(WIDTH, y, Align::Right, c, format!("{:.1}", average(e)));
        });
    }
}

/// Continuum's rating: kill points and net kills, damped for players with few wins.
pub fn rating(e: &Entry) -> i64 {
    let (w, lo) = (e.wins as i64, e.losses as i64);
    ((e.kill_points as i64 + (w - lo) * 10) * 10 / (w + 100).max(1)).max(0)
}

/// Kill points per win.
pub fn average(e: &Entry) -> f64 {
    if e.wins > 0 {
        e.kill_points as f64 / e.wins as f64
    } else {
        0.0
    }
}

fn points_width(players: &[Entry]) -> f32 {
    let max = players.iter().map(Entry::points).max().unwrap_or(0);
    // At least room for the "Point Sort" header beyond the names.
    (digits(max) * 8.0).max(8.0)
}

/// Frequency: one row per team with summed points, wins, losses and flags.
fn frequency(l: &mut Layout, order: &[&Entry], my_freq: i16, selected: Option<i16>) -> f32 {
    let mut teams: Vec<(i16, i64, i64, i64, i64)> = Vec::new();
    for e in order {
        match teams.iter_mut().find(|t| t.0 == e.freq) {
            Some(t) => {
                t.1 += e.points();
                t.2 += e.wins as i64;
                t.3 += e.losses as i64;
                t.4 += e.flags as i64;
            }
            None => teams.push((
                e.freq,
                e.points(),
                e.wins as i64,
                e.losses as i64,
                e.flags as i64,
            )),
        }
    }
    let pw = teams.iter().map(|t| digits(t.1)).fold(1.0, f32::max) * 8.0;
    let width = 252.0 + pw;
    let freq_x = 41.0 + BORDER;
    let points_x = freq_x + pw + 33.0;
    let win_x = points_x + 64.0;
    let lose_x = win_x + 64.0;
    let y0 = BORDER + 1.0;
    l.text(freq_x, y0, Align::Right, color::GREEN, "Freq");
    l.text(points_x, y0, Align::Right, color::GREEN, "Points");
    l.text(win_x, y0, Align::Right, color::GREEN, "Win");
    l.text(lose_x, y0, Align::Right, color::GREEN, "Lose");
    l.text(width, y0, Align::Right, color::GREEN, "Flag");
    let selected_freq =
        selected.and_then(|pid| order.iter().find(|e| e.pid == pid).map(|e| e.freq));
    for (row, &(freq, points, wins, losses, flags)) in teams.iter().take(ROWS).enumerate() {
        let y = BORDER + HEADER_HEIGHT + 1.0 + row as f32 * ROW;
        let c = row_color(freq == my_freq);
        l.text(freq_x, y, Align::Right, c, format!("{freq:4}"));
        l.text(points_x, y, Align::Right, c, points.to_string());
        l.text(win_x, y, Align::Right, c, wins.to_string());
        l.text(lose_x, y, Align::Right, c, losses.to_string());
        l.text(width, y, Align::Right, c, flags.to_string());
        if selected_freq == Some(freq) {
            l.markers.push((BORDER, y + 3.0, Marker::Selected));
        }
    }
    width
}

/// Continuum's orders: you first, then your team by name, then everyone else by name
/// (Names, Points, Full) or by team then name (Team Sort, Frequency); Point Sort is by
/// points, highest first.
fn sorted<'a>(view: View, me: i16, my_freq: i16, players: &'a [Entry]) -> Vec<&'a Entry> {
    let by_name = |a: &&Entry, b: &&Entry| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    };
    if view == View::PointSort {
        let mut all: Vec<&Entry> = players.iter().collect();
        all.sort_by(|a, b| b.points().cmp(&a.points()).then_with(|| by_name(a, b)));
        return all;
    }
    let mut out: Vec<&Entry> = players.iter().filter(|p| p.pid == me).collect();
    let mut team: Vec<&Entry> = players
        .iter()
        .filter(|p| p.pid != me && p.freq == my_freq)
        .collect();
    team.sort_by(by_name);
    let mut others: Vec<&Entry> = players.iter().filter(|p| p.freq != my_freq).collect();
    if matches!(view, View::TeamSort | View::Frequency) {
        others.sort_by(|a, b| a.freq.cmp(&b.freq).then_with(|| by_name(a, b)));
    } else {
        others.sort_by(by_name);
    }
    out.extend(team);
    out.extend(others);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: i16, name: &str, freq: i16, kill_points: i32, wins: i16, losses: i16) -> Entry {
        Entry {
            pid,
            name: name.into(),
            freq,
            kill_points,
            wins,
            losses,
            ..Default::default()
        }
    }

    fn players() -> Vec<Entry> {
        vec![
            p(1, "zed", 1, 50, 5, 1),
            p(2, "Me", 0, 10, 1, 2),
            p(3, "bob", 0, 300, 9, 0),
            p(4, "amy", 2, 0, 0, 0),
            p(5, "Cat", 1, 20, 2, 2),
        ]
    }

    fn names(l: &Layout) -> Vec<String> {
        l.texts
            .iter()
            .filter(|t| t.x == BORDER + MARKER_WIDTH + 2.0)
            .map(|t| t.text.clone())
            .collect()
    }

    #[test]
    fn views_sort_like_continuum() {
        let ps = players();
        let mut sb = StatBox::default();
        let l = sb.layout(2, &ps).unwrap();
        assert_eq!(names(&l), ["Me", "bob", "amy", "Cat", "zed"]);
        assert_eq!(l.texts[0].text, "5"); // player count
        sb.cycle();
        sb.cycle();
        assert_eq!(sb.view, View::PointSort);
        let l = sb.layout(2, &ps).unwrap();
        assert_eq!(names(&l), ["bob", "zed", "Cat", "Me", "amy"]);
        sb.cycle();
        let l = sb.layout(2, &ps).unwrap();
        assert_eq!(names(&l), ["Me", "bob", "Cat", "zed", "amy"]);
        let headers: Vec<_> = l
            .texts
            .iter()
            .filter(|t| t.color == color::DARK_RED && t.text.contains('-'))
            .map(|t| t.text.as_str())
            .collect();
        assert_eq!(
            headers,
            [
                "0000-------------",
                "0001-------------",
                "0002-------------"
            ]
        );
        // Your own team is yellow.
        let me = l.texts.iter().find(|t| t.text == "Me").unwrap();
        assert_eq!(me.color, color::YELLOW);
        for _ in 0..3 {
            sb.cycle();
        }
        assert_eq!(sb.view, View::Hidden);
        assert!(sb.layout(2, &ps).is_none());
    }

    #[test]
    fn selection_scrolls_and_reports_the_player() {
        let ps: Vec<Entry> = (0..30)
            .map(|i| p(i, &format!("p{i:02}"), 0, 0, 0, 0))
            .collect();
        let mut sb = StatBox::default();
        for _ in 0..25 {
            sb.move_selection(true);
        }
        let l = sb.layout(0, &ps).unwrap();
        assert_eq!(sb.selected(), Some(25));
        let shown = names(&l);
        assert_eq!(shown.len(), ROWS);
        assert_eq!(shown.last().map(String::as_str), Some("p25"));
        assert_eq!(l.markers.len(), 1);
        for _ in 0..100 {
            sb.move_selection(true);
        }
        sb.layout(0, &ps);
        assert_eq!(sb.selected(), Some(29));
    }

    #[test]
    fn full_and_frequency_numbers() {
        let e = p(1, "x", 0, 300, 9, 0);
        assert_eq!(rating(&e), (300 + 90) * 10 / 109);
        assert!((average(&e) - 33.3).abs() < 0.05);
        assert_eq!(rating(&p(1, "y", 0, 0, 0, 50)), 0);

        let mut sb = StatBox {
            view: View::Frequency,
            ..Default::default()
        };
        let l = sb.layout(2, &players()).unwrap();
        let row = |freq: &str| -> Vec<String> {
            let y = l.texts.iter().find(|t| t.text == freq).unwrap().y;
            l.texts
                .iter()
                .filter(|t| t.y == y)
                .map(|t| t.text.clone())
                .collect()
        };
        // Team 0: Me + bob. Team 1: zed + Cat.
        assert_eq!(row("   0"), ["   0", "310", "10", "2", "0"]);
        assert_eq!(row("   1"), ["   1", "70", "7", "3", "0"]);
    }
}
