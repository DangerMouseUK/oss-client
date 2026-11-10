//! Colour palettes for colour-blind players, and a high-contrast one.
//!
//! The standard palette is Continuum's: the font's own coloured rows, yellow for your
//! team and blue for enemies. The others draw text from the font's white row tinted
//! with colours from the Okabe–Ito set (chosen to stay distinct under each kind of
//! colour blindness), and recolour the radar's team, enemy, flagger and ball marks.
//!
//! Every colour here is checked by simulation, not by eye: the tests at the bottom
//! put each palette through the Viénot dichromat model and require the colours a
//! player must tell apart to be at least 20 ΔE apart for the eye the palette is
//! for — and for everyone else. That is what found the two faults this file used
//! to have (a tritan player saw their own team's dot and an enemy flag carrier's
//! in the same colour, and warnings in the same colour as team chat). What it
//! cannot check is whether the result is comfortable or whether the hues still
//! read as what they mean, so a colour-blind player should still look.

/// Which palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaletteKind {
    #[default]
    Standard,
    /// Red-green (green-weak).
    Deuteranopia,
    /// Red-green (red-weak).
    Protanopia,
    /// Blue-yellow.
    Tritanopia,
    HighContrast,
}

impl PaletteKind {
    pub const ALL: [PaletteKind; 5] = [
        PaletteKind::Standard,
        PaletteKind::Deuteranopia,
        PaletteKind::Protanopia,
        PaletteKind::Tritanopia,
        PaletteKind::HighContrast,
    ];

    /// Name in the settings file.
    pub fn name(self) -> &'static str {
        match self {
            PaletteKind::Standard => "standard",
            PaletteKind::Deuteranopia => "deuteranopia",
            PaletteKind::Protanopia => "protanopia",
            PaletteKind::Tritanopia => "tritanopia",
            PaletteKind::HighContrast => "high_contrast",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(s))
    }

    /// Text tints by colour index (white, green, blue, dark red, yellow, fuchsia,
    /// red, pink); `None` keeps the font's own coloured rows.
    pub fn text_tints(self) -> Option<[[f32; 4]; 8]> {
        let c = |r: u8, g: u8, b: u8| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
        let white = c(255, 255, 255);
        let (sky, blue, vermillion, yellow, purple, green) = (
            c(86, 180, 233),
            c(0, 114, 178),
            c(213, 94, 0),
            c(240, 228, 66),
            c(204, 121, 167),
            c(0, 158, 115),
        );
        let (magenta, grey, cyan, brown) = (
            c(255, 0, 255),
            c(190, 190, 190),
            c(0, 200, 255),
            c(140, 70, 20),
        );
        match self {
            PaletteKind::Standard => None,
            // Red/green confusions: greens become sky blue, reds vermillion, and
            // channel red purple (orange sat too close to vermillion and yellow).
            PaletteKind::Deuteranopia | PaletteKind::Protanopia => {
                Some([white, sky, blue, vermillion, yellow, magenta, purple, grey])
            }
            // Blue/yellow confusions: blues become cyan and green, and the two reds
            // are told apart by lightness (brown against vermillion), because every
            // warm hue collapses into one for a tritan eye.
            PaletteKind::Tritanopia => {
                Some([white, green, cyan, brown, yellow, magenta, vermillion, grey])
            }
            PaletteKind::HighContrast => Some([
                white,
                c(0, 255, 0),
                c(0, 200, 255),
                c(255, 80, 80),
                c(255, 255, 0),
                c(255, 0, 255),
                c(255, 40, 40),
                c(255, 150, 220),
            ]),
        }
    }

    /// Radar and marker colours.
    pub fn marks(self) -> Marks {
        let c = |r: u8, g: u8, b: u8| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
        match self {
            PaletteKind::Standard => Marks {
                team: [1.0, 1.0, 0.0, 1.0],
                enemy: [0.3, 0.5, 1.0, 1.0],
                flagger: [1.0, 0.25, 0.25, 1.0],
                ball: [1.0, 0.6, 0.1, 1.0],
                weapon: [0.85, 0.85, 0.85, 1.0],
            },
            // One set serves all three: it keeps Continuum's yellow team and blue
            // enemies, and stays apart for every kind of colour blindness. The
            // flagger was orange here and purple for tritanopia; purple and the
            // team's vermillion were the same colour to a tritan eye.
            PaletteKind::Deuteranopia | PaletteKind::Protanopia | PaletteKind::Tritanopia => {
                Marks {
                    team: c(240, 228, 66),
                    enemy: c(0, 114, 178),
                    flagger: c(213, 94, 0),
                    ball: c(255, 255, 255),
                    weapon: c(190, 190, 190),
                }
            }
            PaletteKind::HighContrast => Marks {
                team: c(255, 255, 0),
                enemy: c(0, 200, 255),
                flagger: c(255, 0, 255),
                ball: c(255, 255, 255),
                weapon: c(190, 190, 190),
            },
        }
    }
}

/// Radar dot colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marks {
    pub team: [f32; 4],
    pub enemy: [f32; 4],
    pub flagger: [f32; 4],
    pub ball: [f32; 4],
    /// Bombs and mines on the radar.
    pub weapon: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_team_enemy_stay_apart() {
        for p in PaletteKind::ALL {
            assert_eq!(PaletteKind::from_name(p.name()), Some(p));
            let m = p.marks();
            assert_ne!(m.team, m.enemy);
            if let Some(t) = p.text_tints() {
                // Team (yellow) and enemy (blue) text differ.
                assert_ne!(t[4], t[2]);
            }
        }
        assert_eq!(
            PaletteKind::from_name("HIGH_CONTRAST"),
            Some(PaletteKind::HighContrast)
        );
        assert_eq!(PaletteKind::from_name("rainbow"), None);
    }
}

/// Dichromat simulation, so the palettes can be checked without a colour-blind
/// player in the room: Viénot, Brettel and Mollon (1999), which projects linear
/// RGB through the Hunt-Pointer-Estevez LMS space onto the plane a dichromat can
/// see. Distances are CIE76 ΔE in L\*a\*b\*; 20 is comfortably "a different
/// colour" for marks a few pixels across and for 8×12 text.
///
/// This checks that colours stay apart, which is what the palettes are for. It
/// does **not** check that they are pleasant, that the hue still means what a
/// player expects, or how they read on a real screen: that still wants a
/// colour-blind player, and until one has looked, the palettes stay unverified in
/// `notes/parity.md` for comfort even though separation is now proven.
#[cfg(test)]
mod cvd {
    use super::*;

    /// Which cone is missing.
    #[derive(Clone, Copy)]
    pub enum Vision {
        Normal,
        /// No long-wavelength cone (red-weak).
        Protanopia,
        /// No medium-wavelength cone (green-weak).
        Deuteranopia,
        /// No short-wavelength cone (blue-weak).
        Tritanopia,
    }

    fn to_linear(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// The colour as that eye sees it, in linear RGB.
    fn simulate(rgba: [f32; 4], v: Vision) -> [f32; 3] {
        let [r, g, b] = [to_linear(rgba[0]), to_linear(rgba[1]), to_linear(rgba[2])];
        let (mut l, mut m, mut s) = (
            17.8824 * r + 43.5161 * g + 4.11935 * b,
            3.45565 * r + 27.1554 * g + 3.86714 * b,
            0.0299566 * r + 0.184309 * g + 1.46709 * b,
        );
        match v {
            Vision::Normal => {}
            Vision::Protanopia => l = 2.02344 * m - 2.52581 * s,
            Vision::Deuteranopia => m = 0.494207 * l + 1.24827 * s,
            Vision::Tritanopia => s = -0.395913 * l + 0.801109 * m,
        }
        [
            0.080944 * l - 0.130504 * m + 0.116721 * s,
            -0.010_248_5 * l + 0.054_019_4 * m - 0.113615 * s,
            -0.000_365_294 * l - 0.004_121_63 * m + 0.693513 * s,
        ]
    }

    /// Linear RGB to L\*a\*b\* (D65).
    fn lab(rgb: [f32; 3]) -> [f32; 3] {
        let [r, g, b] = rgb.map(|c| c.clamp(0.0, 1.0));
        let (x, y, z) = (
            0.4124 * r + 0.3576 * g + 0.1805 * b,
            0.2126 * r + 0.7152 * g + 0.0722 * b,
            0.0193 * r + 0.1192 * g + 0.9505 * b,
        );
        let f = |t: f32| {
            if t > 0.008856 {
                t.cbrt()
            } else {
                7.787 * t + 16.0 / 116.0
            }
        };
        let (fx, fy, fz) = (f(x / 0.95047), f(y), f(z / 1.08883));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    fn distance(a: [f32; 4], b: [f32; 4], v: Vision) -> f32 {
        let (la, lb) = (lab(simulate(a, v)), lab(simulate(b, v)));
        ((la[0] - lb[0]).powi(2) + (la[1] - lb[1]).powi(2) + (la[2] - lb[2]).powi(2)).sqrt()
    }

    /// How far apart the closest pair of `colors` is for that eye.
    fn closest_pair(colors: &[([f32; 4], &str)], v: Vision) -> (f32, String) {
        let mut worst = (f32::MAX, String::new());
        for (i, (a, na)) in colors.iter().enumerate() {
            for (b, nb) in &colors[i + 1..] {
                let d = distance(*a, *b, v);
                if d < worst.0 {
                    worst = (d, format!("{na}/{nb}"));
                }
            }
        }
        worst
    }

    fn mark_colors(p: PaletteKind) -> Vec<([f32; 4], &'static str)> {
        let m = p.marks();
        vec![
            (m.team, "team"),
            (m.enemy, "enemy"),
            (m.flagger, "flagger"),
            (m.ball, "ball"),
            (m.weapon, "weapon"),
        ]
    }

    /// The text colours a player has to tell apart: public, team, private/arena,
    /// warnings and channel lines. Fuchsia and pink are left out; zones use them
    /// rarely and never beside each other.
    fn text_colors(p: PaletteKind) -> Vec<([f32; 4], &'static str)> {
        let t = p.text_tints().expect("a tinted palette");
        vec![
            (t[0], "white"),
            (t[1], "green"),
            (t[2], "blue"),
            (t[3], "dark red"),
            (t[4], "yellow"),
            (t[6], "red"),
        ]
    }

    /// ΔE at which two colours are clearly different rather than nearly alike.
    const APART: f32 = 20.0;

    #[test]
    fn colour_blind_palettes_stay_apart_for_the_eye_they_are_for() {
        for (palette, vision) in [
            (PaletteKind::Deuteranopia, Vision::Deuteranopia),
            (PaletteKind::Protanopia, Vision::Protanopia),
            (PaletteKind::Tritanopia, Vision::Tritanopia),
        ] {
            for (what, colors) in [
                ("radar marks", mark_colors(palette)),
                ("chat colours", text_colors(palette)),
            ] {
                let (d, pair) = closest_pair(&colors, vision);
                assert!(
                    d >= APART,
                    "{} {what}: {pair} are only {d:.1} apart",
                    palette.name()
                );
                // And they must stay apart for everyone else too.
                let (d, pair) = closest_pair(&colors, Vision::Normal);
                assert!(
                    d >= APART,
                    "{} {what}: {pair} are only {d:.1} apart in normal vision",
                    palette.name()
                );
            }
        }
    }

    #[test]
    fn the_standard_palette_is_why_the_others_exist() {
        // Continuum's own radar colours collapse for a tritan player: the red of a
        // flag carrier and the orange of a ball are nearly the same colour. If this
        // ever stops being true the check above has lost its teeth.
        let (d, _) = closest_pair(&mark_colors(PaletteKind::Standard), Vision::Tritanopia);
        assert!(
            d < APART,
            "standard marks are {d:.1} apart under tritanopia"
        );
    }

    #[test]
    fn high_contrast_is_bright_against_the_game() {
        // This one is for low vision rather than for a missing cone, so what it has
        // to do is stay legible on a dark background: WCAG AA (4.5:1) against black.
        let tints = PaletteKind::HighContrast.text_tints().unwrap();
        for (i, t) in tints.iter().enumerate() {
            let lum =
                0.2126 * to_linear(t[0]) + 0.7152 * to_linear(t[1]) + 0.0722 * to_linear(t[2]);
            let ratio = (lum + 0.05) / 0.05;
            assert!(
                ratio >= 4.5,
                "high contrast colour {i} is only {ratio:.1}:1"
            );
        }
    }
}
