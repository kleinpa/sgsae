//! Small reference [`Device`] implementations.
//!
//! [`ConsoleOut`] carries bytes *out* of the guest to whatever is displaying
//! them — a `<pre>` on the page, a terminal. Input comes the other way and
//! lives in [`crate::input`].
//!
//! ```
//! use std::cell::RefCell;
//! use std::rc::Rc;
//! use sgsae::devices::ConsoleOut;
//! use sgsae::memory::Bus;
//!
//! let console = Rc::new(RefCell::new(ConsoleOut::default()));
//! let mut bus = Bus::new();
//! bus.map_device(0x0400_0000, console.clone()).unwrap();
//!
//! let text = console.borrow_mut().take_string();   // guest -> host
//! assert!(text.is_empty());
//! ```

use crate::memory::Device;

/// Bytes of address space the device occupies.
pub const SIZE: u32 = 4;

/// A byte sink the guest writes text through.
///
/// | offset | access | meaning                                   |
/// |--------|--------|-------------------------------------------|
/// | `0x00` | write  | `DATA` — append the low byte to the buffer |
///
/// The host drains the buffer with [`ConsoleOut::take`] or
/// [`ConsoleOut::take_string`], typically once per frame.
#[derive(Debug, Default, Clone)]
pub struct ConsoleOut {
    buffer: Vec<u8>,
}

impl ConsoleOut {
    pub const DATA: u32 = 0x00;

    /// Take everything buffered so far, leaving the buffer empty.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer)
    }

    /// [`ConsoleOut::take`], decoded lossily as UTF-8.
    pub fn take_string(&mut self) -> String {
        String::from_utf8_lossy(&self.take()).into_owned()
    }
}

impl Device for ConsoleOut {
    fn size(&self) -> u32 {
        SIZE
    }
    fn read_byte(&mut self, _: u32) -> u8 {
        0
    }
    fn write_byte(&mut self, offset: u32, value: u8) {
        if offset == Self::DATA {
            self.buffer.push(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{Bus, Width};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn console_collects_bytes_the_guest_writes() {
        let mut c = ConsoleOut::default();
        for byte in b"hi\n" {
            c.write(ConsoleOut::DATA, Width::Byte, u32::from(*byte));
        }
        assert_eq!(c.take_string(), "hi\n");
        assert_eq!(c.take_string(), "");
    }

    #[test]
    fn console_takes_only_the_low_byte() {
        let mut c = ConsoleOut::default();
        c.write(ConsoleOut::DATA, Width::Word, 0xDEAD_BE41);
        assert_eq!(c.take_string(), "A");
    }

    #[test]
    fn host_keeps_a_handle_through_the_bus() {
        let console = Rc::new(RefCell::new(ConsoleOut::default()));
        let mut bus = Bus::new();
        bus.map_device(0x0400_0000, console.clone()).unwrap();

        // Guest writes out; the host drains it.
        bus.write(0x0400_0000, Width::Byte, u32::from(b'o')).unwrap();
        bus.write(0x0400_0000, Width::Byte, u32::from(b'k')).unwrap();
        assert_eq!(console.borrow_mut().take_string(), "ok");
    }
}
