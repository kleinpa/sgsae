//! The screen: a 240x160 paletted bitmap.
//!
//! Pixels are palette indices, so recolouring the whole display costs one
//! [`Video::set_palette`] call rather than forty thousand writes. The palette
//! and the pixels are plain video RAM; the display is always on, and shows
//! black until something is drawn.

use crate::map;

/// Screen width in pixels.
pub const WIDTH: i32 = 240;
/// Screen height in pixels.
pub const HEIGHT: i32 = 160;
/// Palette entries.
pub const COLOURS: usize = 256;

/// The one register, at `map::PPU`.
const FRAME: usize = 0x0000;
/// Offsets into video RAM at `map::VIDEO`.
const PALETTE: usize = 0x0000;
const VRAM: usize = 0x0200;

/// A colour, five bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb(u16);

impl Rgb {
    /// Components saturate at 31 rather than bleeding into the next channel.
    pub const fn new(r: u8, g: u8, b: u8) -> Rgb {
        const fn clamp(x: u8) -> u16 {
            if x > 31 {
                31
            } else {
                x as u16
            }
        }
        Rgb(clamp(r) | (clamp(g) << 5) | (clamp(b) << 10))
    }

    pub const BLACK: Rgb = Rgb::new(0, 0, 0);
    pub const WHITE: Rgb = Rgb::new(31, 31, 31);

    pub const fn bits(self) -> u16 {
        self.0
    }
}

/// The display.
pub struct Video {
    _private: (),
}

impl Video {
    pub(crate) const fn new() -> Video {
        Video { _private: () }
    }

    /// Frames the display has drawn since power-on. Useful as a clock.
    pub fn frame(&self) -> u32 {
        unsafe { core::ptr::read_volatile((map::PPU + FRAME) as *const u32) }
    }

    pub fn set_palette(&mut self, index: u8, colour: Rgb) {
        let at = (map::VIDEO + PALETTE) as *mut u16;
        unsafe { core::ptr::write_volatile(at.add(index as usize), colour.bits()) }
    }

    /// Load a run of palette entries starting at `first`.
    pub fn set_palettes(&mut self, first: u8, colours: &[Rgb]) {
        for (i, colour) in colours.iter().enumerate() {
            self.set_palette(first.wrapping_add(i as u8), *colour);
        }
    }

    fn vram(&self) -> *mut u8 {
        (map::VIDEO + VRAM) as *mut u8
    }

    /// Fill the screen.
    ///
    /// Writes whole words, so the interpreter running this executes a quarter
    /// as many stores. The clear is usually a game's single biggest cost.
    pub fn clear(&mut self, colour: u8) {
        let packed = u32::from_ne_bytes([colour; 4]);
        let base = self.vram() as *mut u32;
        for i in 0..(WIDTH * HEIGHT / 4) as usize {
            unsafe { core::ptr::write_volatile(base.add(i), packed) }
        }
    }

    /// Set one pixel. Out-of-bounds coordinates are dropped.
    #[inline]
    pub fn pixel(&mut self, x: i32, y: i32, colour: u8) {
        if (0..WIDTH).contains(&x) && (0..HEIGHT).contains(&y) {
            unsafe { core::ptr::write_volatile(self.vram().add((y * WIDTH + x) as usize), colour) }
        }
    }

    /// Horizontal run from `x` for `width` pixels, clipped.
    pub fn hline(&mut self, x: i32, y: i32, width: i32, colour: u8) {
        if !(0..HEIGHT).contains(&y) {
            return;
        }
        let start = x.max(0);
        let end = (x + width).min(WIDTH);
        let mut at = (y * WIDTH + start) as usize;
        for _ in start..end {
            unsafe { core::ptr::write_volatile(self.vram().add(at), colour) }
            at += 1;
        }
    }

    /// Vertical run from `y` for `height` pixels, clipped.
    pub fn vline(&mut self, x: i32, y: i32, height: i32, colour: u8) {
        if !(0..WIDTH).contains(&x) {
            return;
        }
        let start = y.max(0);
        let end = (y + height).min(HEIGHT);
        let mut at = (start * WIDTH + x) as usize;
        for _ in start..end {
            unsafe { core::ptr::write_volatile(self.vram().add(at), colour) }
            at += WIDTH as usize;
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, colour: u8) {
        for row in y..y + height {
            self.hline(x, row, width, colour);
        }
    }

    /// A line from `(x0, y0)` to `(x1, y1)`, endpoints included.
    ///
    /// Bresenham, but clipped *before* it starts rather than by dropping
    /// pixels as it goes: a line drawn from far off-screen would otherwise
    /// cost thousands of iterations to paint the handful of pixels that
    /// actually land. That matters here because the whole point of a vector
    /// game is that geometry may be anywhere.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, colour: u8) {
        let Some((mut x, mut y, ex, ey)) = clip_line(x0, y0, x1, y1) else {
            return;
        };

        let dx = (ex - x).abs();
        let dy = -(ey - y).abs();
        let step_x = if x < ex { 1 } else { -1 };
        let step_y = if y < ey { 1 } else { -1 };
        let mut error = dx + dy;

        loop {
            // Clipping already proved every point is inside, so this writes
            // straight to VRAM rather than paying for a bounds check per pixel.
            unsafe { core::ptr::write_volatile(self.vram().add((y * WIDTH + x) as usize), colour) }
            if x == ex && y == ey {
                break;
            }
            let doubled = error * 2;
            if doubled >= dy {
                error += dy;
                x += step_x;
            }
            if doubled <= dx {
                error += dx;
                y += step_y;
            }
        }
    }

    /// Connect `points` in order. Does not close the shape.
    pub fn polyline(&mut self, points: &[(i32, i32)], colour: u8) {
        for pair in points.windows(2) {
            self.line(pair[0].0, pair[0].1, pair[1].0, pair[1].1, colour);
        }
    }

    /// Connect `points` in order and close the loop back to the first.
    pub fn polygon(&mut self, points: &[(i32, i32)], colour: u8) {
        self.polyline(points, colour);
        if let (Some(first), Some(last)) = (points.first(), points.last()) {
            if points.len() > 2 {
                self.line(last.0, last.1, first.0, first.1, colour);
            }
        }
    }

    /// A circle outline, by the midpoint method: eight-way symmetry, so only
    /// one octant is actually stepped.
    pub fn circle(&mut self, cx: i32, cy: i32, radius: i32, colour: u8) {
        if radius < 0 {
            return;
        }
        let (mut x, mut y) = (radius, 0);
        let mut error = 1 - radius;
        while x >= y {
            for (px, py) in [
                (cx + x, cy + y),
                (cx + y, cy + x),
                (cx - y, cy + x),
                (cx - x, cy + y),
                (cx - x, cy - y),
                (cx - y, cy - x),
                (cx + y, cy - x),
                (cx + x, cy - y),
            ] {
                self.pixel(px, py, colour);
            }
            y += 1;
            if error < 0 {
                error += 2 * y + 1;
            } else {
                x -= 1;
                error += 2 * (y - x) + 1;
            }
        }
    }

    /// A rectangle outline. Named to leave [`Video::frame`], the frame
    /// counter, alone.
    pub fn outline(&mut self, x: i32, y: i32, width: i32, height: i32, colour: u8) {
        self.hline(x, y, width, colour);
        self.hline(x, y + height - 1, width, colour);
        self.vline(x, y, height, colour);
        self.vline(x + width - 1, y, height, colour);
    }

    /// Draw one 3x5 glyph, top-left at `x`, `y`.
    pub fn glyph(&mut self, x: i32, y: i32, glyph: u16, colour: u8) {
        for row in 0..5 {
            for col in 0..3 {
                if glyph >> (14 - row * 3 - col) & 1 != 0 {
                    self.pixel(x + col as i32, y + row as i32, colour);
                }
            }
        }
    }

    /// A 3x5 digit, top-left at `x`, `y`.
    pub fn digit(&mut self, x: i32, y: i32, value: u32, colour: u8) {
        self.glyph(x, y, DIGITS[(value % 10) as usize], colour);
    }

    /// Uppercase text, 4 pixels per character. Lowercase is folded to upper;
    /// anything with no glyph comes out blank.
    pub fn text(&mut self, x: i32, y: i32, text: &str, colour: u8) {
        for (i, byte) in text.bytes().enumerate() {
            self.glyph(x + i as i32 * 4, y, glyph_for(byte), colour);
        }
    }

    /// Width in pixels of `text` drawn by [`Video::text`].
    pub const fn text_width(text: &str) -> i32 {
        text.len() as i32 * 4 - 1
    }

    /// A right-aligned, zero-padded hexadecimal number `digits` wide.
    ///
    /// For looking at bytes as they came off the wire, where a decimal value
    /// tells you much less than the nibbles do.
    pub fn hex(&mut self, x: i32, y: i32, mut value: u32, digits: i32, colour: u8) {
        for i in (0..digits).rev() {
            let nibble = (value & 0xF) as u8;
            let glyph = if nibble < 10 {
                DIGITS[nibble as usize]
            } else {
                LETTERS[(nibble - 10) as usize]
            };
            self.glyph(x + i * 4, y, glyph, colour);
            value >>= 4;
        }
    }

    /// A right-aligned, zero-padded number `digits` wide.
    pub fn number(&mut self, x: i32, y: i32, mut value: u32, digits: i32, colour: u8) {
        for i in (0..digits).rev() {
            self.digit(x + i * 4, y, value % 10, colour);
            value /= 10;
        }
    }
}

/// Where a point sits relative to the screen, as the four outside bits
/// Cohen–Sutherland tests.
const INSIDE: u8 = 0;
const LEFT: u8 = 1;
const RIGHT: u8 = 2;
const BELOW: u8 = 4;
const ABOVE: u8 = 8;

const fn outcode(x: i32, y: i32) -> u8 {
    let mut code = INSIDE;
    if x < 0 {
        code |= LEFT;
    } else if x >= WIDTH {
        code |= RIGHT;
    }
    if y < 0 {
        code |= ABOVE;
    } else if y >= HEIGHT {
        code |= BELOW;
    }
    code
}

/// Trim a line to the screen, or `None` if none of it lands.
///
/// Cohen–Sutherland: while either end is outside, replace it with where the
/// segment crosses the edge it is outside of. Each pass clears at least one
/// bit, so it settles in at most four.
fn clip_line(mut x0: i32, mut y0: i32, mut x1: i32, mut y1: i32) -> Option<(i32, i32, i32, i32)> {
    let mut code0 = outcode(x0, y0);
    let mut code1 = outcode(x1, y1);

    loop {
        if code0 | code1 == INSIDE {
            // Both ends inside: accept.
            return Some((x0, y0, x1, y1));
        }
        if code0 & code1 != INSIDE {
            // Both ends off the same edge: reject.
            return None;
        }

        // Move whichever end is still outside.
        let code = if code0 != INSIDE { code0 } else { code1 };
        let (dx, dy) = ((x1 - x0) as i64, (y1 - y0) as i64);
        let (x, y) = if code & BELOW != 0 {
            let edge = (HEIGHT - 1) as i64;
            ((x0 as i64 + dx * (edge - y0 as i64) / dy) as i32, HEIGHT - 1)
        } else if code & ABOVE != 0 {
            ((x0 as i64 + dx * (0 - y0 as i64) / dy) as i32, 0)
        } else if code & RIGHT != 0 {
            let edge = (WIDTH - 1) as i64;
            (WIDTH - 1, (y0 as i64 + dy * (edge - x0 as i64) / dx) as i32)
        } else {
            (0, (y0 as i64 + dy * (0 - x0 as i64) / dx) as i32)
        };

        if code == code0 {
            x0 = x;
            y0 = y;
            code0 = outcode(x0, y0);
        } else {
            x1 = x;
            y1 = y;
            code1 = outcode(x1, y1);
        }
    }
}

/// Digit glyphs, 3x5, packed row by row from the top.
const DIGITS: [u16; 10] = [
    0b111_101_101_101_111,
    0b010_110_010_010_111,
    0b111_001_111_100_111,
    0b111_001_111_001_111,
    0b101_101_111_001_001,
    0b111_100_111_001_111,
    0b111_100_111_101_111,
    0b111_001_001_001_001,
    0b111_101_111_101_111,
    0b111_101_111_001_111,
];

/// Letter glyphs, A through Z.
#[rustfmt::skip]
const LETTERS: [u16; 26] = [
    0b010_101_111_101_101, // A
    0b110_101_110_101_110, // B
    0b011_100_100_100_011, // C
    0b110_101_101_101_110, // D
    0b111_100_110_100_111, // E
    0b111_100_110_100_100, // F
    0b011_100_101_101_011, // G
    0b101_101_111_101_101, // H
    0b111_010_010_010_111, // I
    0b001_001_001_101_010, // J
    0b101_101_110_101_101, // K
    0b100_100_100_100_111, // L
    0b101_111_101_101_101, // M
    0b110_101_101_101_101, // N
    0b010_101_101_101_010, // O
    0b110_101_110_100_100, // P
    0b010_101_101_111_011, // Q
    0b110_101_110_101_101, // R
    0b011_100_010_001_110, // S
    0b111_010_010_010_010, // T
    0b101_101_101_101_111, // U
    0b101_101_101_101_010, // V
    0b101_101_101_111_101, // W
    0b101_101_010_101_101, // X
    0b101_101_010_010_010, // Y
    0b111_001_010_100_111, // Z
];

/// The glyph for an ASCII byte, or blank if there is none.
const fn glyph_for(byte: u8) -> u16 {
    match byte {
        b'A'..=b'Z' => LETTERS[(byte - b'A') as usize],
        b'a'..=b'z' => LETTERS[(byte - b'a') as usize],
        b'0'..=b'9' => DIGITS[(byte - b'0') as usize],
        b'-' => 0b000_000_111_000_000,
        b'#' => 0b101_111_101_111_101,
        b'.' => 0b000_000_000_000_010,
        b':' => 0b000_010_000_010_000,
        b'/' => 0b001_001_010_100_100,
        b'+' => 0b000_010_111_010_000,
        b'>' => 0b100_010_001_010_100,
        b'*' => 0b101_010_111_010_101,
        b'?' => 0b110_001_010_000_010,
        b'!' => 0b010_010_010_000_010,
        _ => 0,
    }
}
