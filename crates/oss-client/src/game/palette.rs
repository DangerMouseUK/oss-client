//! Colour palettes for colour-blind players, and a high-contrast one.
//!
//! The standard palette is Continuum's: the font's own coloured rows, yellow for your
//! team and blue for enemies. The others draw text from the font's white row tinted
//! with colours from the Okabe–Ito set (chosen to stay distinct under each kind of
//! colour blindness), and recolour the radar's team, enemy, flagger and ball marks.
//! They haven't been checked with colour-blind players yet.

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
        let (sky, blue, vermillion, yellow, purple, orange, green) = (
            c(86, 180, 233),
            c(0, 114, 178),
            c(213, 94, 0),
            c(240, 228, 66),
            c(204, 121, 167),
            c(230, 159, 0),
            c(0, 158, 115),
        );
        match self {
            PaletteKind::Standard => None,
            // Red/green confusions: greens become sky blue, reds orange/vermillion.
            PaletteKind::Deuteranopia | PaletteKind::Protanopia => {
                Some([white, sky, blue, vermillion, yellow, purple, orange, purple])
            }
            // Blue/yellow confusions: team yellow becomes vermillion, blue becomes green.
            PaletteKind::Tritanopia => Some([
                white, sky, green, vermillion, vermillion, purple, orange, purple,
            ]),
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
            PaletteKind::Deuteranopia | PaletteKind::Protanopia => Marks {
                team: c(240, 228, 66),
                enemy: c(0, 114, 178),
                flagger: c(230, 159, 0),
                ball: c(255, 255, 255),
                weapon: c(190, 190, 190),
            },
            PaletteKind::Tritanopia => Marks {
                team: c(213, 94, 0),
                enemy: c(0, 158, 115),
                flagger: c(204, 121, 167),
                ball: c(255, 255, 255),
                weapon: c(190, 190, 190),
            },
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
