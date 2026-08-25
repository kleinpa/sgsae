//! The console SDK.
//!
//! Everything a cartridge needs and nothing it does not: a reset vector, a
//! panic handler, and typed access to the hardware. Games link this instead of
//! poking raw addresses.
//!
//! ```no_run
//! #![no_std]
//! #![no_main]
//! use console::{entry, input::Button, video::Rgb, Console};
//!
//! fn main(mut console: Console) -> ! {
//!     console.video.set_palette(1, Rgb::new(31, 20, 0));
//!     loop {
//!         let pad = console.input.gamepad(0);
//!         if pad.is_held(Button::A) {
//!             console.video.clear(1);
//!         }
//!         console.wait_frame();
//!     }
//! }
//!
//! entry!(main);
//! ```
//!
//! The hardware handles live in a [`Console`], taken once at startup, in the
//! style embedded Rust uses for peripherals: there is exactly one screen, so
//! there is exactly one [`video::Video`] to reach it through.

#![no_std]

pub mod audio;
pub mod fixed;
pub mod gfx;
pub mod input;
pub mod loader;
pub mod midi;
pub mod sys;
pub mod video;

use core::panic::PanicInfo;

/// Base addresses, matching the memory map in `sgsae/console.rs`.
pub(crate) mod map {
    pub const LOADER: usize = 0x0300_0000;
    pub const CONSOLE_OUT: usize = 0x0400_0000;
    pub const INPUT: usize = 0x0500_0000;
    pub const APU: usize = 0x0600_0000;
    pub const PPU: usize = 0x0700_0000;
    pub const MIDI: usize = 0x0800_0000;
    pub const VIDEO: usize = 0x0900_0000;
}

// The reset vector. There is no C runtime, so this is all of it: point the
// stack at the top of RAM, copy .data out of ROM, clear .bss, and call the
// entry point. The symbols come from console.ld.
core::arch::global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "  la   sp, __stack_top",
    "  la   t0, __data_rom",
    "  la   t1, __data_start",
    "  la   t2, __data_end",
    "1:",
    "  beq  t1, t2, 2f",
    "  lw   t3, 0(t0)",
    "  sw   t3, 0(t1)",
    "  addi t0, t0, 4",
    "  addi t1, t1, 4",
    "  j    1b",
    "2:",
    "  la   t0, __bss_start",
    "  la   t1, __bss_end",
    "3:",
    "  beq  t0, t1, 4f",
    "  sw   zero, 0(t0)",
    "  addi t0, t0, 4",
    "  j    3b",
    "4:",
    "  call __console_start",
);

/// The console's hardware, handed to a game once at startup.
pub struct Console {
    pub video: video::Video,
    pub audio: audio::Audio,
    pub input: input::Input,
    /// The cartridge slot: how a program hands over to another one.
    pub loader: loader::Loader,
    /// MIDI input: notes and clock from outside.
    pub midi: midi::Midi,
}

impl Console {
    /// Claim the hardware.
    ///
    /// # Safety
    ///
    /// Creates handles that assume exclusive access to the peripherals. Call
    /// it once; [`entry!`] does that for you.
    pub const unsafe fn steal() -> Console {
        Console {
            video: video::Video::new(),
            audio: audio::Audio::new(),
            input: input::Input::new(),
            loader: loader::Loader::new(),
            midi: midi::Midi::new(),
        }
    }

    /// Hand the frame to the host, which draws it and plays the audio queued
    /// for it, then come back for the next one.
    pub fn wait_frame(&mut self) {
        sys::wait_frame()
    }

    /// Hand `samples` samples of sound to the host — at most a frame — and
    /// come back. See [`sys::wait_samples`].
    pub fn wait_samples(&mut self, samples: u32) {
        sys::wait_samples(samples)
    }

    /// Write text to the host's console. There is no screen font behind this —
    /// it is for logging, not for the game's own display.
    pub fn print(&mut self, text: &str) {
        sys::print(text)
    }

    /// Stop, handing the machine back to whatever started this program.
    ///
    /// The entire exit protocol, and it costs a program one instruction.
    pub fn exit(&mut self) -> ! {
        sys::exit()
    }
}

/// Name the function the console should start.
///
/// It is called with the [`Console`] and never returns.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn __console_start() -> ! {
            // Type-check the signature before handing over.
            let entry: fn($crate::Console) -> ! = $main;
            entry(unsafe { $crate::Console::steal() })
        }
    };
}

/// Stop the machine. A panicking cartridge halts rather than running on with
/// corrupt state; the host reports the breakpoint.
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    sys::breakpoint()
}
