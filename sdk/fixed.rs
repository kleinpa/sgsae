//! Fixed point, angles, and 2D vectors.
//!
//! There is no floating point unit, and `no_std` means no `sin` to call, so
//! everything geometric on this console is integers. [`Fx`] is Q16.16 — one
//! unit is `1 << 16` — which leaves room for a screen coordinate and about
//! four decimal digits of fraction under it.
//!
//! ```
//! # use console::fixed::{Angle, Fx, Vec2};
//! let nose = Vec2::new(Fx::ZERO, Fx::from_int(-6));
//! let pointing = nose.rotate(Angle::from_degrees(90));
//! ```
//!
//! Angles are a [`Angle`], a u16 of turn, so they wrap by themselves and never
//! need a range check — the trick every console of this era used. Sine comes
//! off a quarter-wave table by symmetry, so it costs a shift, a lookup and a
//! negate.

use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Fractional bits in an [`Fx`].
pub const SHIFT: u32 = 16;
/// [`Fx`] units in one whole number.
pub const ONE: i32 = 1 << SHIFT;

/// A Q16.16 fixed-point number.
///
/// Arithmetic is the ordinary operators; `*` and `/` rescale, so the type
/// reads like the maths it stands for rather than like a pile of shifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Fx(pub i32);

impl Fx {
    pub const ZERO: Fx = Fx(0);
    pub const ONE: Fx = Fx(ONE);
    pub const HALF: Fx = Fx(ONE / 2);

    /// A whole number.
    pub const fn from_int(value: i32) -> Fx {
        Fx(value << SHIFT)
    }

    /// A fraction, `numerator / denominator`, evaluated at full precision.
    ///
    /// `const`, so tuning constants can be written the way they are meant —
    /// `Fx::ratio(3, 100)` rather than a raw `1966`.
    pub const fn ratio(numerator: i32, denominator: i32) -> Fx {
        Fx(((numerator as i64 * ONE as i64) / denominator as i64) as i32)
    }

    /// The raw Q16.16 bits.
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// From raw Q16.16 bits.
    pub const fn from_raw(bits: i32) -> Fx {
        Fx(bits)
    }

    /// Round towards negative infinity, so that pixel coordinates step
    /// evenly through zero instead of bunching up either side of it.
    pub const fn floor(self) -> i32 {
        self.0 >> SHIFT
    }

    /// Round to nearest.
    pub const fn round(self) -> i32 {
        (self.0 + ONE / 2) >> SHIFT
    }

    pub const fn abs(self) -> Fx {
        Fx(self.0.abs())
    }

    pub const fn min(self, other: Fx) -> Fx {
        if self.0 < other.0 { self } else { other }
    }

    pub const fn max(self, other: Fx) -> Fx {
        if self.0 > other.0 { self } else { other }
    }

    pub const fn clamp(self, low: Fx, high: Fx) -> Fx {
        self.max(low).min(high)
    }

    /// Multiply, keeping the intermediate at 64 bits so that ordinary screen
    /// magnitudes cannot overflow on the way through.
    pub const fn mul(self, other: Fx) -> Fx {
        Fx(((self.0 as i64 * other.0 as i64) >> SHIFT) as i32)
    }

    pub const fn div(self, other: Fx) -> Fx {
        if other.0 == 0 {
            return Fx(0);
        }
        Fx((((self.0 as i64) << SHIFT) / other.0 as i64) as i32)
    }

    /// Square root, by Newton's method on integers.
    ///
    /// Used for vector length; five iterations settle to within a bit at
    /// screen magnitudes.
    pub fn sqrt(self) -> Fx {
        if self.0 <= 0 {
            return Fx::ZERO;
        }
        // Work in raw units: sqrt(x * ONE) is the fixed-point root of x.
        let target = (self.0 as i64) << SHIFT;
        let mut guess = self.0 as i64;
        for _ in 0..8 {
            if guess == 0 {
                break;
            }
            let next = (guess + target / guess) >> 1;
            if (next - guess).abs() <= 1 {
                guess = next;
                break;
            }
            guess = next;
        }
        Fx(guess as i32)
    }
}

impl Add for Fx {
    type Output = Fx;
    fn add(self, other: Fx) -> Fx {
        Fx(self.0 + other.0)
    }
}

impl Sub for Fx {
    type Output = Fx;
    fn sub(self, other: Fx) -> Fx {
        Fx(self.0 - other.0)
    }
}

impl Neg for Fx {
    type Output = Fx;
    fn neg(self) -> Fx {
        Fx(-self.0)
    }
}

impl Mul for Fx {
    type Output = Fx;
    fn mul(self, other: Fx) -> Fx {
        Fx::mul(self, other)
    }
}

/// Scaling by a plain integer is exact and needs no rescale, so it is worth
/// having as its own impl rather than converting first.
impl Mul<i32> for Fx {
    type Output = Fx;
    fn mul(self, scale: i32) -> Fx {
        Fx(self.0 * scale)
    }
}

impl Div for Fx {
    type Output = Fx;
    fn div(self, other: Fx) -> Fx {
        Fx::div(self, other)
    }
}

impl Div<i32> for Fx {
    type Output = Fx;
    fn div(self, divisor: i32) -> Fx {
        if divisor == 0 { Fx::ZERO } else { Fx(self.0 / divisor) }
    }
}

impl AddAssign for Fx {
    fn add_assign(&mut self, other: Fx) {
        self.0 += other.0;
    }
}

impl SubAssign for Fx {
    fn sub_assign(&mut self, other: Fx) {
        self.0 -= other.0;
    }
}

impl From<i32> for Fx {
    fn from(value: i32) -> Fx {
        Fx::from_int(value)
    }
}

/// An angle, as a fraction of a turn.
///
/// A `u16` covers the circle exactly, so adding a turn rate every frame wraps
/// on its own and there is never a range to normalise. Zero points along +x
/// and the angle increases towards +y, which is *clockwise* on screen because
/// y grows downwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Angle(pub u16);

impl Angle {
    /// A quarter turn: the unit the named constants are built from.
    pub const QUARTER: Angle = Angle(0x4000);
    pub const ZERO: Angle = Angle(0);
    pub const HALF: Angle = Angle(0x8000);

    /// Straight up the screen. Worth naming: y points down here, so "up" is
    /// three quarters of a turn, and writing that out at every call site is
    /// how sign errors get in.
    pub const UP: Angle = Angle(0xC000);
    pub const DOWN: Angle = Angle(0x4000);
    pub const RIGHT: Angle = Angle(0);
    pub const LEFT: Angle = Angle(0x8000);

    /// From degrees. Any integer works; the circle wraps.
    pub const fn from_degrees(degrees: i32) -> Angle {
        Angle((degrees.wrapping_mul(0x10000) / 360) as u16)
    }

    /// In degrees, `0..360`.
    pub const fn to_degrees(self) -> i32 {
        self.0 as i32 * 360 / 0x10000
    }

    /// Turn by `delta` sixteenth-thousandths of a circle. Wraps.
    pub const fn wrapping_add(self, delta: i32) -> Angle {
        Angle(self.0.wrapping_add(delta as u16))
    }

    /// The signed difference to `other`, taking the short way round.
    ///
    /// Wrapping subtraction lands in `-32768..32768` for free, which is
    /// exactly the shortest arc — no branch on which side of the circle the
    /// two angles fell.
    pub const fn delta(self, other: Angle) -> i32 {
        self.0.wrapping_sub(other.0) as i16 as i32
    }

    pub fn sin(self) -> Fx {
        Fx(sine(self.0))
    }

    pub fn cos(self) -> Fx {
        Fx(sine(self.0.wrapping_add(Angle::QUARTER.0)))
    }

    /// The unit vector pointing this way.
    pub fn unit(self) -> Vec2 {
        Vec2 { x: self.cos(), y: self.sin() }
    }
}

impl Add for Angle {
    type Output = Angle;
    fn add(self, other: Angle) -> Angle {
        Angle(self.0.wrapping_add(other.0))
    }
}

impl Sub for Angle {
    type Output = Angle;
    fn sub(self, other: Angle) -> Angle {
        Angle(self.0.wrapping_sub(other.0))
    }
}

impl AddAssign for Angle {
    fn add_assign(&mut self, other: Angle) {
        self.0 = self.0.wrapping_add(other.0);
    }
}

/// Sine of a u16 turn, in Q16.16.
///
/// The table is a quarter wave; the other three quarters are that one
/// mirrored and negated. Storing all four would cost 1 KiB of ROM to save two
/// branches, which is the wrong trade on a machine with this much ROM and
/// this little else to do.
fn sine(turn: u16) -> i32 {
    let quadrant = turn >> 14;
    let index = (turn >> 6) & 0xFF;
    // Quadrants 1 and 3 run backwards through the table.
    let sample = if quadrant & 1 == 0 {
        QUARTER_SINE[index as usize]
    } else {
        QUARTER_SINE[256 - index as usize]
    } as i32;
    if quadrant & 2 == 0 { sample } else { -sample }
}

/// One quarter of a sine wave, 257 entries so the endpoint is exact, in
/// Q16.16. Generated as `round(sin(i * PI / 512) * 65536)`.
#[rustfmt::skip]
static QUARTER_SINE: [i32; 257] = [
    0, 402, 804, 1206, 1608, 2010, 2412, 2814, 3216, 3617, 4019, 4420, 4821,
    5222, 5623, 6023, 6424, 6824, 7224, 7623, 8022, 8421, 8820, 9218, 9616,
    10014, 10411, 10808, 11204, 11600, 11996, 12391, 12785, 13179, 13573,
    13966, 14359, 14751, 15143, 15534, 15924, 16314, 16703, 17091, 17479,
    17866, 18253, 18639, 19024, 19408, 19792, 20175, 20557, 20939, 21320,
    21699, 22078, 22457, 22834, 23210, 23586, 23961, 24335, 24708, 25080,
    25451, 25821, 26190, 26558, 26925, 27291, 27656, 28020, 28383, 28745,
    29106, 29466, 29824, 30182, 30538, 30893, 31248, 31600, 31952, 32303,
    32652, 33000, 33347, 33692, 34037, 34380, 34721, 35062, 35401, 35738,
    36075, 36410, 36744, 37076, 37407, 37736, 38064, 38391, 38716, 39040,
    39362, 39683, 40002, 40320, 40636, 40951, 41264, 41576, 41886, 42194,
    42501, 42806, 43110, 43412, 43713, 44011, 44308, 44604, 44898, 45190,
    45480, 45769, 46056, 46341, 46624, 46906, 47186, 47464, 47741, 48015,
    48288, 48559, 48828, 49095, 49361, 49624, 49886, 50146, 50404, 50660,
    50914, 51166, 51417, 51665, 51911, 52156, 52398, 52639, 52878, 53114,
    53349, 53581, 53812, 54040, 54267, 54491, 54714, 54934, 55152, 55368,
    55582, 55794, 56004, 56212, 56418, 56621, 56823, 57022, 57219, 57414,
    57607, 57798, 57986, 58172, 58356, 58538, 58718, 58896, 59071, 59244,
    59415, 59583, 59750, 59914, 60075, 60235, 60392, 60547, 60700, 60851,
    60999, 61145, 61288, 61429, 61568, 61705, 61839, 61971, 62100, 62228,
    62353, 62475, 62596, 62714, 62829, 62942, 63053, 63162, 63268, 63371,
    63473, 63572, 63668, 63763, 63854, 63944, 64031, 64115, 64197, 64277,
    64354, 64429, 64501, 64571, 64639, 64704, 64766, 64827, 64884, 64940,
    64993, 65043, 65091, 65137, 65180, 65220, 65259, 65294, 65328, 65358,
    65387, 65413, 65436, 65457, 65476, 65492, 65505, 65516, 65525, 65531,
    65535, 65536,
];

/// A point or a direction, in fixed point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vec2 {
    pub x: Fx,
    pub y: Fx,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: Fx::ZERO, y: Fx::ZERO };

    pub const fn new(x: Fx, y: Fx) -> Vec2 {
        Vec2 { x, y }
    }

    /// From whole numbers, which is how model geometry is usually written.
    pub const fn int(x: i32, y: i32) -> Vec2 {
        Vec2 { x: Fx::from_int(x), y: Fx::from_int(y) }
    }

    /// To whole pixels, for handing to the rasteriser.
    pub const fn to_pixels(self) -> (i32, i32) {
        (self.x.round(), self.y.round())
    }

    pub fn dot(self, other: Vec2) -> Fx {
        self.x * other.x + self.y * other.y
    }

    /// The z component of the cross product: positive when `other` is
    /// clockwise of `self` on screen. Which side of a line a point falls on,
    /// in one multiply-subtract.
    pub fn cross(self, other: Vec2) -> Fx {
        self.x * other.y - self.y * other.x
    }

    pub fn length(self) -> Fx {
        // Squaring first would overflow past about 180 pixels, so scale down
        // to whole units, square there, and scale the root back up.
        let (x, y) = (self.x.0 as i64, self.y.0 as i64);
        Fx::from_raw((((x * x + y * y) >> SHIFT) as i32).max(0)).sqrt()
    }

    /// Rotate clockwise on screen by `angle`.
    pub fn rotate(self, angle: Angle) -> Vec2 {
        let (sin, cos) = (angle.sin(), angle.cos());
        Vec2 { x: self.x * cos - self.y * sin, y: self.x * sin + self.y * cos }
    }

    pub fn scale(self, factor: Fx) -> Vec2 {
        Vec2 { x: self.x * factor, y: self.y * factor }
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, other: Vec2) -> Vec2 {
        Vec2 { x: self.x + other.x, y: self.y + other.y }
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, other: Vec2) -> Vec2 {
        Vec2 { x: self.x - other.x, y: self.y - other.y }
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2 { x: -self.x, y: -self.y }
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, other: Vec2) {
        *self = *self + other;
    }
}

impl SubAssign for Vec2 {
    fn sub_assign(&mut self, other: Vec2) {
        *self = *self - other;
    }
}
