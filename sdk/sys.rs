//! Talking to the host.

use crate::map;

/// Audio samples per second.
pub const SAMPLE_RATE: u32 = 48_000;
/// Audio samples in one frame, at sixty frames a second.
pub const SAMPLES_PER_FRAME: u32 = SAMPLE_RATE / 60;

/// Yield to the host, which renders one frame of picture and sound.
///
/// A game's main loop is built around this: everything written to the video
/// and audio registers since the last call is what the frame will contain.
#[inline]
pub fn wait_frame() {
    wait_samples(SAMPLES_PER_FRAME)
}

/// Yield to the host for `samples` samples of sound — at most a frame.
///
/// The picture is only presented on frame boundaries, so this is for
/// listening: a sequencer that waits a few milliseconds at a time hears a
/// note played live within a few milliseconds, instead of within a frame.
#[inline]
pub fn wait_samples(samples: u32) {
    // ECALL traps to the host, which services it and steps over it. `a0`
    // says how long to wait for.
    unsafe { core::arch::asm!("ecall", in("a0") samples, options(nostack, preserves_flags)) }
}

/// Stop, handing the machine back to whatever started this program.
///
/// One instruction, and no API to link against: that is the whole exit
/// protocol. A program that panics ends the same way, so a broken cartridge
/// returns to the menu rather than taking the console down with it.
#[inline]
pub fn exit() -> ! {
    breakpoint()
}

/// Halt. With a supervisor present this returns to it; without one it stops
/// the machine.
#[inline]
pub fn breakpoint() -> ! {
    loop {
        unsafe { core::arch::asm!("ebreak", options(nostack, preserves_flags)) }
    }
}

/// Write one byte to the host's console.
#[inline]
pub fn put(byte: u8) {
    unsafe { core::ptr::write_volatile(map::CONSOLE_OUT as *mut u8, byte) }
}

/// Write text to the host's console.
pub fn print(text: &str) {
    for byte in text.as_bytes() {
        put(*byte);
    }
}

/// Write text and a newline.
pub fn println(text: &str) {
    print(text);
    put(b'\n');
}
