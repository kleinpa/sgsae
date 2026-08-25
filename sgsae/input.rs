//! Memory-mapped input: four gamepads and a keyboard.
//!
//! The host pushes state in; the guest polls. Nothing here interprets input —
//! that is the game's job — so the device is a plain window onto whatever the
//! browser, a WebHID gamepad, or a test harness last reported.
//!
//! # Register map
//!
//! | offset   | access | register                                            |
//! |----------|--------|-----------------------------------------------------|
//! | `0x00`   | r      | `PORTS` — bit *n*: gamepad *n* is connected          |
//! | `0x01`   | r      | `MODIFIERS` — shift, ctrl, alt, meta                 |
//!
//! Each gamepad *n* then occupies 16 bytes at `0x10 + n * 16`:
//!
//! | offset   | access | register                                             |
//! |----------|--------|------------------------------------------------------|
//! | `+0x00`  | r      | `BUTTONS` (u16) — held, see [`Button`]               |
//! | `+0x02`  | r      | `PRESSED` (u16) — rising edges, cleared by a halfword read |
//! | `+0x04`  | r      | `LEFT_X` / `LEFT_Y` (i8) — left stick, -127..=127    |
//! | `+0x06`  | r      | `RIGHT_X` / `RIGHT_Y` (i8) — right stick             |
//! | `+0x08`  | r      | `TRIGGER_L` / `TRIGGER_R` (u8) — analog, 0..=255     |
//!
//! And the keyboard sits above them:
//!
//! | offset          | access | register                                      |
//! |-----------------|--------|-----------------------------------------------|
//! | `0x80`..`0xA0`  | r      | `KEYS_HELD` — 256-bit map, one bit per key    |
//! | `0xA0`..`0xC0`  | r      | `KEYS_PRESSED` — rising edges, cleared on read |
//!
//! Edge registers latch, so a key tapped and released between two polls is
//! still observed exactly once. That is why device reads take `&mut self`.
//! `PRESSED` is taken whole by a halfword read; a byte read only peeks at it.

use crate::memory::{read_le, Device, Width};

/// Gamepad ports.
pub const PORTS: usize = 4;
/// Keys the keyboard bitmap can represent.
pub const KEYS: usize = 256;
/// Bytes in a key bitmap.
pub const KEY_BYTES: usize = KEYS / 8;
/// Bytes of address space the device occupies.
pub const SIZE: u32 = 0x100;

const PORTS_REG: u32 = 0x00;
const MODIFIERS_REG: u32 = 0x01;
const PAD_BASE: u32 = 0x10;
const PAD_STRIDE: u32 = 0x10;
const KEYS_HELD: u32 = 0x80;
const KEYS_PRESSED: u32 = 0xA0;
/// Offset of `PRESSED` within a pad's block.
const PRESSED: u32 = 0x02;

/// A gamepad button. Values are bit positions in `BUTTONS`.
///
/// The layout is a modern pad rather than a period-accurate one: a game that
/// only wants a d-pad and two buttons simply ignores the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    Up = 0,
    Down = 1,
    Left = 2,
    Right = 3,
    A = 4,
    B = 5,
    X = 6,
    Y = 7,
    L1 = 8,
    R1 = 9,
    L2 = 10,
    R2 = 11,
    Select = 12,
    Start = 13,
    L3 = 14,
    R3 = 15,
}

impl Button {
    /// This button's bit in `BUTTONS`.
    pub const fn mask(self) -> u16 {
        1 << (self as u8)
    }

    /// Every button, in bit order.
    pub const ALL: [Button; 16] = [
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
        Button::A,
        Button::B,
        Button::X,
        Button::Y,
        Button::L1,
        Button::R1,
        Button::L2,
        Button::R2,
        Button::Select,
        Button::Start,
        Button::L3,
        Button::R3,
    ];
}

/// Keyboard modifier bits, in `MODIFIERS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Modifier {
    Shift = 0,
    Ctrl = 1,
    Alt = 2,
    Meta = 3,
}

impl Modifier {
    pub const fn mask(self) -> u8 {
        1 << (self as u8)
    }
}

/// Everything one gamepad reports.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pad {
    /// Buttons currently held, as [`Button`] bits.
    pub buttons: u16,
    /// Left stick, -127..=127 on each axis. Positive is right and down.
    pub left: (i8, i8),
    /// Right stick.
    pub right: (i8, i8),
    /// Analog triggers, 0..=255.
    pub triggers: (u8, u8),
    /// Whether anything is plugged into this port.
    pub connected: bool,
}

impl Pad {
    pub fn is_held(&self, button: Button) -> bool {
        self.buttons & button.mask() != 0
    }

    /// Set or clear one button.
    pub fn set(&mut self, button: Button, held: bool) {
        if held {
            self.buttons |= button.mask();
        } else {
            self.buttons &= !button.mask();
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Port {
    pad: Pad,
    /// Buttons that went down since the guest last read `PRESSED`.
    edges: u16,
}

/// The input device.
#[derive(Debug, Clone, Default)]
pub struct Input {
    ports: [Port; PORTS],
    held: [u8; KEY_BYTES],
    edges: [u8; KEY_BYTES],
    modifiers: u8,
}

impl Input {
    // -- host side -----------------------------------------------------------

    /// Replace a gamepad's state, latching any newly pressed buttons.
    ///
    /// Out-of-range ports are ignored rather than panicking: a host enumerating
    /// however many pads a browser reports should not have to bounds-check.
    pub fn set_pad(&mut self, port: usize, pad: Pad) {
        let Some(slot) = self.ports.get_mut(port) else {
            return;
        };
        slot.edges |= pad.buttons & !slot.pad.buttons;
        slot.pad = pad;
    }

    /// Set or clear one key, latching a rising edge.
    ///
    /// `key` is whatever code the host and the guest have agreed on; the SDK's
    /// `Key` enum is the one this project uses.
    pub fn set_key(&mut self, key: u8, down: bool) {
        let (byte, bit) = (usize::from(key) / 8, key % 8);
        let mask = 1u8 << bit;
        if down {
            if self.held[byte] & mask == 0 {
                self.edges[byte] |= mask;
            }
            self.held[byte] |= mask;
        } else {
            self.held[byte] &= !mask;
        }
    }

    /// Replace the whole keyboard at once, latching every rising edge.
    pub fn set_keys(&mut self, held: &[u8; KEY_BYTES]) {
        for (i, byte) in held.iter().enumerate() {
            self.edges[i] |= byte & !self.held[i];
            self.held[i] = *byte;
        }
    }

    /// Adopt another device's held state without latching any edges.
    ///
    /// Used when one cartridge replaces another. A key or button that was
    /// already down when the new program started was not pressed *by* it, and
    /// must not read as a fresh press — otherwise holding a button to leave one
    /// program immediately activates something in the next. Anything genuinely
    /// pressed afterwards latches as normal.
    pub fn adopt(&mut self, previous: &Input) {
        self.held = previous.held;
        self.modifiers = previous.modifiers;
        for (slot, old) in self.ports.iter_mut().zip(previous.ports.iter()) {
            // The pad's state, but deliberately not its edge latch.
            slot.pad = old.pad;
        }
    }

    /// Latch presses the host observed but that are no longer held.
    ///
    /// A host that polls state once a frame cannot see a key that went down
    /// and back up in between — by the time it looks, nothing is held and
    /// there is no edge to infer. An event-driven host records those taps and
    /// reports them here, so the guest still sees each one exactly once.
    pub fn latch_keys(&mut self, pressed: &[u8; KEY_BYTES]) {
        for (i, byte) in pressed.iter().enumerate() {
            self.edges[i] |= byte;
        }
    }

    pub fn is_key_held(&self, key: u8) -> bool {
        self.held[usize::from(key) / 8] & (1 << (key % 8)) != 0
    }

    pub fn set_modifiers(&mut self, modifiers: u8) {
        self.modifiers = modifiers;
    }

    pub fn modifiers(&self) -> u8 {
        self.modifiers
    }

    /// Bit *n* set if gamepad *n* is connected.
    pub fn connected(&self) -> u8 {
        let mut bits = 0;
        for (i, port) in self.ports.iter().enumerate() {
            if port.pad.connected {
                bits |= 1 << i;
            }
        }
        bits
    }

    // -- guest side ----------------------------------------------------------

    /// The pad block `offset` falls in, and the register within it.
    fn pad_at(offset: u32) -> Option<(usize, u32)> {
        let relative = offset.checked_sub(PAD_BASE)?;
        let port = (relative / PAD_STRIDE) as usize;
        (port < PORTS).then_some((port, relative % PAD_STRIDE))
    }

    fn read_byte(&mut self, offset: u32) -> u8 {
        match offset {
            PORTS_REG => self.connected(),
            MODIFIERS_REG => self.modifiers,
            _ if (KEYS_HELD..KEYS_HELD + KEY_BYTES as u32).contains(&offset) => {
                self.held[(offset - KEYS_HELD) as usize]
            }
            _ if (KEYS_PRESSED..KEYS_PRESSED + KEY_BYTES as u32).contains(&offset) => {
                std::mem::take(&mut self.edges[(offset - KEYS_PRESSED) as usize])
            }
            _ => {
                let Some((port, register)) = Self::pad_at(offset) else { return 0 };
                let port = &self.ports[port];
                match register {
                    0x00 => port.pad.buttons as u8,
                    0x01 => (port.pad.buttons >> 8) as u8,
                    // A byte read peeks; only a halfword read takes the latch.
                    0x02 => port.edges as u8,
                    0x03 => (port.edges >> 8) as u8,
                    0x04 => port.pad.left.0 as u8,
                    0x05 => port.pad.left.1 as u8,
                    0x06 => port.pad.right.0 as u8,
                    0x07 => port.pad.right.1 as u8,
                    0x08 => port.pad.triggers.0,
                    0x09 => port.pad.triggers.1,
                    _ => 0,
                }
            }
        }
    }
}

impl Device for Input {
    fn size(&self) -> u32 {
        SIZE
    }

    fn read_byte(&mut self, offset: u32) -> u8 {
        Input::read_byte(self, offset)
    }

    fn write_byte(&mut self, _: u32, _: u8) {}

    fn read(&mut self, offset: u32, width: Width) -> u32 {
        // PRESSED is a latch: a halfword read takes the whole thing at once,
        // so a button above bit 7 cannot be lost between two byte reads.
        if let (Width::Half, Some((port, PRESSED))) = (width, Self::pad_at(offset)) {
            return u32::from(std::mem::take(&mut self.ports[port].edges));
        }
        read_le(width, |i| self.read_byte(offset + i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad_at(port: u32, register: u32) -> u32 {
        PAD_BASE + port * PAD_STRIDE + register
    }

    #[test]
    fn a_pad_reports_held_buttons() {
        let mut input = Input::default();
        let mut pad = Pad { connected: true, ..Pad::default() };
        pad.set(Button::A, true);
        pad.set(Button::Start, true);
        input.set_pad(0, pad);

        let held = input.read(pad_at(0, 0), Width::Half) as u16;
        assert_eq!(held, Button::A.mask() | Button::Start.mask());
    }

    #[test]
    fn ports_are_independent() {
        let mut input = Input::default();
        for port in 0..PORTS {
            let mut pad = Pad { connected: true, ..Pad::default() };
            pad.set(Button::ALL[port], true);
            input.set_pad(port, pad);
        }
        for port in 0..PORTS {
            let held = input.read(pad_at(port as u32, 0), Width::Half) as u16;
            assert_eq!(held, Button::ALL[port].mask(), "port {port}");
        }
        assert_eq!(input.read(PORTS_REG, Width::Byte), 0b1111);
    }

    #[test]
    fn an_unconnected_port_reads_as_nothing_held() {
        let mut input = Input::default();
        input.set_pad(1, Pad { connected: true, buttons: Button::A.mask(), ..Pad::default() });
        assert_eq!(input.read(PORTS_REG, Width::Byte), 0b0010);
        assert_eq!(input.read(pad_at(0, 0), Width::Half), 0);
    }

    #[test]
    fn setting_a_port_that_does_not_exist_is_ignored() {
        let mut input = Input::default();
        input.set_pad(99, Pad { connected: true, ..Pad::default() });
        assert_eq!(input.connected(), 0);
    }

    #[test]
    fn an_edge_on_a_high_button_survives_a_16_bit_read() {
        // PRESSED is read as two bytes. If the low read cleared the latch
        // outright, everything above bit 7 — Select, Start, the stick clicks —
        // would never report a press at all.
        let mut input = Input::default();
        for button in [Button::Start, Button::Select, Button::R3, Button::L2] {
            input.set_pad(0, Pad { buttons: button.mask(), connected: true, ..Pad::default() });
            let pressed = input.read(pad_at(0, 2), Width::Half) as u16;
            assert_eq!(pressed, button.mask(), "{button:?} did not survive the read");
            input.set_pad(0, Pad { connected: true, ..Pad::default() });
            assert_eq!(input.read(pad_at(0, 2), Width::Half), 0);
        }
    }

    #[test]
    fn button_edges_latch_and_clear_on_read() {
        let mut input = Input::default();
        // A tap that begins and ends between two polls is still seen.
        input.set_pad(0, Pad { buttons: Button::B.mask(), connected: true, ..Pad::default() });
        input.set_pad(0, Pad { buttons: 0, connected: true, ..Pad::default() });

        assert_eq!(input.read(pad_at(0, 0), Width::Half), 0, "not held any more");
        assert_eq!(input.read(pad_at(0, 2), Width::Half) as u16, Button::B.mask());
        assert_eq!(input.read(pad_at(0, 2), Width::Half), 0, "edges clear on read");

        // Holding across polls latches one edge, not one per poll.
        let held = Pad { buttons: Button::X.mask(), connected: true, ..Pad::default() };
        input.set_pad(0, held);
        assert_eq!(input.read(pad_at(0, 2), Width::Half) as u16, Button::X.mask());
        input.set_pad(0, held);
        assert_eq!(input.read(pad_at(0, 2), Width::Half), 0);
    }

    #[test]
    fn sticks_and_triggers_survive_the_trip() {
        let mut input = Input::default();
        input.set_pad(
            2,
            Pad {
                left: (-127, 64),
                right: (12, -100),
                triggers: (0, 255),
                connected: true,
                ..Pad::default()
            },
        );
        assert_eq!(input.read(pad_at(2, 4), Width::Byte) as u8 as i8, -127);
        assert_eq!(input.read(pad_at(2, 5), Width::Byte) as u8 as i8, 64);
        assert_eq!(input.read(pad_at(2, 6), Width::Byte) as u8 as i8, 12);
        assert_eq!(input.read(pad_at(2, 7), Width::Byte) as u8 as i8, -100);
        assert_eq!(input.read(pad_at(2, 8), Width::Byte), 0);
        assert_eq!(input.read(pad_at(2, 9), Width::Byte), 255);
    }

    // -- keyboard ------------------------------------------------------------

    #[test]
    fn keys_map_to_the_right_bit() {
        let mut input = Input::default();
        for key in [0u8, 1, 7, 8, 63, 64, 200, 255] {
            input.set_key(key, true);
            assert!(input.is_key_held(key), "key {key}");
            let byte = input.read(KEYS_HELD + u32::from(key) / 8, Width::Byte);
            assert_eq!(byte, 1 << (key % 8), "key {key}");
            input.set_key(key, false);
            assert!(!input.is_key_held(key), "key {key} stuck");
        }
    }

    #[test]
    fn every_key_is_addressable_and_independent() {
        let mut input = Input::default();
        for key in 0..=255u8 {
            input.set_key(key, true);
        }
        for byte in 0..KEY_BYTES as u32 {
            assert_eq!(input.read(KEYS_HELD + byte, Width::Byte), 0xFF);
        }
        input.set_key(137, false);
        assert_eq!(input.read(KEYS_HELD + 137 / 8, Width::Byte), 0xFF & !(1 << (137 % 8)));
    }

    #[test]
    fn key_edges_latch_and_clear_on_read() {
        let mut input = Input::default();
        input.set_key(65, true);
        input.set_key(65, false);
        assert_eq!(input.read(KEYS_HELD + 65 / 8, Width::Byte), 0, "released");
        assert_eq!(input.read(KEYS_PRESSED + 65 / 8, Width::Byte), 1 << (65 % 8));
        assert_eq!(input.read(KEYS_PRESSED + 65 / 8, Width::Byte), 0, "cleared");

        // Repeated set_key while already down must not re-latch.
        input.set_key(65, true);
        assert_eq!(input.read(KEYS_PRESSED + 65 / 8, Width::Byte), 1 << (65 % 8));
        input.set_key(65, true);
        assert_eq!(input.read(KEYS_PRESSED + 65 / 8, Width::Byte), 0);
    }

    #[test]
    fn a_key_already_down_when_a_program_starts_is_not_a_fresh_press() {
        // Holding Enter to leave one cartridge must not activate whatever the
        // next one has under Enter.
        let mut leaving = Input::default();
        leaving.set_key(41, true);
        let _ = leaving.read(KEYS_PRESSED + 41 / 8, Width::Byte);

        let mut arriving = Input::default();
        arriving.adopt(&leaving);
        assert!(arriving.is_key_held(41), "the key is still down");
        assert_eq!(
            arriving.read(KEYS_PRESSED + 41 / 8, Width::Byte),
            0,
            "but it was not pressed by this program"
        );

        // Releasing and pressing again is a real press.
        arriving.set_key(41, false);
        arriving.set_key(41, true);
        assert_ne!(arriving.read(KEYS_PRESSED + 41 / 8, Width::Byte), 0);
    }

    #[test]
    fn a_button_already_down_when_a_program_starts_is_not_a_fresh_press() {
        let mut leaving = Input::default();
        leaving.set_pad(0, Pad { buttons: Button::A.mask(), connected: true, ..Pad::default() });
        let _ = leaving.read(pad_at(0, 2), Width::Half);

        let mut arriving = Input::default();
        arriving.adopt(&leaving);
        assert_eq!(arriving.read(pad_at(0, 0), Width::Half) as u16, Button::A.mask());
        assert_eq!(arriving.read(pad_at(0, 2), Width::Half), 0, "no fresh edge");

        // The host keeps reporting it held; still no new edge.
        arriving.set_pad(0, Pad { buttons: Button::A.mask(), connected: true, ..Pad::default() });
        assert_eq!(arriving.read(pad_at(0, 2), Width::Half), 0);
    }

    #[test]
    fn a_tap_between_two_polls_is_still_reported() {
        // The host never saw the key held: it went down and up between frames.
        // Without a way to report that, the press would vanish entirely.
        let mut input = Input::default();
        let mut tapped = [0u8; KEY_BYTES];
        tapped[41 / 8] = 1 << (41 % 8);

        input.set_keys(&[0; KEY_BYTES]);
        input.latch_keys(&tapped);

        assert_eq!(input.read(KEYS_HELD + 41 / 8, Width::Byte), 0, "not held");
        assert_eq!(input.read(KEYS_PRESSED + 41 / 8, Width::Byte), 1 << (41 % 8));
        assert_eq!(input.read(KEYS_PRESSED + 41 / 8, Width::Byte), 0, "cleared");
    }

    #[test]
    fn a_whole_keyboard_can_be_pushed_at_once() {
        let mut input = Input::default();
        let mut map = [0u8; KEY_BYTES];
        map[1] = 0b1010_0000;
        input.set_keys(&map);
        assert_eq!(input.read(KEYS_HELD + 1, Width::Byte), 0b1010_0000);
        assert_eq!(input.read(KEYS_PRESSED + 1, Width::Byte), 0b1010_0000);

        // Releasing everything leaves nothing held and latches no new edges.
        input.set_keys(&[0; KEY_BYTES]);
        assert_eq!(input.read(KEYS_HELD + 1, Width::Byte), 0);
        assert_eq!(input.read(KEYS_PRESSED + 1, Width::Byte), 0);
    }

    #[test]
    fn modifiers_are_readable() {
        let mut input = Input::default();
        input.set_modifiers(Modifier::Shift.mask() | Modifier::Meta.mask());
        assert_eq!(input.read(MODIFIERS_REG, Width::Byte), 0b1001);
    }

    // -- device behaviour ----------------------------------------------------


    #[test]
    fn gaps_read_as_zero_and_writes_are_ignored() {
        let mut input = Input::default();
        assert_eq!(input.read(0x02, Width::Byte), 0);
        assert_eq!(input.read(0xF0, Width::Byte), 0);
        input.write(KEYS_HELD, Width::Byte, 0xFF);
        assert_eq!(input.read(KEYS_HELD, Width::Byte), 0);
    }

    #[test]
    fn a_byte_read_of_pressed_peeks_without_taking_the_latch() {
        let mut input = Input::default();
        input.set_pad(0, Pad { buttons: Button::Start.mask(), connected: true, ..Pad::default() });
        assert_eq!(input.read(pad_at(0, 3), Width::Byte) as u16, Button::Start.mask() >> 8);
        assert_eq!(input.read(pad_at(0, 2), Width::Half) as u16, Button::Start.mask());
        assert_eq!(input.read(pad_at(0, 2), Width::Half), 0);
    }

    #[test]
    fn the_register_map_does_not_overlap() {
        assert!(PAD_BASE + PAD_STRIDE * PORTS as u32 <= KEYS_HELD);
        assert!(KEYS_HELD + KEY_BYTES as u32 <= KEYS_PRESSED);
        assert!(KEYS_PRESSED + KEY_BYTES as u32 <= SIZE);
    }

    #[test]
    fn button_masks_are_distinct_and_in_bit_order() {
        for (i, button) in Button::ALL.iter().enumerate() {
            assert_eq!(button.mask(), 1 << i);
        }
    }
}
