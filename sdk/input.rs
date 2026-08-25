//! Gamepads and the keyboard.
//!
//! Four pads, each with a d-pad, six face and shoulder buttons, two analog
//! sticks and two analog triggers; plus a full keyboard. A game that only wants
//! a d-pad and two buttons ignores the rest.
//!
//! Both pads and the keyboard offer *held* and *pressed* state. Held is the
//! current truth; pressed latches rising edges and clears when read, so a tap
//! that starts and ends between two polls is still seen exactly once.

use crate::map;

/// Gamepad ports.
pub const PORTS: u32 = 4;

const PORTS_REG: usize = 0x00;
const MODIFIERS_REG: usize = 0x01;
const PAD_BASE: usize = 0x10;
const PAD_STRIDE: usize = 0x10;
const KEYS_HELD: usize = 0x80;
const KEYS_PRESSED: usize = 0xA0;

/// A gamepad button.
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
    pub const fn mask(self) -> u16 {
        1 << (self as u8)
    }
}

/// A keyboard key.
///
/// Values are this console's own codes, not a PC scancode set. The host maps
/// whatever it has — a browser's `event.code`, a USB HID report — onto these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[rustfmt::skip]
pub enum Key {
    A = 1, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Digit0 = 30, Digit1, Digit2, Digit3, Digit4,
    Digit5, Digit6, Digit7, Digit8, Digit9,
    Escape = 40, Enter, Tab, Space, Backspace, Delete,
    Minus = 50, Equal, LeftBracket, RightBracket, Backslash,
    Semicolon, Quote, Backquote, Comma, Period, Slash,
    Up = 70, Down, Left, Right,
    Home = 80, End, PageUp, PageDown, Insert,
    LeftShift = 90, RightShift, LeftCtrl, RightCtrl,
    LeftAlt, RightAlt, LeftMeta, RightMeta,
    F1 = 100, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
}

impl Key {
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// Keyboard modifiers.
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

/// A snapshot of one gamepad.
///
/// Taken by value, so the state cannot change underneath a frame's worth of
/// logic — and reading it consumes the pad's pressed-button latch exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Gamepad {
    /// Buttons held right now.
    pub held: u16,
    /// Buttons that went down since this pad was last read.
    pub pressed: u16,
    /// Left stick, -127..=127. Positive is right and down.
    pub left: Stick,
    /// Right stick.
    pub right: Stick,
    /// Analog triggers, 0..=255.
    pub triggers: (u8, u8),
    pub connected: bool,
}

/// An analog stick position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stick {
    pub x: i8,
    pub y: i8,
}

impl Stick {
    /// Ignore small deflections, so a stick that does not quite centre does
    /// not creep. Returns -127..=127 with the dead zone removed.
    pub fn with_deadzone(self, threshold: i8) -> Stick {
        let apply = |v: i8| if v.saturating_abs() < threshold { 0 } else { v };
        Stick { x: apply(self.x), y: apply(self.y) }
    }
}

impl Gamepad {
    pub fn is_held(&self, button: Button) -> bool {
        self.held & button.mask() != 0
    }

    /// Whether `button` went down since the last read of this pad.
    pub fn was_pressed(&self, button: Button) -> bool {
        self.pressed & button.mask() != 0
    }

    /// Any of the given buttons held.
    pub fn any_held(&self, buttons: &[Button]) -> bool {
        buttons.iter().any(|b| self.is_held(*b))
    }
}

/// The input hardware.
pub struct Input {
    _private: (),
}

impl Input {
    pub(crate) const fn new() -> Input {
        Input { _private: () }
    }

    fn reg(&self, offset: usize) -> *mut u8 {
        (map::INPUT + offset) as *mut u8
    }

    /// Bit *n* set if gamepad *n* is connected.
    pub fn connected(&self) -> u8 {
        unsafe { core::ptr::read_volatile(self.reg(PORTS_REG)) }
    }

    /// Read a gamepad. Ports at or above [`PORTS`] wrap.
    ///
    /// Takes `&mut self` because it consumes the pad's pressed-button latch:
    /// call it once per frame and keep the result.
    pub fn gamepad(&mut self, port: u32) -> Gamepad {
        let base = PAD_BASE + (port % PORTS) as usize * PAD_STRIDE;
        unsafe {
            let held = core::ptr::read_volatile(self.reg(base) as *const u16);
            let pressed = core::ptr::read_volatile(self.reg(base + 2) as *const u16);
            Gamepad {
                held,
                pressed,
                left: Stick {
                    x: core::ptr::read_volatile(self.reg(base + 4)) as i8,
                    y: core::ptr::read_volatile(self.reg(base + 5)) as i8,
                },
                right: Stick {
                    x: core::ptr::read_volatile(self.reg(base + 6)) as i8,
                    y: core::ptr::read_volatile(self.reg(base + 7)) as i8,
                },
                triggers: (
                    core::ptr::read_volatile(self.reg(base + 8)),
                    core::ptr::read_volatile(self.reg(base + 9)),
                ),
                connected: self.connected() & (1 << (port % PORTS)) != 0,
            }
        }
    }

    /// Is a key down right now?
    pub fn is_key_held(&self, key: Key) -> bool {
        let code = key.code();
        let byte = unsafe { core::ptr::read_volatile(self.reg(KEYS_HELD + usize::from(code) / 8)) };
        byte & (1 << (code % 8)) != 0
    }

    /// Every key currently down, as a 256-bit map.
    pub fn keys_held(&self) -> Keys {
        let mut bits = [0u8; 32];
        for (i, byte) in bits.iter_mut().enumerate() {
            *byte = unsafe { core::ptr::read_volatile(self.reg(KEYS_HELD + i)) };
        }
        Keys(bits)
    }

    /// Every key pressed since the last snapshot, clearing the latches.
    ///
    /// There is deliberately no way to ask about a single key: reading the
    /// hardware clears the latches for eight keys at once, so a second
    /// question in the same frame would silently answer "no". Take one
    /// snapshot and interrogate that.
    pub fn keys_pressed(&mut self) -> Keys {
        let mut bits = [0u8; 32];
        for (i, byte) in bits.iter_mut().enumerate() {
            *byte = unsafe { core::ptr::read_volatile(self.reg(KEYS_PRESSED + i)) };
        }
        Keys(bits)
    }

    pub fn modifiers(&self) -> u8 {
        unsafe { core::ptr::read_volatile(self.reg(MODIFIERS_REG)) }
    }

    pub fn is_modifier_held(&self, modifier: Modifier) -> bool {
        self.modifiers() & modifier.mask() != 0
    }
}

/// A snapshot of the whole keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keys([u8; 32]);

impl Keys {
    pub fn contains(&self, key: Key) -> bool {
        self.contains_code(key.code())
    }

    /// Whether a raw key code is in the snapshot, for code that works in codes
    /// rather than named keys.
    pub fn contains_code(&self, code: u8) -> bool {
        self.0[usize::from(code) / 8] & (1 << (code % 8)) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }

    /// Raw codes of every key in the snapshot.
    pub fn codes(&self) -> impl Iterator<Item = u8> + '_ {
        (0..=255u8).filter(move |c| self.0[usize::from(*c) / 8] & (1 << (c % 8)) != 0)
    }
}
