//! MIDI input.
//!
//! Channel messages arrive in a queue the guest drains; the clock does not.
//! A clock pulse comes twenty-four times a beat, so ticks are counted in a
//! register and [`Midi::take_ticks`] reports how many arrived since the last
//! look. A sequencer polls that once a frame and advances by the difference,
//! which keeps it locked to an external device without needing interrupts.
//!
//! ```no_run
//! # use console::midi::{Message, Midi};
//! # fn f(midi: &mut Midi) {
//! for message in midi.messages() {
//!     if let Message::NoteOn { note, velocity, .. } = message {
//!         if velocity > 0 {
//!             // …play it
//!         }
//!     }
//! }
//! # }
//! ```

use crate::map;

const STATUS: usize = 0x00;
const PENDING: usize = 0x02;
const DROPPED: usize = 0x03;
const CLOCK: usize = 0x04;
const MESSAGE: usize = 0x08;
const TICKS: usize = 0x0C;

const CONNECTED: u8 = 0b01;
const RUNNING: u8 = 0b10;

/// Clock pulses in one quarter note, as the MIDI spec fixes it.
pub const PULSES_PER_BEAT: u32 = 24;
/// Clock pulses in one sixteenth note — one step of a sixteen-step bar.
pub const PULSES_PER_STEP: u32 = PULSES_PER_BEAT / 4;

/// A message from the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// A key went down. A velocity of zero is a note off, which is how most
    /// keyboards actually release a note — handle both.
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    ControlChange { channel: u8, controller: u8, value: u8 },
    ProgramChange { channel: u8, program: u8 },
    /// Centred is zero; the range is -8192..=8191.
    PitchBend { channel: u8, value: i16 },
    /// Rewind and run.
    Start,
    /// Run from where the clock stopped.
    Continue,
    Stop,
    /// Anything this SDK does not model, kept whole so a game can still act on
    /// it rather than having it silently disappear.
    Other { status: u8, data: (u8, u8) },
}

impl Message {
    /// Decode a packed register value, or `None` if the queue was empty.
    ///
    /// Zero is unambiguous: every status byte has its high bit set.
    pub fn decode(packed: u32) -> Option<Message> {
        if packed == 0 {
            return None;
        }
        let status = packed as u8;
        let data1 = (packed >> 8) as u8 & 0x7F;
        let data2 = (packed >> 16) as u8 & 0x7F;
        let channel = status & 0x0F;

        Some(match status & 0xF0 {
            0x80 => Message::NoteOff { channel, note: data1, velocity: data2 },
            0x90 => Message::NoteOn { channel, note: data1, velocity: data2 },
            0xB0 => Message::ControlChange { channel, controller: data1, value: data2 },
            0xC0 => Message::ProgramChange { channel, program: data1 },
            0xE0 => Message::PitchBend {
                channel,
                value: ((i16::from(data2) << 7) | i16::from(data1)) - 8192,
            },
            _ => match status {
                0xFA => Message::Start,
                0xFB => Message::Continue,
                0xFC => Message::Stop,
                _ => Message::Other { status, data: (data1, data2) },
            },
        })
    }

    /// The note this message starts, if it starts one.
    ///
    /// Folds away the note-on-with-zero-velocity convention, which is the one
    /// piece of MIDI trivia every beginner trips over.
    pub fn note_on(&self) -> Option<(u8, u8)> {
        match *self {
            Message::NoteOn { note, velocity, .. } if velocity > 0 => Some((note, velocity)),
            _ => None,
        }
    }

    /// The note this message releases, if it releases one.
    pub fn note_off(&self) -> Option<u8> {
        match *self {
            Message::NoteOff { note, .. } => Some(note),
            Message::NoteOn { note, velocity: 0, .. } => Some(note),
            _ => None,
        }
    }
}

/// The MIDI input port.
pub struct Midi {
    _private: (),
}

impl Midi {
    pub(crate) const fn new() -> Midi {
        Midi { _private: () }
    }

    fn reg(&self, offset: usize) -> *mut u8 {
        (map::MIDI + offset) as *mut u8
    }

    fn status(&self) -> u8 {
        unsafe { core::ptr::read_volatile(self.reg(STATUS)) }
    }

    /// Whether an input port is open.
    pub fn is_connected(&self) -> bool {
        self.status() & CONNECTED != 0
    }

    /// Whether an external clock is running, between Start and Stop.
    pub fn is_running(&self) -> bool {
        self.status() & RUNNING != 0
    }

    /// Clock pulses since the last Start.
    pub fn clock(&self) -> u32 {
        unsafe { core::ptr::read_volatile(self.reg(CLOCK) as *const u32) }
    }

    /// Messages waiting to be read.
    pub fn pending(&self) -> u8 {
        unsafe { core::ptr::read_volatile(self.reg(PENDING)) }
    }

    /// Messages lost because the queue filled up. Clears when read.
    pub fn take_dropped(&mut self) -> u8 {
        unsafe { core::ptr::read_volatile(self.reg(DROPPED)) }
    }

    /// Clock pulses since this was last called.
    ///
    /// The count clears on read, so calling once a frame gives a clean delta
    /// and no pulse is seen twice.
    pub fn take_ticks(&mut self) -> u8 {
        unsafe { core::ptr::read_volatile(self.reg(TICKS)) }
    }

    /// Take the next message, if there is one.
    pub fn poll(&mut self) -> Option<Message> {
        let packed = unsafe { core::ptr::read_volatile(self.reg(MESSAGE) as *const u32) };
        Message::decode(packed)
    }

    /// Drain everything waiting.
    pub fn messages(&mut self) -> Messages<'_> {
        Messages { midi: self }
    }
}

/// Iterator over queued messages, draining as it goes.
pub struct Messages<'a> {
    midi: &'a mut Midi,
}

impl Iterator for Messages<'_> {
    type Item = Message;

    fn next(&mut self) -> Option<Message> {
        self.midi.poll()
    }
}

/// The pitch of a MIDI note, in whole Hz.
///
/// A table rather than a calculation: there is no floating point here, and 128
/// entries cost less than the code to work them out would.
pub const fn note_hz(note: u8) -> u16 {
    NOTE_HZ[(note & 0x7F) as usize]
}

/// The name of a note within its octave, as three characters.
pub const fn note_name(note: u8) -> &'static str {
    match note % 12 {
        0 => "C-",
        1 => "C#",
        2 => "D-",
        3 => "D#",
        4 => "E-",
        5 => "F-",
        6 => "F#",
        7 => "G-",
        8 => "G#",
        9 => "A-",
        10 => "A#",
        _ => "B-",
    }
}

/// The octave a note falls in.
///
/// Scientific pitch notation, where middle C — note 60 — is C4. Some makers
/// number it C3 or C5 instead; this is the convention most software uses.
pub const fn note_octave(note: u8) -> u8 {
    note / 12 - 1
}

#[rustfmt::skip]
const NOTE_HZ: [u16; 128] = [
    8, 9, 9, 10, 10, 11, 12, 12,
    13, 14, 15, 15, 16, 17, 18, 19,
    21, 22, 23, 24, 26, 28, 29, 31,
    33, 35, 37, 39, 41, 44, 46, 49,
    52, 55, 58, 62, 65, 69, 73, 78,
    82, 87, 92, 98, 104, 110, 117, 123,
    131, 139, 147, 156, 165, 175, 185, 196,
    208, 220, 233, 247, 262, 277, 294, 311,
    330, 349, 370, 392, 415, 440, 466, 494,
    523, 554, 587, 622, 659, 698, 740, 784,
    831, 880, 932, 988, 1047, 1109, 1175, 1245,
    1319, 1397, 1480, 1568, 1661, 1760, 1865, 1976,
    2093, 2217, 2349, 2489, 2637, 2794, 2960, 3136,
    3322, 3520, 3729, 3951, 4186, 4435, 4699, 4978,
    5274, 5588, 5920, 6272, 6645, 7040, 7459, 7902,
    8372, 8870, 9397, 9956, 10548, 11175, 11840, 12544,
];
