//! Input test.
//!
//! Shows everything the console can currently see — every gamepad's buttons,
//! sticks and triggers, every key held on the keyboard, and whatever arrives
//! over MIDI — and sounds a DTMF pair whenever a pad or key is pressed, or a
//! MIDI note at its own pitch when one arrives.
//!
//! The MIDI panel is a monitor rather than an interpretation: it prints the
//! raw status and data bytes of the last few messages. That is the point. If a
//! controller sends something under a button and you do not know what, this is
//! where you find out.
//!
//! The tones are the real thing. A touch-tone keypad sends two sine waves at
//! once, one for the row and one for the column, and there are sixteen codes:
//! exactly as many as this console has gamepad buttons. Two pulse channels
//! stand in for the sines; a third, idle, channel plays MIDI notes at pitch.

#![no_std]
#![no_main]

use console::audio::Duty;
use console::input::{Key, PORTS};
use console::midi::{self, Message};
use console::video::{Rgb, Video, WIDTH};
use console::{entry, Console};

const BACKGROUND: u8 = 0;
const LABEL: u8 = 1;
const OFF: u8 = 2;
const ON: u8 = 3;
const AXIS: u8 = 4;
const LIVE: u8 = 5;
const DIM: u8 = 6;

const PALETTE: [Rgb; 7] = [
    Rgb::new(1, 1, 4),
    Rgb::new(12, 24, 31),
    Rgb::new(5, 6, 9),
    Rgb::new(31, 26, 6),
    Rgb::new(8, 10, 14),
    Rgb::new(6, 31, 12),
    Rgb::new(3, 4, 6),
];

/// DTMF row frequencies, in Hz.
const ROWS: [u16; 4] = [697, 770, 852, 941];
/// DTMF column frequencies.
const COLUMNS: [u16; 4] = [1209, 1336, 1477, 1633];

/// Play the tone pair for key `index` of a 4x4 keypad.
fn dtmf(console: &mut Console, index: u32) {
    let row = ROWS[(index as usize / 4) % 4];
    let column = COLUMNS[(index as usize % 4) % 4];
    console.audio.channel(0).play(row);
    console.audio.channel(1).play(column);
}

/// Short button names, in bit order.
const BUTTON_NAMES: [&str; 16] = [
    "U", "D", "L", "R", "A", "B", "X", "Y", "L1", "R1", "L2", "R2", "SE", "ST", "L3", "R3",
];

/// One gamepad's row of the display.
fn draw_pad(console: &mut Console, port: u32, y: i32) {
    let pad = console.input.gamepad(port);
    let video = &mut console.video;

    video.text(1, y, "P", LABEL);
    video.digit(5, y, port, LABEL);

    // Sixteen cells, lit while their button is down.
    for (bit, name) in BUTTON_NAMES.iter().enumerate() {
        let x = 12 + bit as i32 * 11;
        let held = pad.held & (1 << bit) != 0;
        video.rect(x, y - 1, 10, 7, if held { ON } else { OFF });
        video.text(x + 1, y, name, if held { BACKGROUND } else { DIM });
    }

    // Stick positions, as a dot inside a box.
    for (i, stick) in [pad.left, pad.right].iter().enumerate() {
        let box_x = 190 + i as i32 * 15;
        video.rect(box_x, y - 2, 12, 9, AXIS);
        let dx = i32::from(stick.x) * 4 / 127;
        let dy = i32::from(stick.y) * 3 / 127;
        video.rect(box_x + 5 + dx, y + 2 + dy, 2, 2, if pad.connected { LIVE } else { DIM });
    }

    // Analog triggers, as bars that fill from the left.
    for (i, level) in [pad.triggers.0, pad.triggers.1].iter().enumerate() {
        let bar_y = y - 2 + i as i32 * 4;
        video.rect(221, bar_y, 16, 3, AXIS);
        video.rect(221, bar_y, i32::from(*level) * 16 / 255, 3, LIVE);
    }
}

/// The keyboard, as a grid of 256 cells: 32 across by 8 down.
fn draw_keys(console: &mut Console, y: i32) {
    let held = console.input.keys_held();
    let video = &mut console.video;
    video.text(1, y, "KEYBOARD", LABEL);

    for code in 0..=255u8 {
        let x = 1 + (code as i32 % 32) * 4;
        let row = y + 8 + (code as i32 / 32) * 4;
        let down = held.contains_code(code);
        video.rect(x, row, 3, 3, if down { ON } else { OFF });
    }
}

/// The last few MIDI messages, as they arrived.
///
/// Deliberately excludes the clock. A running controller sends twenty-four
/// pulses a beat, which would push everything else off the list within a frame
/// — and the clock already has its own counters above. What belongs here is
/// the thing you pressed a button to find out about.
const LOG_DEPTH: usize = 8;

#[derive(Clone, Copy)]
struct Monitor {
    log: [(u8, u8, u8); LOG_DEPTH],
    count: usize,
    /// Total channel messages seen, so a repeated one is still visibly new.
    seen: u32,
    /// Frames left showing the activity lamp.
    lamp: u32,
}

impl Monitor {
    const fn new() -> Monitor {
        Monitor { log: [(0, 0, 0); LOG_DEPTH], count: 0, seen: 0, lamp: 0 }
    }

    fn push(&mut self, status: u8, data1: u8, data2: u8) {
        // Newest first, oldest pushed off the end.
        self.log.copy_within(0..LOG_DEPTH - 1, 1);
        self.log[0] = (status, data1, data2);
        self.count = (self.count + 1).min(LOG_DEPTH);
        self.seen = self.seen.wrapping_add(1);
        self.lamp = 10;
    }

    /// A short name for a status byte, so the raw hex has something beside it.
    fn label(status: u8) -> &'static str {
        match status {
            0xF8 => "CLOCK",
            0xFA => "START",
            0xFB => "CONT",
            0xFC => "STOP",
            0xFE => "SENSE",
            0xFF => "RESET",
            _ => match status & 0xF0 {
                0x80 => "NOTEOFF",
                0x90 => "NOTEON",
                0xA0 => "AFTER",
                0xB0 => "CC",
                0xC0 => "PROG",
                0xD0 => "CHPRESS",
                0xE0 => "BEND",
                _ => "SYS",
            },
        }
    }

    fn draw(&self, console: &mut Console, y: i32) {
        let connected = console.midi.is_connected();
        let running = console.midi.is_running();
        let clock = console.midi.clock();
        let pending = console.midi.pending();
        let video: &mut Video = &mut console.video;

        video.text(1, y, "MIDI", LABEL);
        video.text(22, y, if connected { "PORT" } else { "----" }, if connected { LIVE } else { DIM });
        video.text(44, y, if running { "RUN" } else { "STOP" }, if running { LIVE } else { DIM });

        // Clock pulses, and the beat they fall on: 24 pulses to a quarter note.
        video.text(66, y, "CLK", LABEL);
        video.number(80, y, clock % 100000, 5, if self.lamp > 0 { ON } else { DIM });
        video.text(104, y, "BEAT", LABEL);
        video.number(122, y, clock / midi::PULSES_PER_BEAT % 10000, 4, DIM);

        video.text(146, y, "MSG", LABEL);
        video.number(162, y, self.seen % 10000, 4, DIM);
        video.text(184, y, "Q", LABEL);
        video.number(190, y, u32::from(pending), 2, if pending > 0 { ON } else { DIM });

        // The raw bytes, newest at the top.
        for i in 0..self.count {
            let (status, data1, data2) = self.log[i];
            let row = y + 8 + i as i32 * 6;
            let colour = if i == 0 { ON } else { DIM };
            video.hex(1, row, u32::from(status), 2, colour);
            video.hex(11, row, u32::from(data1), 2, colour);
            video.hex(21, row, u32::from(data2), 2, colour);
            video.text(34, row, Monitor::label(status), colour);
        }
    }
}

fn main(mut console: Console) -> ! {
    console.print("io test\n");
    console.video.set_palettes(0, &PALETTE);

    console.audio.enable(10);
    console.audio.channel(0).set_volume(9).set_duty(Duty::Half).set_decay(6);
    console.audio.channel(1).set_volume(9).set_duty(Duty::Half).set_decay(6);
    // The triangle voice is otherwise idle, so MIDI notes get it to themselves
    // and can sound at their real pitch instead of borrowing the keypad tones.
    console.audio.channel(2).set_volume(9).set_decay(0);

    let mut monitor = Monitor::new();
    let mut sounding = None;

    loop {
        console.video.clear(BACKGROUND);
        console.video.text(1, 2, "IO TEST", LABEL);
        console.video.text(WIDTH - 44, 2, "ESC-EXIT", DIM);

        // Pulses only light the lamp; the counters above show the clock, and
        // logging them would bury everything else.
        if console.midi.take_ticks() > 0 {
            monitor.lamp = 10;
        }
        for message in console.midi.messages() {
            let (status, d1, d2) = raw(&message);
            monitor.push(status, d1, d2);
            if let Some((note, _)) = message.note_on() {
                console.audio.channel(2).play(midi::note_hz(note));
                sounding = Some(note);
            } else if let Some(note) = message.note_off() {
                // Only the note actually sounding can silence the channel — an
                // out-of-order release for an older note must not cut off
                // whatever is playing now.
                if sounding == Some(note) {
                    console.audio.channel(2).stop();
                    sounding = None;
                }
            }
        }

        // Sound the keypad for anything newly pressed, before the pads are
        // drawn — `gamepad` consumes the edge latch, so it has to happen once.
        let mut struck = None;
        for port in 0..PORTS {
            let pad = console.input.gamepad(port);
            if pad.pressed != 0 && struck.is_none() {
                struck = Some(pad.pressed.trailing_zeros());
            }
        }
        let pressed_keys = console.input.keys_pressed();
        if struck.is_none() {
            if let Some(code) = pressed_keys.codes().next() {
                // Spread key codes over the keypad so different keys differ.
                struck = Some(u32::from(code) % 16);
            }
        }
        if let Some(index) = struck {
            dtmf(&mut console, index);
        }

        for port in 0..PORTS {
            draw_pad(&mut console, port, 14 + port as i32 * 10);
        }
        draw_keys(&mut console, 56);

        // Modifier lamps.
        let modifiers = console.input.modifiers();
        for (i, name) in ["SHIFT", "CTRL", "ALT", "META"].iter().enumerate() {
            let lit = modifiers & (1 << i) != 0;
            console.video.text(150 + i as i32 * 22, 56, name, if lit { ON } else { DIM });
        }

        monitor.draw(&mut console, 98);
        monitor.lamp = monitor.lamp.saturating_sub(1);

        // Exiting costs nothing and needs to know nothing: stopping is the
        // whole protocol, and whatever started this gets the machine back.
        if pressed_keys.contains(Key::Escape) {
            console.exit();
        }

        console.wait_frame();
    }
}

entry!(main);

/// Repack a decoded message into the bytes it arrived as.
///
/// The monitor shows the wire, not the SDK's view of it: if a controller sends
/// something unexpected under a button, an interpretation would hide exactly
/// the detail you are looking for.
fn raw(message: &Message) -> (u8, u8, u8) {
    match *message {
        Message::NoteOff { channel, note, velocity } => (0x80 | channel, note, velocity),
        Message::NoteOn { channel, note, velocity } => (0x90 | channel, note, velocity),
        Message::ControlChange { channel, controller, value } => (0xB0 | channel, controller, value),
        Message::ProgramChange { channel, program } => (0xC0 | channel, program, 0),
        Message::PitchBend { channel, value } => {
            let raw = (value + 8192) as u16;
            (0xE0 | channel, (raw & 0x7F) as u8, (raw >> 7) as u8)
        }
        Message::Start => (0xFA, 0, 0),
        Message::Continue => (0xFB, 0, 0),
        Message::Stop => (0xFC, 0, 0),
        Message::Other { status, data } => (status, data.0, data.1),
    }
}
