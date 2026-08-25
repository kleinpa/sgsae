//! A MIDI input port.
//!
//! Channel messages — notes, controllers, pitch bend — queue up in a FIFO the
//! guest drains. Realtime messages do not: a clock arrives twenty-four times a
//! beat and would swamp any queue, so ticks are counted in a register and the
//! transport state is a flag. A sequencer reads the tick delta once a frame and
//! advances by however many arrived.
//!
//! # Register map
//!
//! | offset | access | register                                              |
//! |--------|--------|-------------------------------------------------------|
//! | `0x00` | r      | `STATUS` — bit 0: a port is open, bit 1: clock running |
//! | `0x02` | r      | `PENDING` — channel messages waiting                   |
//! | `0x03` | r      | `DROPPED` — messages lost to overflow, clears on read  |
//! | `0x04` | r      | `CLOCK` (u32) — clock pulses since power-on            |
//! | `0x08` | r      | `MESSAGE` (u32) — pop one: `status \| d1<<8 \| d2<<16` |
//! | `0x0C` | r      | `TICKS` — pulses since this register was last read     |
//!
//! A word read of `MESSAGE` pops it; a byte read only peeks. An empty queue
//! reads as zero, which is not a valid message because every status byte has
//! its high bit set. Reading `TICKS` clears the count, so a guest that polls
//! once a frame sees each pulse exactly once.

use std::collections::VecDeque;

use crate::memory::{read_le, Device, Width};

/// Channel messages the queue holds before it starts dropping them.
pub const QUEUE_DEPTH: usize = 64;
/// Bytes of address space the device occupies.
pub const SIZE: u32 = 0x10;

const STATUS: u32 = 0x00;
const PENDING: u32 = 0x02;
const DROPPED: u32 = 0x03;
const CLOCK: u32 = 0x04;
const MESSAGE: u32 = 0x08;
const TICKS: u32 = 0x0C;

/// `STATUS` bit set while an input port is open.
pub const CONNECTED: u8 = 0b01;
/// `STATUS` bit set between Start/Continue and Stop.
pub const RUNNING: u8 = 0b10;

// System realtime status bytes.
const CLOCK_PULSE: u8 = 0xF8;
const START: u8 = 0xFA;
const CONTINUE: u8 = 0xFB;
const STOP: u8 = 0xFC;

/// Clock pulses in one quarter note, as the MIDI spec fixes it.
pub const PULSES_PER_BEAT: u32 = 24;

/// A MIDI input port.
#[derive(Debug, Clone, Default)]
pub struct Midi {
    queue: VecDeque<u32>,
    connected: bool,
    running: bool,
    clock: u32,
    ticks: u8,
    dropped: u8,
}

impl Midi {
    // -- host side -----------------------------------------------------------

    /// Report whether an input port is open.
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn clock(&self) -> u32 {
        self.clock
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Deliver one message, exactly as it came off the wire.
    ///
    /// Realtime bytes are acted on rather than queued. Everything else is
    /// queued; if the guest is not draining fast enough the oldest is dropped,
    /// because in a live performance the newest note matters more than the one
    /// from a second ago.
    pub fn receive(&mut self, status: u8, data1: u8, data2: u8) {
        match status {
            CLOCK_PULSE => {
                self.clock = self.clock.wrapping_add(1);
                self.ticks = self.ticks.saturating_add(1);
                return;
            }
            START => {
                // Start rewinds; Continue does not.
                self.running = true;
                self.clock = 0;
                self.ticks = 0;
            }
            CONTINUE => self.running = true,
            STOP => self.running = false,
            _ => {}
        }

        if self.queue.len() >= QUEUE_DEPTH {
            self.queue.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        let packed = u32::from(status) | (u32::from(data1) << 8) | (u32::from(data2) << 16);
        self.queue.push_back(packed);
    }

    /// Deliver a raw MIDI packet, as a browser hands one over.
    ///
    /// Short packets are padded; anything longer than three bytes — a system
    /// exclusive dump, say — is ignored rather than truncated into something
    /// that would look like a different message.
    pub fn receive_packet(&mut self, bytes: &[u8]) {
        match bytes.len() {
            1 => self.receive(bytes[0], 0, 0),
            2 => self.receive(bytes[0], bytes[1], 0),
            3 => self.receive(bytes[0], bytes[1], bytes[2]),
            _ => {}
        }
    }

    // -- guest side ----------------------------------------------------------

    fn status_bits(&self) -> u8 {
        let mut bits = 0;
        if self.connected {
            bits |= CONNECTED;
        }
        if self.running {
            bits |= RUNNING;
        }
        bits
    }

    fn read_byte(&mut self, offset: u32) -> u8 {
        match offset {
            STATUS => self.status_bits(),
            PENDING => self.queue.len().min(255) as u8,
            DROPPED => std::mem::take(&mut self.dropped),
            _ if (CLOCK..CLOCK + 4).contains(&offset) => {
                self.clock.to_le_bytes()[(offset - CLOCK) as usize]
            }
            _ if (MESSAGE..MESSAGE + 4).contains(&offset) => {
                // A peek: only a word read pops.
                self.queue.front().copied().unwrap_or(0).to_le_bytes()[(offset - MESSAGE) as usize]
            }
            TICKS => std::mem::take(&mut self.ticks),
            _ => 0,
        }
    }
}

impl Device for Midi {
    fn size(&self) -> u32 {
        SIZE
    }

    fn read_byte(&mut self, offset: u32) -> u8 {
        Midi::read_byte(self, offset)
    }

    fn write_byte(&mut self, _: u32, _: u8) {}

    fn read(&mut self, offset: u32, width: Width) -> u32 {
        if (offset, width) == (MESSAGE, Width::Word) {
            return self.queue.pop_front().unwrap_or(0);
        }
        read_le(width, |i| self.read_byte(offset + i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE_ON: u8 = 0x90;
    const NOTE_OFF: u8 = 0x80;

    fn pop(midi: &mut Midi) -> u32 {
        midi.read(MESSAGE, Width::Word)
    }

    #[test]
    fn a_note_survives_the_trip_intact() {
        let mut midi = Midi::default();
        midi.receive(NOTE_ON, 60, 100);
        assert_eq!(midi.read(PENDING, Width::Byte), 1);

        let message = pop(&mut midi);
        assert_eq!(message & 0xFF, u32::from(NOTE_ON));
        assert_eq!((message >> 8) & 0xFF, 60);
        assert_eq!((message >> 16) & 0xFF, 100);
    }

    #[test]
    fn an_empty_queue_reads_as_zero() {
        // Zero cannot be mistaken for a message: every status byte has its top
        // bit set, so a real one is never zero.
        let mut midi = Midi::default();
        assert_eq!(pop(&mut midi), 0);
        midi.receive(NOTE_ON, 60, 100);
        assert_ne!(pop(&mut midi), 0);
        assert_eq!(pop(&mut midi), 0);
    }

    #[test]
    fn messages_come_back_in_the_order_they_arrived() {
        let mut midi = Midi::default();
        for note in 60u8..64 {
            midi.receive(NOTE_ON, note, 64);
        }
        for note in 60u8..64 {
            assert_eq!((pop(&mut midi) >> 8) & 0xFF, u32::from(note));
        }
    }

    #[test]
    fn a_full_queue_drops_the_oldest_and_says_so() {
        let mut midi = Midi::default();
        for i in 0..QUEUE_DEPTH + 4 {
            midi.receive(NOTE_ON, (i % 128) as u8, 64);
        }
        assert_eq!(midi.pending(), QUEUE_DEPTH);
        assert_eq!(midi.read(DROPPED, Width::Byte), 4);
        assert_eq!(midi.read(DROPPED, Width::Byte), 0, "clears on read");
        // The survivors are the most recent ones.
        assert_eq!((pop(&mut midi) >> 8) & 0xFF, 4);
    }

    // -- realtime ------------------------------------------------------------

    #[test]
    fn clock_pulses_are_counted_not_queued() {
        let mut midi = Midi::default();
        for _ in 0..PULSES_PER_BEAT {
            midi.receive(CLOCK_PULSE, 0, 0);
        }
        assert_eq!(midi.pending(), 0, "the clock must not fill the queue");
        assert_eq!(midi.read(CLOCK, Width::Word), PULSES_PER_BEAT);
    }

    #[test]
    fn ticks_report_a_delta_and_clear_on_read() {
        let mut midi = Midi::default();
        for _ in 0..6 {
            midi.receive(CLOCK_PULSE, 0, 0);
        }
        assert_eq!(midi.read(TICKS, Width::Byte), 6);
        assert_eq!(midi.read(TICKS, Width::Byte), 0);

        midi.receive(CLOCK_PULSE, 0, 0);
        assert_eq!(midi.read(TICKS, Width::Byte), 1);
        // The absolute count keeps going regardless.
        assert_eq!(midi.read(CLOCK, Width::Word), 7);
    }

    #[test]
    fn transport_messages_move_the_running_flag() {
        let mut midi = Midi::default();
        assert_eq!(midi.read(STATUS, Width::Byte) & u32::from(RUNNING), 0);

        midi.receive(START, 0, 0);
        assert_ne!(midi.read(STATUS, Width::Byte) & u32::from(RUNNING), 0);
        midi.receive(STOP, 0, 0);
        assert_eq!(midi.read(STATUS, Width::Byte) & u32::from(RUNNING), 0);
        midi.receive(CONTINUE, 0, 0);
        assert!(midi.is_running());

        // They are queued as well, so a guest can react to the transport.
        assert_eq!(pop(&mut midi) & 0xFF, u32::from(START));
    }

    #[test]
    fn start_rewinds_the_clock_but_continue_does_not() {
        let mut midi = Midi::default();
        for _ in 0..10 {
            midi.receive(CLOCK_PULSE, 0, 0);
        }
        midi.receive(STOP, 0, 0);
        midi.receive(CONTINUE, 0, 0);
        assert_eq!(midi.clock(), 10, "continue picks up where it left off");

        midi.receive(START, 0, 0);
        assert_eq!(midi.clock(), 0, "start goes back to the top");
    }

    #[test]
    fn connection_state_is_reported() {
        let mut midi = Midi::default();
        assert_eq!(midi.read(STATUS, Width::Byte) & u32::from(CONNECTED), 0);
        midi.set_connected(true);
        assert_ne!(midi.read(STATUS, Width::Byte) & u32::from(CONNECTED), 0);
    }

    // -- packets -------------------------------------------------------------

    #[test]
    fn short_packets_are_padded_and_long_ones_ignored() {
        let mut midi = Midi::default();
        midi.receive_packet(&[0xF8]);
        assert_eq!(midi.clock(), 1);

        midi.receive_packet(&[0xC0, 5]); // program change: two bytes
        assert_eq!(pop(&mut midi) & 0xFFFF, 0x05C0);

        // A sysex dump must not be truncated into a bogus channel message.
        midi.receive_packet(&[0xF0, 0x7E, 0x00, 0x06, 0x01, 0xF7]);
        assert_eq!(midi.pending(), 0);
        midi.receive_packet(&[]);
        assert_eq!(midi.pending(), 0);
    }

    #[test]
    fn a_note_off_is_distinguishable_from_a_note_on() {
        let mut midi = Midi::default();
        midi.receive(NOTE_ON | 3, 64, 100);
        midi.receive(NOTE_OFF | 3, 64, 0);
        assert_eq!(pop(&mut midi) & 0xFF, 0x93);
        assert_eq!(pop(&mut midi) & 0xFF, 0x83);
    }

    // -- device behaviour ----------------------------------------------------

    #[test]
    fn a_byte_read_peeks_and_a_word_read_pops() {
        let mut midi = Midi::default();
        midi.receive(NOTE_ON, 60, 100);
        assert_eq!(midi.read(MESSAGE + 1, Width::Byte), 60);
        assert_eq!(midi.pending(), 1, "a byte read must not consume the message");
        assert_eq!(pop(&mut midi) & 0xFF, u32::from(NOTE_ON));
        assert_eq!(midi.pending(), 0);
    }

    #[test]
    fn writes_are_ignored() {
        let mut midi = Midi::default();
        midi.write(STATUS, Width::Byte, 1);
        midi.write(MESSAGE, Width::Word, 0x90);
        assert_eq!(midi.pending(), 0);
        assert_eq!(midi.read(STATUS, Width::Byte), 0);
    }


}
