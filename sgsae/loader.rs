//! The program slot: the one piece of hardware the supervisor needs.
//!
//! A program is started the way a machine of this era would have started
//! anything: write the address into a buffer, set a length, poke a control
//! register. Execution stops there, the host fetches whatever the address
//! names, and the machine restarts with it in ROM.
//!
//! The address buffer is the only state shared across a program change, and it
//! is written from both sides. The host puts the boot address there before
//! anything runs, so a deep link arrives as data rather than as a special case;
//! the supervisor reads it to decide what to open, and writes it before handing
//! over. [`Status`] says which of those just happened.
//!
//! There is no protocol beyond "here is a string, go and get it". What an
//! address means is the host'"'"'s business entirely.
//!
//! # Register map
//!
//! | offset          | access | register                                     |
//! |-----------------|--------|----------------------------------------------|
//! | `0x00`          | w      | `CTRL` — write [`Loader::LOAD`] to request    |
//! | `0x01`          | r      | `STATUS` — see [`Status`]                     |
//! | `0x02`          | rw     | `LENGTH` (u16) — bytes of `URL` that are meaningful |
//! | `0x10`..`0x110` | rw     | `URL` — the address, as bytes                 |
//!
//! Writing `CTRL` latches whatever is in the buffer. The host reads it with
//! [`Loader::take_request`], which is also what tells it to stop running the
//! current cartridge.

use crate::memory::Device;

/// Bytes of URL the buffer holds.
pub const URL_CAPACITY: usize = 256;
/// Bytes of address space the device occupies.
pub const SIZE: u32 = URL + URL_CAPACITY as u32;

const CTRL: u32 = 0x00;
const STATUS: u32 = 0x01;
const LENGTH: u32 = 0x02;
const LENGTH_HI: u32 = 0x03;
const URL: u32 = 0x10;
const URL_END: u32 = URL + URL_CAPACITY as u32;

/// What the slot is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Status {
    /// Nothing outstanding.
    #[default]
    Idle = 0,
    /// A load was requested and the host has not picked it up yet.
    Pending = 1,
    /// The last load failed. The host sets this; the program that asked is
    /// gone, so it is really for whatever runs next.
    Failed = 2,
    /// A program stopped and the supervisor is running again. The address
    /// buffer still holds what that program was, so a menu can put its cursor
    /// back where it was.
    Returned = 3,
}

/// The cartridge slot.
#[derive(Debug, Clone)]
pub struct Loader {
    url: [u8; URL_CAPACITY],
    length: u16,
    status: Status,
    request: Option<String>,
}

impl Loader {
    /// Value written to `CTRL` to request a load.
    pub const LOAD: u8 = 1;

    pub fn new() -> Loader {
        Loader { url: [0; URL_CAPACITY], length: 0, status: Status::Idle, request: None }
    }

    /// Take the pending request, if the guest made one.
    ///
    /// The host calls this once per frame. A `Some` means: stop running this
    /// cartridge and go and get that one.
    pub fn take_request(&mut self) -> Option<String> {
        let url = self.request.take()?;
        self.status = Status::Idle;
        Some(url)
    }

    /// Tell the supervisor how it was entered.
    pub fn set_status(&mut self, status: Status) {
        self.status = status;
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// Put an address in the buffer from the host side.
    ///
    /// This is how a deep link reaches the supervisor: the page pulls it out
    /// of its own address bar before the machine starts, and the supervisor
    /// finds it already there. Longer addresses are truncated.
    pub fn set_url(&mut self, url: &str) {
        let bytes = url.as_bytes();
        let length = bytes.len().min(URL_CAPACITY);
        self.url[..length].copy_from_slice(&bytes[..length]);
        self.url[length..].fill(0);
        self.length = length as u16;
    }

    /// The URL currently in the buffer, however far the guest has got.
    pub fn url(&self) -> String {
        let length = usize::from(self.length).min(URL_CAPACITY);
        String::from_utf8_lossy(&self.url[..length]).into_owned()
    }

    fn read_byte(&self, offset: u32) -> u8 {
        match offset {
            STATUS => self.status as u8,
            LENGTH => self.length as u8,
            LENGTH_HI => (self.length >> 8) as u8,
            _ if (URL..URL_END).contains(&offset) => self.url[(offset - URL) as usize],
            _ => 0,
        }
    }

    fn write_byte(&mut self, offset: u32, value: u8) {
        match offset {
            CTRL => {
                if value == Loader::LOAD {
                    // Latch a copy: the guest is about to be replaced, and the
                    // host may not look until after that has happened.
                    self.request = Some(self.url());
                    self.status = Status::Pending;
                }
            }
            LENGTH => self.length = (self.length & 0xFF00) | u16::from(value),
            LENGTH_HI => self.length = (self.length & 0x00FF) | (u16::from(value) << 8),
            _ if (URL..URL_END).contains(&offset) => {
                self.url[(offset - URL) as usize] = value;
            }
            _ => {}
        }
    }
}

impl Device for Loader {
    fn size(&self) -> u32 {
        SIZE
    }
    fn read_byte(&mut self, offset: u32) -> u8 {
        Loader::read_byte(self, offset)
    }
    fn write_byte(&mut self, offset: u32, value: u8) {
        Loader::write_byte(self, offset, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Width;

    /// Write `url` into the buffer and set the length, as a guest would.
    fn stage(loader: &mut Loader, url: &str) {
        for (i, byte) in url.bytes().enumerate() {
            loader.write(URL + i as u32, Width::Byte, u32::from(byte));
        }
        loader.write(LENGTH, Width::Half, url.len() as u32);
    }

    #[test]
    fn a_url_round_trips_through_the_buffer() {
        let mut loader = Loader::new();
        stage(&mut loader, "/lander.elf");
        assert_eq!(loader.url(), "/lander.elf");
        assert_eq!(loader.read(LENGTH, Width::Half), 11);
        assert_eq!(loader.read(URL, Width::Byte), u32::from(b'/'));
    }

    #[test]
    fn nothing_happens_until_ctrl_is_poked() {
        let mut loader = Loader::new();
        stage(&mut loader, "/io.elf");
        assert_eq!(loader.take_request(), None, "staging alone must not load");
        assert_eq!(loader.read(STATUS, Width::Byte), Status::Idle as u32);

        loader.write(CTRL, Width::Byte, u32::from(Loader::LOAD));
        assert_eq!(loader.read(STATUS, Width::Byte), Status::Pending as u32);
        assert_eq!(loader.take_request().as_deref(), Some("/io.elf"));
    }

    #[test]
    fn a_request_is_taken_only_once() {
        let mut loader = Loader::new();
        stage(&mut loader, "/home.elf");
        loader.write(CTRL, Width::Byte, u32::from(Loader::LOAD));
        assert!(loader.take_request().is_some());
        assert_eq!(loader.take_request(), None);
        assert_eq!(loader.read(STATUS, Width::Byte), Status::Idle as u32);
    }

    #[test]
    fn the_request_is_a_snapshot_of_the_buffer() {
        // The guest is replaced the moment it asks, so what it asked for has to
        // survive anything that happens to the buffer afterwards.
        let mut loader = Loader::new();
        stage(&mut loader, "/first.elf");
        loader.write(CTRL, Width::Byte, u32::from(Loader::LOAD));
        stage(&mut loader, "/second.elf");
        assert_eq!(loader.take_request().as_deref(), Some("/first.elf"));
    }

    #[test]
    fn writing_ctrl_with_anything_else_does_nothing() {
        let mut loader = Loader::new();
        stage(&mut loader, "/x.elf");
        loader.write(CTRL, Width::Byte, 0);
        loader.write(CTRL, Width::Byte, 99);
        assert_eq!(loader.take_request(), None);
    }

    #[test]
    fn a_length_past_the_buffer_is_clamped() {
        let mut loader = Loader::new();
        loader.write(LENGTH, Width::Half, 9999);
        // Must not panic, and must not read past the buffer.
        assert_eq!(loader.url().len(), URL_CAPACITY);
    }

    #[test]
    fn status_is_read_only_and_gaps_read_as_zero() {
        let mut loader = Loader::new();
        loader.write(STATUS, Width::Byte, 3);
        assert_eq!(loader.read(STATUS, Width::Byte), Status::Idle as u32);
        assert_eq!(loader.read(0x08, Width::Word), 0);
    }

    #[test]
    fn a_full_length_url_fits() {
        let mut loader = Loader::new();
        let url: String = core::iter::repeat_n('a', URL_CAPACITY).collect();
        stage(&mut loader, &url);
        loader.write(CTRL, Width::Byte, u32::from(Loader::LOAD));
        assert_eq!(loader.take_request().unwrap().len(), URL_CAPACITY);
    }

    #[test]
    fn invalid_utf8_does_not_panic() {
        let mut loader = Loader::new();
        for i in 0..4 {
            loader.write(URL + i, Width::Byte, 0xFF);
        }
        loader.write(LENGTH, Width::Half, 4);
        loader.write(CTRL, Width::Byte, u32::from(Loader::LOAD));
        assert!(loader.take_request().is_some());
    }

    #[test]
    fn a_failed_load_is_visible_to_whatever_runs_next() {
        let mut loader = Loader::new();
        loader.set_status(Status::Failed);
        assert_eq!(loader.read(STATUS, Width::Byte), Status::Failed as u32);
    }


    #[test]
    fn the_url_buffer_fits_inside_the_device() {
        assert!(URL_END <= SIZE);
    }
}
