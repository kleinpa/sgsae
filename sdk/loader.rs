//! Starting other programs.
//!
//! This is the supervisor'"'"'s side of the console. Ordinary programs never touch
//! it: they are started by whatever ran before them, and they end by stopping.
//!
//! [`Loader::run`] never returns, because the caller does not survive it. That
//! is what makes a menu work — opening an entry *is* the load, and the menu is
//! simply gone afterwards. It comes back when the program stops.

use crate::map;

/// Bytes of address the slot will accept.
pub const URL_CAPACITY: usize = 256;

const CTRL: usize = 0x00;
const STATUS: usize = 0x01;
const LENGTH: usize = 0x02;
const URL: usize = 0x10;

const LOAD: u8 = 1;

/// What the slot is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing has happened yet: a fresh boot. If the address buffer holds
    /// something, the host put it there and it is where to go.
    Idle,
    Pending,
    /// The last load did not work.
    Failed,
    /// A program stopped and handed the machine back. The address buffer still
    /// holds what it was, so a menu can put its cursor back.
    Returned,
    Unknown(u8),
}

/// The cartridge slot.
pub struct Loader {
    _private: (),
}

impl Loader {
    pub(crate) const fn new() -> Loader {
        Loader { _private: () }
    }

    fn reg(&self, offset: usize) -> *mut u8 {
        (map::LOADER + offset) as *mut u8
    }

    /// Whether the last load worked, for a cartridge that wants to report it.
    pub fn status(&self) -> Status {
        match unsafe { core::ptr::read_volatile(self.reg(STATUS)) } {
            0 => Status::Idle,
            1 => Status::Pending,
            2 => Status::Failed,
            3 => Status::Returned,
            other => Status::Unknown(other),
        }
    }

    /// Read the address in the buffer into `out`, returning its length.
    ///
    /// On a fresh boot this is whatever the host put there — a deep link.
    /// After a program stops it is the address that program was loaded from.
    pub fn url(&self, out: &mut [u8]) -> usize {
        let length = usize::from(unsafe {
            core::ptr::read_volatile(self.reg(LENGTH) as *const u16)
        })
        .min(URL_CAPACITY)
        .min(out.len());
        for (i, slot) in out[..length].iter_mut().enumerate() {
            *slot = unsafe { core::ptr::read_volatile(self.reg(URL + i)) };
        }
        length
    }

    /// Load the program at `url` and never come back.
    ///
    /// Addresses longer than [`URL_CAPACITY`] are truncated rather than
    /// refused — there is nowhere to report an error to, since the caller is
    /// about to stop existing either way.
    pub fn run(&mut self, url: &str) -> ! {
        let bytes = url.as_bytes();
        let length = bytes.len().min(URL_CAPACITY);
        for (i, byte) in bytes[..length].iter().enumerate() {
            unsafe { core::ptr::write_volatile(self.reg(URL + i), *byte) }
        }
        unsafe {
            core::ptr::write_volatile(self.reg(LENGTH) as *mut u16, length as u16);
            core::ptr::write_volatile(self.reg(CTRL), LOAD);
        }
        // The host stops the machine at the next frame boundary. Yield until it
        // does rather than running on through code that assumes it still owns
        // the hardware.
        loop {
            crate::sys::wait_frame();
        }
    }
}
