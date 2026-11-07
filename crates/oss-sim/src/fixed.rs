//! Integer units and math. Everything that must be reproducible across machines is
//! integer: positions, velocities, rotation, energy.
//!
//! Units are chosen so the wire/settings units map exactly:
//! - Settings speeds are "pixels per 10 seconds". One tick is 1/100 s, so a velocity of
//!   `v` px/10s moves `v / 1000` px per tick. We store velocity as `v * 1000`
//!   ([`VEL_SCALE`]) and position in micro-pixels ([`POS_SCALE`]); then each tick is
//!   simply `pos += vel`.
//! - Thrust `t` adds `t` px/10s per tick (Continuum: `t * 10/16` tiles/s²).
//! - Rotation speed `r` is `r/400` turns per second, so with orientation measured in
//!   1/40000 turn ([`ROTATION_UNITS`]) each tick adds exactly `r`.
//! - Energy/recharge: recharge `r` is energy per 10 seconds, i.e. `r/1000` energy per
//!   tick; energy is stored in milli-units ([`ENERGY_SCALE`]) so a tick adds `r`.

/// Simulation rate.
pub const TICKS_PER_SECOND: u32 = 100;
/// Micro-pixels per pixel.
pub const POS_SCALE: i64 = 1_000_000;
/// Velocity sub-units per (pixel per 10 seconds).
pub const VEL_SCALE: i32 = 1000;
/// Orientation units per full turn.
pub const ROTATION_UNITS: u32 = 40_000;
/// Energy sub-units per energy point.
pub const ENERGY_SCALE: i32 = 1000;
/// Pixels per map tile.
pub const TILE_PIXELS: i32 = 16;

/// Fixed-point fraction bits used for unit vectors.
pub const UNIT_BITS: u32 = 16;
/// 1.0 in [`UNIT_BITS`] fixed point.
pub const UNIT_ONE: i32 = 1 << UNIT_BITS;

/// Integer 2D vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Vec2<T> {
    pub x: T,
    pub y: T,
}

impl<T> Vec2<T> {
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

/// Position in micro-pixels.
pub type Pos = Vec2<i64>;
/// Velocity in 1/1000 pixel per 10 seconds (= micro-pixels per tick).
pub type Vel = Vec2<i32>;

impl Pos {
    /// Builds a position from whole pixels.
    pub const fn from_pixels(x: i32, y: i32) -> Self {
        Self::new(x as i64 * POS_SCALE, y as i64 * POS_SCALE)
    }
    /// Whole-pixel position (floor).
    pub const fn pixels(self) -> Vec2<i32> {
        Vec2::new(
            self.x.div_euclid(POS_SCALE) as i32,
            self.y.div_euclid(POS_SCALE) as i32,
        )
    }
    /// Position in pixels as floating point, for rendering only.
    pub fn to_f32(self) -> (f32, f32) {
        (
            self.x as f64 as f32 / POS_SCALE as f32,
            self.y as f64 as f32 / POS_SCALE as f32,
        )
    }
}

impl Vel {
    /// Velocity from settings/wire units (pixels per 10 s).
    pub const fn from_wire(x: i32, y: i32) -> Self {
        Self::new(x * VEL_SCALE, y * VEL_SCALE)
    }
    /// Velocity in wire units (pixels per 10 s), rounded toward zero.
    pub const fn to_wire(self) -> Vec2<i32> {
        Vec2::new(self.x / VEL_SCALE, self.y / VEL_SCALE)
    }
    /// Magnitude in the same units.
    pub fn length(self) -> i64 {
        isqrt(self.x as i64 * self.x as i64 + self.y as i64 * self.y as i64)
    }
}

/// Integer square root (floor) of a non-negative value; negatives give 0.
pub const fn isqrt(v: i64) -> i64 {
    if v <= 0 {
        return 0;
    }
    let mut x = v as u64;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + v as u64 / x) / 2;
    }
    x as i64
}

/// Division rounding half away from zero (symmetric, so no drift on negative values).
pub const fn div_round(a: i64, b: i64) -> i64 {
    if (a < 0) != (b < 0) {
        (a - b / 2) / b
    } else {
        (a + b / 2) / b
    }
}

/// sin of an angle in 1/40000 turn, in 2^30 fixed point. Integer-only Taylor series, so
/// results are bit-identical on every platform.
pub const fn sin_q30(angle: i64) -> i64 {
    let a = angle.rem_euclid(ROTATION_UNITS as i64);
    let quarter = ROTATION_UNITS as i64 / 4;
    let (q, r) = (a / quarter, a % quarter);
    // Angle within the quadrant, mirrored so we always evaluate on [0, pi/2].
    let r = if q % 2 == 1 { quarter - r } else { r };
    // x = r / quarter * pi/2, in 2^30 fixed point. pi/2 * 2^30 = 1686629713.065...
    let x = (r as i128 * 1_686_629_713) / quarter as i128;
    let one: i128 = 1 << 30;
    let x2 = x * x / one;
    let mut term = x;
    let mut sum = x;
    let mut k = 1;
    while k <= 7 {
        term = -term * x2 / one / ((2 * k) * (2 * k + 1));
        sum += term;
        k += 1;
    }
    let sum = if sum > one { one } else { sum };
    let s = sum as i64;
    if q >= 2 { -s } else { s }
}

/// cos of an angle in 1/40000 turn, in 2^30 fixed point.
pub const fn cos_q30(angle: i64) -> i64 {
    sin_q30(angle + ROTATION_UNITS as i64 / 4)
}

/// Unit heading for an orientation (1/40000 turn, 0 = up/north, clockwise), in
/// [`UNIT_BITS`] fixed point. Screen y grows downward.
pub const fn heading(angle: i64) -> Vec2<i32> {
    let s = sin_q30(angle);
    let c = cos_q30(angle);
    let shift = 30 - UNIT_BITS;
    let half = 1 << (shift - 1);
    let x = if s >= 0 {
        (s + half) >> shift
    } else {
        -((-s + half) >> shift)
    };
    let y = if c >= 0 {
        (c + half) >> shift
    } else {
        -((-c + half) >> shift)
    };
    Vec2::new(x as i32, -y as i32)
}

/// Headings for the 40 discrete directions SubSpace uses for thrust, firing and the
/// ship sprite frame.
pub const DIRECTION_HEADINGS: [Vec2<i32>; 40] = {
    let mut out = [Vec2::new(0, 0); 40];
    let mut i = 0;
    while i < 40 {
        out[i] = heading(i as i64 * 1000);
        i += 1;
    }
    out
};

/// Scales a unit vector (in [`UNIT_BITS`] fixed point) by `magnitude`, rounding
/// symmetrically.
pub const fn scale_unit(unit: Vec2<i32>, magnitude: i64) -> Vec2<i64> {
    Vec2::new(
        div_round(unit.x as i64 * magnitude, UNIT_ONE as i64),
        div_round(unit.y as i64 * magnitude, UNIT_ONE as i64),
    )
}

/// Small deterministic RNG (xorshift32) for anything random inside the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rng(u32);

impl Rng {
    pub const fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.next_u32() % n }
    }
}

/// The pseudo-random generator SubSpace uses for anything every client must agree on
/// (prize and green positions, door seeds, shrapnel spread): a Park–Miller minimal
/// standard generator (Schrage's method) with an added offset of 123.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VieRng {
    pub seed: i32,
}

impl VieRng {
    pub const fn new(seed: u32) -> Self {
        Self { seed: seed as i32 }
    }

    /// Advances and returns the new seed (always 1..=0x7FFF_FFFF after the first call
    /// from a positive seed).
    pub fn next_u32(&mut self) -> u32 {
        let s = self.seed;
        let mut n = (s % 127_773)
            .wrapping_mul(16_807)
            .wrapping_sub((s / 127_773).wrapping_mul(2_836))
            .wrapping_add(123);
        if n < 1 {
            n = n.wrapping_add(0x7FFF_FFFF);
        }
        self.seed = n;
        n as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_exact() {
        for v in [0i64, 1, 2, 3, 4, 15, 16, 17, 1 << 40, (1 << 62) - 1] {
            let r = isqrt(v);
            assert!(r * r <= v && (r + 1) * (r + 1) > v, "{v}");
        }
    }

    #[test]
    fn sine_matches_float() {
        for a in (0..ROTATION_UNITS as i64).step_by(37) {
            let want = (a as f64 / ROTATION_UNITS as f64 * std::f64::consts::TAU).sin();
            let got = sin_q30(a) as f64 / (1u64 << 30) as f64;
            assert!((want - got).abs() < 1e-8, "angle {a}: {want} vs {got}");
        }
    }

    #[test]
    fn cardinal_headings() {
        let one = UNIT_ONE;
        assert_eq!(DIRECTION_HEADINGS[0], Vec2::new(0, -one)); // up
        assert_eq!(DIRECTION_HEADINGS[10], Vec2::new(one, 0)); // right
        assert_eq!(DIRECTION_HEADINGS[20], Vec2::new(0, one)); // down
        assert_eq!(DIRECTION_HEADINGS[30], Vec2::new(-one, 0)); // left
        // Symmetry: opposite directions are exact negations.
        for d in 0..20 {
            let a = DIRECTION_HEADINGS[d];
            let b = DIRECTION_HEADINGS[d + 20];
            assert_eq!((a.x, a.y), (-b.x, -b.y), "direction {d}");
        }
    }

    #[test]
    fn vie_rng_sequence() {
        // Reference values from the recurrence (16930 * 16807 + 123 = 284542633).
        let mut r = VieRng::new(1);
        assert_eq!(r.next_u32(), 16_807 + 123);
        assert_eq!(r.next_u32(), 284_542_633);
        let mut r = VieRng::new(0);
        assert_eq!(r.next_u32(), 123);
        let mut r = VieRng::new(-5i32 as u32);
        let v = r.next_u32();
        assert!((1..=0x7FFF_FFFF).contains(&v));
    }

    #[test]
    fn rounding_is_symmetric() {
        assert_eq!(div_round(5, 2), 3);
        assert_eq!(div_round(-5, 2), -3);
        assert_eq!(div_round(4, 3), 1);
        assert_eq!(div_round(-4, 3), -1);
    }
}
