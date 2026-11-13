//! The start-up screen's clickable regions, by the names Continuum's skins use.
//!
//! Continuum's start-up screen is **defined** by its hot spots: its skin format
//! names a rectangle for `profile`, `prevprof`, `nextprof`, `ship1`..`ship8`,
//! `spectate`, `zones`, `zoneup`, `zonedown`, `trace`, `play`, `quit`, `macros`,
//! `keydefs`, `options` and `advopt`, plus an `infobox` that explains whichever
//! region the pointer is over. It was never a keyboard-only screen, and the one
//! we built was -- which is the fidelity gap this closes, not a convenience.
//!
//! The spots are collected **by the drawing code as it draws**, so a region can
//! never be somewhere other than the thing a player can see. They are one frame
//! old by the time a click is tested against them, which at a hundred frames a
//! second is ten milliseconds and no pointer's problem.
//!
//! Spots this client has no equivalent for are simply absent: there is no
//! profile carousel (`prevprof`/`nextprof`), no macro or key editor
//! (`macros`/`keydefs`), no `trace` and no advanced options. `zoneup` and
//! `zonedown` are the scroll wheel over the list, which is where a pointer
//! expects them now.

use super::login::Field;

/// A rectangle in UI pixels -- game pixels from the top-left of the window,
/// which is what every position in the drawing code is in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn holds(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Something on the start-up screen a pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
    /// A line of the profile block, or the zone list's heading: clicking it
    /// gives it the focus, exactly as tabbing to it would.
    Field(Field),
    /// `ship1`..`ship8`.
    Ship(u8),
    /// `spectate`.
    Spectate,
    /// A row of the zone list. The index is into the browser's own list, not
    /// into what is on screen.
    Zone(usize),
    /// `play`.
    Play,
    /// `quit`.
    Quit,
    /// `options`.
    Options,
    /// Ask the directory servers again. Continuum refreshes its list from the
    /// same button that opens it, so this has no skin name of its own.
    RefreshZones,
    /// The offline sandbox, which is ours and has no skin name either.
    Sandbox,
}

impl Spot {
    /// One line saying what this does, for the footer while the pointer is over
    /// it -- the job Continuum's `infobox` does.
    pub fn hint(self) -> &'static str {
        match self {
            Spot::Field(Field::Name) => "Your name in the zone.",
            Spot::Field(Field::Password) => "Your password, if the zone knows you.",
            Spot::Field(Field::Arena) => "Arena to enter, or blank for any public one.",
            Spot::Field(Field::Server) => "The zone's address, as host:port.",
            Spot::Field(Field::Ship) => "The ship you will fly.",
            Spot::Field(Field::Zones) => "Zones the directory servers know about.",
            Spot::Ship(_) => "Fly this ship.",
            Spot::Spectate => "Enter watching rather than flying.",
            Spot::Zone(_) => "Click to pick this zone.",
            Spot::Play => "Enter the zone.",
            Spot::Quit => "Leave Open SubSpace.",
            Spot::Options => "Settings: keys, sound, display.",
            Spot::RefreshZones => "Ask the directory servers again.",
            Spot::Sandbox => "Fly offline, with no zone.",
        }
    }
}

/// Where everything on the start-up screen was drawn last frame.
#[derive(Default)]
pub struct Spots {
    spots: Vec<(Rect, Spot)>,
    /// The zone list as a whole, for the scroll wheel.
    list: Option<Rect>,
}

impl Spots {
    /// Forgets last frame's regions. Called as the screen starts drawing.
    pub fn clear(&mut self) {
        self.spots.clear();
        self.list = None;
    }

    /// Records a region as it is drawn.
    pub fn add(&mut self, rect: Rect, spot: Spot) {
        self.spots.push((rect, spot));
    }

    /// Records where the zone list as a whole is, for the wheel.
    pub fn set_list(&mut self, rect: Rect) {
        self.list = Some(rect);
    }

    /// What is at this point, if anything.
    ///
    /// Last added wins, so a row drawn over a panel is found before the panel.
    pub fn at(&self, x: f32, y: f32) -> Option<Spot> {
        self.spots
            .iter()
            .rev()
            .find(|(r, _)| r.holds(x, y))
            .map(|(_, s)| *s)
    }

    /// Whether the pointer is over the zone list, which is what the wheel
    /// scrolls.
    pub fn over_list(&self, x: f32, y: f32) -> bool {
        self.list.is_some_and(|r| r.holds(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_thing_drawn_is_the_thing_clicked() {
        // The panel is drawn first and the rows over it, so a click on a row is
        // the row's. Without this a click would land on whatever was drawn
        // first, which is always the background.
        let mut s = Spots::default();
        s.add(Rect::new(0.0, 0.0, 100.0, 100.0), Spot::Field(Field::Zones));
        s.add(Rect::new(10.0, 10.0, 50.0, 12.0), Spot::Zone(3));
        assert_eq!(s.at(20.0, 15.0), Some(Spot::Zone(3)));
        assert_eq!(s.at(20.0, 50.0), Some(Spot::Field(Field::Zones)));
        assert_eq!(s.at(200.0, 50.0), None, "outside everything");
    }

    #[test]
    fn a_rectangle_takes_its_top_left_and_not_its_bottom_right() {
        // Half-open, so two rows drawn 12 apart cannot both claim a point.
        let r = Rect::new(10.0, 20.0, 30.0, 12.0);
        assert!(r.holds(10.0, 20.0), "top-left corner is inside");
        assert!(!r.holds(40.0, 25.0), "right edge is outside");
        assert!(!r.holds(20.0, 32.0), "bottom edge is outside");
        assert!(r.holds(39.9, 31.9));
    }

    #[test]
    fn clearing_forgets_the_frame_before() {
        let mut s = Spots::default();
        s.add(Rect::new(0.0, 0.0, 10.0, 10.0), Spot::Play);
        s.set_list(Rect::new(0.0, 0.0, 10.0, 10.0));
        s.clear();
        assert_eq!(s.at(5.0, 5.0), None);
        assert!(!s.over_list(5.0, 5.0));
    }
}
