//! A picture processing unit.
//!
//! The simplest thing that is still recognisably a PPU: a paletted bitmap.
//! Video RAM is plain memory — the guest writes it with ordinary stores, at
//! any width, as fast as the bus goes — holding a 256-entry palette of RGB555
//! colours and one byte per pixel indexing into it. [`Ppu::render`] resolves
//! the two into RGBA8888, the layout a canvas `ImageData` wants, so the host
//! can hand the framebuffer straight to the page.
//!
//! Indirecting pixels through a palette is what makes the cheap tricks of the
//! era possible: the guest animates a whole screen by rewriting sixteen palette
//! entries instead of forty thousand pixels.
//!
//! The display is always on. Video RAM starts zeroed and palette entry 0 is
//! black, so a program shows a black screen until it draws something.
//!
//! # Video RAM layout
//!
//! | offset    | contents                                            |
//! |-----------|-----------------------------------------------------|
//! | `0x0000`  | palette — 256 entries of RGB555                     |
//! | `0x0200`  | VRAM — [`WIDTH`] × [`HEIGHT`] palette indices, row-major |
//!
//! # Register map
//!
//! | offset    | access | register                                  |
//! |-----------|--------|-------------------------------------------|
//! | `0x0000`  | r      | `FRAME` (u32) — frames rendered so far     |
//!
//! ```
//! use sgsae::ppu::{self, Ppu};
//!
//! let mut video = vec![0u8; ppu::VIDEO_SIZE];
//! video[2..4].copy_from_slice(&ppu::rgb(31, 0, 0).to_le_bytes()); // entry 1 = red
//! video[ppu::VRAM as usize] = 1;                                   // pixel 0 = entry 1
//!
//! let mut ppu = Ppu::new();
//! ppu.render(&video);
//! assert_eq!(&ppu.framebuffer()[0..4], &[255, 0, 0, 255]);
//! ```

use crate::memory::Device;

/// Screen width in pixels.
pub const WIDTH: usize = 240;
/// Screen height in pixels.
pub const HEIGHT: usize = 160;
/// Total pixels, and therefore bytes of VRAM.
pub const PIXELS: usize = WIDTH * HEIGHT;
/// Palette entries.
pub const PALETTE_ENTRIES: usize = 256;

/// Offset of the palette in video RAM: 256 RGB555 entries.
pub const PALETTE: u32 = 0x0000;
/// Offset of VRAM in video RAM: one palette index per pixel.
pub const VRAM: u32 = 0x0200;
/// Bytes of video RAM.
pub const VIDEO_SIZE: usize = VRAM as usize + PIXELS;

/// Bytes of address space the register device occupies.
pub const SIZE: u32 = 4;

/// Pack five-bit components into an RGB555 palette entry.
///
/// Components above 31 saturate.
pub const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    let (r, g, b) = (min5(r), min5(g), min5(b));
    (r as u16) | ((g as u16) << 5) | ((b as u16) << 10)
}

const fn min5(x: u8) -> u8 {
    if x > 31 {
        31
    } else {
        x
    }
}

/// Widen a 5-bit component to 8 bits, so full-scale maps to 255 rather than 248.
const fn widen(component: u16) -> u8 {
    let c = (component & 0x1F) as u8;
    (c << 3) | (c >> 2)
}

/// Expand an RGB555 entry to RGBA8888.
const fn to_rgba(entry: u16) -> [u8; 4] {
    [widen(entry), widen(entry >> 5), widen(entry >> 10), 0xFF]
}

/// A paletted bitmap display.
pub struct Ppu {
    frame: u32,
    framebuffer: Box<[u8]>,
}

impl Ppu {
    /// `FRAME`: a 32-bit count of frames rendered.
    pub const FRAME: u32 = 0x0000;

    pub fn new() -> Ppu {
        Ppu { frame: 0, framebuffer: vec![0; PIXELS * 4].into_boxed_slice() }
    }

    /// Frames rendered since power-on.
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// The last rendered frame as RGBA8888, `WIDTH * HEIGHT * 4` bytes.
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Resolve `video` — palette then VRAM, laid out as above — into the
    /// framebuffer, and count the frame.
    pub fn render(&mut self, video: &[u8]) {
        let palette = &video[PALETTE as usize..VRAM as usize];
        let vram = &video[VRAM as usize..VIDEO_SIZE];

        // Expand the palette once per frame rather than per pixel.
        let mut colours = [[0u8; 4]; PALETTE_ENTRIES];
        for (slot, entry) in colours.iter_mut().zip(palette.chunks_exact(2)) {
            *slot = to_rgba(u16::from_le_bytes([entry[0], entry[1]]));
        }
        for (pixel, index) in self.framebuffer.chunks_exact_mut(4).zip(vram) {
            pixel.copy_from_slice(&colours[usize::from(*index)]);
        }
        self.frame = self.frame.wrapping_add(1);
    }
}

impl Device for Ppu {
    fn size(&self) -> u32 {
        SIZE
    }
    fn read_byte(&mut self, offset: u32) -> u8 {
        self.frame.to_le_bytes()[(offset - Self::FRAME) as usize]
    }
    fn write_byte(&mut self, _: u32, _: u8) {}
}

/// Encode an RGBA framebuffer as a binary PPM image.
///
/// The simplest format that every image viewer understands, for looking at a
/// frame without a browser in the loop.
pub fn ppm(framebuffer: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + PIXELS * 3);
    out.extend_from_slice(format!("P6\n{WIDTH} {HEIGHT}\n255\n").as_bytes());
    for pixel in framebuffer.chunks_exact(4) {
        out.extend_from_slice(&pixel[..3]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Width;

    const RED: u16 = rgb(31, 0, 0);
    const GREEN: u16 = rgb(0, 31, 0);
    const BLUE: u16 = rgb(0, 0, 31);

    /// Video RAM with entries 1..=3 set to red, green, and blue.
    fn video() -> Vec<u8> {
        let mut video = vec![0u8; VIDEO_SIZE];
        for (i, colour) in [RED, GREEN, BLUE].iter().enumerate() {
            let at = PALETTE as usize + (i + 1) * 2;
            video[at..at + 2].copy_from_slice(&colour.to_le_bytes());
        }
        video
    }

    fn pixel(p: &Ppu, x: usize, y: usize) -> [u8; 4] {
        let at = (y * WIDTH + x) * 4;
        p.framebuffer()[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn rgb555_expands_so_full_scale_is_full_brightness() {
        assert_eq!(to_rgba(rgb(31, 31, 31)), [255, 255, 255, 255]);
        assert_eq!(to_rgba(rgb(0, 0, 0)), [0, 0, 0, 255]);
        assert_eq!(to_rgba(RED), [255, 0, 0, 255]);
        assert_eq!(to_rgba(GREEN), [0, 255, 0, 255]);
        assert_eq!(to_rgba(BLUE), [0, 0, 255, 255]);
        // Components saturate rather than bleeding into their neighbours.
        assert_eq!(rgb(99, 0, 0), rgb(31, 0, 0));
    }

    #[test]
    fn vram_indices_resolve_through_the_palette() {
        let mut video = video();
        let vram = VRAM as usize;
        video[vram] = 1;
        video[vram + 1] = 2;
        video[vram + WIDTH] = 3;
        let mut p = Ppu::new();
        p.render(&video);

        assert_eq!(pixel(&p, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&p, 1, 0), [0, 255, 0, 255]);
        assert_eq!(pixel(&p, 0, 1), [0, 0, 255, 255]);
        // Index 0 was never given a colour, so it stays black.
        assert_eq!(pixel(&p, 2, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn the_framebuffer_is_row_major_and_fully_covered() {
        let mut video = video();
        video[VRAM as usize..].fill(1);
        let mut p = Ppu::new();
        p.render(&video);
        assert_eq!(p.framebuffer().len(), WIDTH * HEIGHT * 4);
        assert!(
            p.framebuffer().chunks_exact(4).all(|px| px == [255, 0, 0, 255]),
            "some pixels were not painted"
        );
    }

    #[test]
    fn rewriting_the_palette_recolours_the_screen_without_touching_vram() {
        let mut video = video();
        video[VRAM as usize] = 1;
        let mut p = Ppu::new();
        p.render(&video);
        assert_eq!(pixel(&p, 0, 0), [255, 0, 0, 255]);

        // The whole point of a paletted display: one write, whole screen.
        video[2..4].copy_from_slice(&BLUE.to_le_bytes());
        p.render(&video);
        assert_eq!(pixel(&p, 0, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn the_frame_counter_advances_and_is_readable_by_the_guest() {
        let video = video();
        let mut p = Ppu::new();
        assert_eq!(p.read(Ppu::FRAME, Width::Word), 0);
        for expected in 1..=3 {
            p.render(&video);
            assert_eq!(p.read(Ppu::FRAME, Width::Word), expected);
            assert_eq!(p.frame(), expected);
        }
        // And it cannot be written.
        p.write(Ppu::FRAME, Width::Word, 99);
        assert_eq!(p.frame(), 3);
    }

    #[test]
    fn ppm_has_a_header_and_three_bytes_per_pixel() {
        let mut video = video();
        video[VRAM as usize] = 1;
        let mut p = Ppu::new();
        p.render(&video);

        let bytes = ppm(p.framebuffer());
        let header = format!("P6\n{WIDTH} {HEIGHT}\n255\n");
        assert!(bytes.starts_with(header.as_bytes()));
        assert_eq!(bytes.len(), header.len() + PIXELS * 3);
        // The alpha channel is dropped, not passed through.
        assert_eq!(&bytes[header.len()..header.len() + 3], &[255, 0, 0]);
    }
}
