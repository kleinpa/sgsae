//! A four-track step sequencer driving the FM chip, with a pool of voices for
//! playing along.
//!
//! Runs from its own clock or follows one arriving over MIDI. An external
//! device sends twenty-four pulses a beat; six of those is a sixteenth note, so
//! the sequencer counts pulses and steps when it has collected enough. Start
//! and Stop move the transport, and a Start rewinds — which is exactly what a
//! KeyStep's transport buttons do.
//!
//! The column down the left decides where played notes go. Four of those rows
//! are tracks, each with its own voice and its own timbre; arming one puts it
//! in record, and anything played then lands on the step under the cursor. The
//! fifth row turns on polyphony, at which point notes stop being recorded and
//! are spread across a pool of voices instead, so chords work.
//!
//! The cursor lives in one of two places: the track-select column on the
//! left, or a step of the selected track's grid. Left and Right cross between
//! them; what Up, Down and A do depends on which side of that line the cursor
//! is on.
//!
//! ```text
//!   left / right     move into the grid, or back out to the track column
//!
//!   on the track column:
//!     up / down        choose a row
//!     A / enter        arm the row: play, then record, then back
//!
//!   on a grid step:
//!     up / down        transpose the note
//!     A / enter        add a note (the default, if the step was empty)
//!     B / backspace    clear the step
//!
//!   always:
//!     B (on the track column)   internal clock / external clock
//!     start / space              run / stop
//!     L1 / R1                    tempo
//!     Q W E R                    ratio / index / decay / attack of the row
//!     escape / select            back to the launcher
//! ```

#![no_std]
#![no_main]

use console::audio::{Ratio, FM_VOICES};
use console::sys::SAMPLES_PER_FRAME;
use console::input::{Button, Key};
use console::midi::{self, Message};
use console::video::{Rgb, Video, HEIGHT, WIDTH};
use console::{entry, Console};

const BACKGROUND: u8 = 0;
const PANEL: u8 = 1;
const LABEL: u8 = 2;
const STEP_OFF: u8 = 3;
const STEP_ON: u8 = 4;
const CURSOR: u8 = 5;
const PLAYING: u8 = 6;
const ARMED: u8 = 7;
const DIM: u8 = 8;

const PALETTE: [Rgb; 9] = [
    Rgb::new(2, 1, 5),
    Rgb::new(7, 6, 14),
    Rgb::new(16, 24, 31),
    Rgb::new(6, 7, 13),
    Rgb::new(24, 10, 22),
    Rgb::new(31, 29, 12),
    Rgb::new(10, 31, 18),
    Rgb::new(31, 7, 7),
    Rgb::new(10, 11, 17),
];

const STEPS: usize = 16;
const TRACKS: usize = 4;
/// Rows in the left-hand column: the tracks, then the polyphony switch.
const ROWS: usize = TRACKS + 1;
const POLY_ROW: usize = TRACKS;

/// A rest.
const REST: u8 = 0;
/// Where a fresh note lands: middle C.
const DEFAULT_NOTE: u8 = 60;

/// The first voice of the polyphony pool: voices 0..4 belong to the tracks.
const POOL_BASE: u32 = TRACKS as u32;
/// How many notes can sound at once when polyphony is on.
const POOL: usize = 8;

/// Frames per step at the fastest and slowest internal tempos. Six frames is
/// 160 BPM in sixteenths at 60 fps; twenty-four is 40.
const FASTEST: u32 = 3;
const SLOWEST: u32 = 24;

const ROW_TOP: i32 = 22;
const ROW_HEIGHT: i32 = 18;
const GRID_LEFT: i32 = 46;
const CELL: i32 = (WIDTH - GRID_LEFT - 2) / STEPS as i32;

/// Where the sequencer takes its timing from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Clock {
    Internal,
    External,
}

/// Which column the cursor is in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The track-select column: Up/Down choose a row, A arms it.
    Tracks,
    /// A step of the selected track's grid: Up/Down transpose it, A adds a
    /// note, B clears it.
    Grid,
}

/// What a track does with the notes you play.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Plays its pattern and ignores the keyboard.
    Play,
    /// Plays its pattern, and what you play lands under the cursor.
    Record,
}

struct Track {
    notes: [u8; STEPS],
    mode: Mode,
    name: &'static str,
}

impl Track {
    const fn new(name: &'static str, notes: [u8; STEPS]) -> Track {
        Track { notes, mode: Mode::Play, name }
    }
}

/// The timbre of one row. Every row owns one, the polyphony pool included, so
/// a chord does not have to sound like whichever track was edited last.
#[derive(Clone, Copy)]
struct Tone {
    /// The modulator's frequency, in eighths of the carrier's.
    ratio: u8,
    /// How hard the modulator drives: the difference between a flute and a
    /// klaxon.
    index: u8,
    /// How fast the note dies away. Zero holds it for as long as it is gated.
    decay: u8,
    /// How fast the note reaches full volume. Zero is instant; the envelope
    /// ticks at 256 Hz, so even a small value here is a slow swell rather
    /// than the quick fade-in it looks like — worth keeping low on a voice
    /// meant to answer a played note rather than a held pad.
    attack: u8,
}

impl Tone {
    const fn new(ratio: Ratio, index: u8, decay: u8, attack: u8) -> Tone {
        Tone { ratio: ratio.0, index, decay, attack }
    }
}

/// What the player asked for this frame, whichever control they used.
#[derive(Default, Clone, Copy)]
struct Controls {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    arm: bool,
    clear: bool,
    transport: bool,
    faster: bool,
    slower: bool,
    ratio: bool,
    index: bool,
    decay: bool,
    attack: bool,
    quit: bool,
}

struct Sequencer {
    tracks: [Track; TRACKS],
    /// One per row: the four tracks, then the polyphony pool.
    tones: [Tone; ROWS],
    /// Which row of the left-hand column the keyboard is talking to.
    row: usize,
    /// Which column the cursor is in.
    focus: Focus,
    cursor: usize,
    step: usize,
    running: bool,
    poly: bool,
    clock: Clock,
    /// Frames per step, when running internally.
    period: u32,
    countdown: u32,
    /// MIDI pulses collected towards the next step.
    pulses: u32,
    /// The note each pooled voice is holding, so a note-off knows which to
    /// release. `REST` means the voice is free.
    pool: [u8; POOL],
    /// Round-robin position, for when every pooled voice is busy.
    next_voice: usize,
    /// Whether anything was played into the step now sounding. Live recording
    /// clears a step that went by untouched.
    wrote: bool,
    /// Frames left showing the "MIDI in" lamp.
    lamp: u32,
}

impl Sequencer {
    fn new() -> Sequencer {
        Sequencer {
            tracks: [
                // Something to hear on the first frame, rather than four rows
                // of rests.
                Track::new("T1", [60, 0, 63, 0, 67, 0, 63, 0, 70, 0, 67, 0, 63, 0, 60, 0]),
                Track::new("T2", [0; STEPS]),
                Track::new("T3", [36, 0, 0, 0, 36, 0, 0, 0, 43, 0, 0, 0, 36, 0, 0, 0]),
                Track::new("T4", [0; STEPS]),
            ],
            // Five voices that do not sound alike, so a pattern spread across
            // the tracks is legible by ear.
            tones: [
                Tone::new(Ratio::UNISON, 96, 6, 0),  // lead
                Tone::new(Ratio::OCTAVE, 130, 9, 0), // pluck
                Tone::new(Ratio::SUB, 64, 4, 0),     // bass
                Tone::new(Ratio::BELL, 180, 2, 0),   // bell
                // The pool answers a keyboard in real time, so it starts with
                // an instant attack too — a swell here is the one that most
                // often gets played into by accident.
                Tone::new(Ratio::OCTAVE, 110, 0, 0),
            ],
            row: 0,
            focus: Focus::Tracks,
            cursor: 0,
            step: 0,
            running: true,
            poly: false,
            clock: Clock::Internal,
            period: 8,
            countdown: 8,
            pulses: 0,
            pool: [REST; POOL],
            next_voice: 0,
            wrote: false,
            lamp: 0,
        }
    }

    /// The track the keyboard is talking to, unless the polyphony row is
    /// selected.
    fn selected(&self) -> Option<usize> {
        (self.row < TRACKS).then_some(self.row)
    }

    /// The track currently armed, if any. Only ever one, so a played note has
    /// exactly one destination.
    fn armed(&self) -> Option<usize> {
        self.tracks.iter().position(|track| track.mode == Mode::Record)
    }

    /// Sound every track's note for the current step.
    fn strike(&self, console: &mut Console) {
        for (i, track) in self.tracks.iter().enumerate() {
            let note = track.notes[self.step];
            if note != REST {
                console.audio.fm(i as u32).play(midi::note_hz(note));
            }
        }
    }

    fn advance(&mut self, console: &mut Console) {
        // Live recording replaces rather than overlays: a step the playhead
        // passed without anything being played is cleared, so going round the
        // loop rewrites the bar instead of piling notes onto it.
        if let Some(index) = self.armed() {
            if !self.wrote {
                self.tracks[index].notes[self.step] = REST;
            }
        }
        self.wrote = false;
        self.step = (self.step + 1) % STEPS;
        self.strike(console);
    }

    // -- notes in -----------------------------------------------------------

    /// Take a note from outside: record it, sound it, or both.
    fn note_on(&mut self, note: u8, console: &mut Console) {
        self.lamp = 8;

        // Polyphony wins: it exists so chords can be played, and recording a
        // chord into one monophonic step would only lose most of it.
        if !self.poly {
            if let Some(index) = self.armed() {
                if self.running {
                    // Playing along with a running sequence writes where the
                    // playhead is, which is the step you just heard yourself
                    // play against.
                    self.tracks[index].notes[self.step] = note;
                    self.wrote = true;
                } else {
                    // Stopped, it is step entry: each note lands under the
                    // cursor and moves it on.
                    self.tracks[index].notes[self.cursor] = note;
                    self.cursor = (self.cursor + 1) % STEPS;
                }
                // Sound it on that track's voice, so you hear what you wrote
                // with the timbre it will have.
                console.audio.fm(index as u32).play(midi::note_hz(note));
                return;
            }
        }

        self.hold(note, console);
    }

    /// Give `note` a voice out of the pool.
    fn hold(&mut self, note: u8, console: &mut Console) {
        // Prefer a free voice; with none left, take the oldest, which is what
        // stealing meant on hardware that ran out.
        let slot = match self.pool.iter().position(|held| *held == REST) {
            Some(free) => free,
            None => self.next_voice % POOL,
        };
        self.pool[slot] = note;
        self.next_voice = self.next_voice.wrapping_add(1);
        console.audio.fm(POOL_BASE + slot as u32).play(midi::note_hz(note));
    }

    /// Release whichever pooled voice is holding `note`.
    fn note_off(&mut self, note: u8, console: &mut Console) {
        for slot in 0..POOL {
            if self.pool[slot] == note {
                self.pool[slot] = REST;
                console.audio.fm(POOL_BASE + slot as u32).set_gate(false);
            }
        }
    }

    fn release_pool(&mut self, console: &mut Console) {
        for slot in 0..POOL {
            self.pool[slot] = REST;
            console.audio.fm(POOL_BASE + slot as u32).set_gate(false);
        }
    }

    /// Take everything MIDI has to say this frame.
    fn read_midi(&mut self, console: &mut Console) {
        // The clock first: ticks are counted, not queued, so this is a delta
        // rather than a stream of messages.
        let ticks = u32::from(console.midi.take_ticks());
        if ticks > 0 {
            self.lamp = 8;
            if self.clock == Clock::External && self.running {
                self.pulses += ticks;
                while self.pulses >= midi::PULSES_PER_STEP {
                    self.pulses -= midi::PULSES_PER_STEP;
                    self.advance(console);
                }
            }
        }

        // Drain into a buffer first: acting on a message needs the console,
        // and the iterator is borrowing it.
        let mut inbox = [Message::Stop; 16];
        let mut count = 0;
        for message in console.midi.messages() {
            if count < inbox.len() {
                inbox[count] = message;
                count += 1;
            }
        }

        for message in &inbox[..count] {
            match *message {
                Message::Start => {
                    // Start rewinds; Continue picks up where it stopped.
                    self.clock = Clock::External;
                    self.running = true;
                    self.step = STEPS - 1;
                    self.pulses = 0;
                }
                Message::Continue => {
                    self.clock = Clock::External;
                    self.running = true;
                }
                Message::Stop => {
                    if self.clock == Clock::External {
                        self.running = false;
                    }
                }
                other => {
                    if let Some(note) = other.note_off() {
                        self.note_off(note, console);
                    } else if let Some((note, _velocity)) = other.note_on() {
                        self.note_on(note, console);
                    }
                }
            }
        }
    }

    // -- controls -----------------------------------------------------------

    fn read_controls(&mut self, console: &mut Console) -> Controls {
        let pad = console.input.gamepad(0);
        // Read once: the latch clears, so a second call this frame sees
        // nothing.
        let keys = console.input.keys_pressed();

        Controls {
            up: pad.was_pressed(Button::Up) || keys.contains(Key::Up),
            down: pad.was_pressed(Button::Down) || keys.contains(Key::Down),
            left: pad.was_pressed(Button::Left) || keys.contains(Key::Left),
            right: pad.was_pressed(Button::Right) || keys.contains(Key::Right),
            arm: pad.was_pressed(Button::A) || keys.contains(Key::Enter),
            clear: pad.was_pressed(Button::B) || keys.contains(Key::Backspace) || keys.contains(Key::C),
            transport: pad.was_pressed(Button::Start) || keys.contains(Key::Space),
            faster: pad.was_pressed(Button::R1) || keys.contains(Key::Equal),
            slower: pad.was_pressed(Button::L1) || keys.contains(Key::Minus),
            ratio: keys.contains(Key::Q),
            index: keys.contains(Key::W),
            decay: keys.contains(Key::E),
            attack: keys.contains(Key::R),
            quit: pad.was_pressed(Button::Select) || keys.contains(Key::Escape),
        }
    }

    fn apply(&mut self, controls: Controls, console: &mut Console) {
        match self.focus {
            Focus::Tracks => {
                if controls.up {
                    self.row = (self.row + ROWS - 1) % ROWS;
                }
                if controls.down {
                    self.row = (self.row + 1) % ROWS;
                }
                if controls.right && self.selected().is_some() {
                    // The pool has no grid: there is nowhere for Right to go
                    // from the polyphony row.
                    self.focus = Focus::Grid;
                }
                if controls.arm {
                    self.arm_selected(console);
                }
                // On the track column, B is the transport's clock source
                // rather than an edit — there is no step here to clear.
                if controls.clear {
                    self.clock = match self.clock {
                        Clock::Internal => Clock::External,
                        Clock::External => Clock::Internal,
                    };
                    self.pulses = 0;
                }
            }
            Focus::Grid => {
                let Some(index) = self.selected() else {
                    // Unreachable in practice: Right only enters the grid from
                    // a track row, and nothing else changes `row` while here.
                    self.focus = Focus::Tracks;
                    return;
                };

                if controls.left {
                    if self.cursor == 0 {
                        self.focus = Focus::Tracks;
                    } else {
                        self.cursor -= 1;
                    }
                }
                if controls.right {
                    self.cursor = (self.cursor + 1) % STEPS;
                }

                let note = &mut self.tracks[index].notes[self.cursor];
                if controls.up {
                    let base = if *note == REST { DEFAULT_NOTE } else { *note };
                    *note = (i32::from(base) + 1).clamp(1, 127) as u8;
                }
                if controls.down {
                    let base = if *note == REST { DEFAULT_NOTE } else { *note };
                    *note = (i32::from(base) - 1).clamp(1, 127) as u8;
                }
                if controls.arm && *note == REST {
                    *note = DEFAULT_NOTE;
                }
                if controls.clear {
                    *note = REST;
                }
            }
        }

        let tone = &mut self.tones[self.row];
        if controls.ratio {
            tone.ratio = if tone.ratio >= 40 { 4 } else { tone.ratio + 4 };
        }
        if controls.index {
            tone.index = tone.index.wrapping_add(32);
        }
        if controls.decay {
            // Zero holds the note; wrapping past the top comes back to it.
            tone.decay = if tone.decay >= 12 { 0 } else { tone.decay + 2 };
        }
        if controls.attack {
            tone.attack = if tone.attack >= 12 { 0 } else { tone.attack + 2 };
        }

        if controls.transport {
            self.running = !self.running;
            self.countdown = 1;
        }
        // Tempo, only meaningful on the internal clock.
        if controls.faster {
            self.period = self.period.saturating_sub(1).max(FASTEST);
        }
        if controls.slower {
            self.period = (self.period + 1).min(SLOWEST);
        }
    }

    /// Cycle the selected row: a track between play and record, or the
    /// polyphony switch on and off.
    fn arm_selected(&mut self, console: &mut Console) {
        let Some(index) = self.selected() else {
            self.poly = !self.poly;
            if !self.poly {
                // Nothing else will ever send a note-off for these.
                self.release_pool(console);
            }
            return;
        };

        // Only one track records at a time, so a played note has a single
        // destination and arming a second does not quietly write to both.
        let arming = self.tracks[index].mode == Mode::Play;
        for track in &mut self.tracks {
            track.mode = Mode::Play;
        }
        if arming {
            self.tracks[index].mode = Mode::Record;
        }
    }

    fn tick_internal(&mut self, console: &mut Console) {
        if self.clock != Clock::Internal || !self.running {
            return;
        }
        self.countdown = self.countdown.saturating_sub(1);
        if self.countdown == 0 {
            self.countdown = self.period;
            self.advance(console);
        }
    }

    /// Push each track's timbre to its voice. Live, so a change is heard on
    /// the note already ringing.
    fn apply_timbre(&self, console: &mut Console) {
        for (row, tone) in self.tones.iter().enumerate() {
            if row == POLY_ROW {
                // One tone across the whole pool: it is one instrument that
                // happens to have eight voices.
                for slot in 0..POOL as u32 {
                    Self::set_tone(console.audio.fm(POOL_BASE + slot), tone);
                }
            } else {
                Self::set_tone(console.audio.fm(row as u32), tone);
            }
        }
    }

    fn set_tone(mut voice: console::audio::Voice<'_>, tone: &Tone) {
        voice.set_ratio(Ratio(tone.ratio)).set_index(tone.index).set_decay(tone.decay).set_attack(tone.attack);
    }

    // -- drawing ------------------------------------------------------------

    fn draw(&self, console: &mut Console) {
        let connected = console.midi.is_connected();
        let sounding = console.audio.fm_status().count_ones();
        let video = &mut console.video;
        video.clear(BACKGROUND);

        video.rect(0, 0, WIDTH, 14, PANEL);
        video.text(3, 4, "FM SEQ", LABEL);

        let external = self.clock == Clock::External;
        video.text(44, 4, if external { "EXT" } else { "INT" }, if external { PLAYING } else { LABEL });
        video.text(64, 4, if self.running { "RUN" } else { "STOP" }, if self.running { PLAYING } else { STEP_OFF });
        video.text(92, 4, "T", DIM);
        video.number(98, 4, self.period, 2, LABEL);

        video.text(118, 4, "MIDI", if self.lamp > 0 { PLAYING } else { DIM });
        if connected {
            video.rect(139, 4, 3, 5, PLAYING);
        }
        // Voices in use, tracks and pool together.
        video.text(152, 4, "VOX", DIM);
        video.number(170, 4, sounding, 2, LABEL);

        self.draw_rows(video);
        self.draw_grid(video);
        self.draw_settings(video);
        self.draw_keyboard(video);

        video.rect(0, HEIGHT - 12, WIDTH, 12, PANEL);
        let help = match self.focus {
            Focus::Tracks => "UD ROW  A ARM  B CLOCK  RIGHT GRID",
            Focus::Grid => "UD NOTE  A ADD  B CLEAR  LEFT ROWS",
        };
        video.text(3, HEIGHT - 9, help, DIM);
    }

    /// The column down the left: four tracks, then the polyphony switch.
    fn draw_rows(&self, video: &mut Video) {
        // Bright while this is where the cursor keys act, dim while they are
        // busy editing a step instead — the same distinction the grid cursor
        // draws in reverse.
        let active = if self.focus == Focus::Tracks { CURSOR } else { DIM };
        for row in 0..ROWS {
            let y = ROW_TOP + row as i32 * ROW_HEIGHT;
            let chosen = row == self.row;
            if chosen {
                video.rect(0, y - 3, GRID_LEFT - 3, ROW_HEIGHT - 2, PANEL);
                // A pointer, so the selected row still reads on a small screen.
                video.text(0, y, ">", active);
            }

            if row == POLY_ROW {
                video.text(6, y, "POLY", if self.poly { PLAYING } else { DIM });
                let held = self.pool.iter().filter(|note| **note != REST).count() as u32;
                video.number(30, y, held, 1, if held > 0 { PLAYING } else { DIM });
            } else {
                let track = &self.tracks[row];
                let recording = track.mode == Mode::Record;
                video.text(6, y, track.name, if chosen { LABEL } else { DIM });
                video.text(18, y, if recording { "REC" } else { "PLAY" }, if recording { ARMED } else { DIM });
                if recording {
                    // A lamp, so an armed track is obvious at a glance.
                    video.rect(GRID_LEFT - 7, y, 3, 5, ARMED);
                }
            }
        }
    }

    /// The tone of whichever row is selected, and where a recorded note will
    /// land.
    fn draw_settings(&self, video: &mut Video) {
        let y = ROW_TOP + ROWS as i32 * ROW_HEIGHT + 2;
        let tone = &self.tones[self.row];
        let name = match self.selected() {
            Some(index) => self.tracks[index].name,
            None => "POLY",
        };

        video.text(4, y, name, LABEL);
        video.text(22, y, "RAT", DIM);
        video.number(34, y, u32::from(tone.ratio), 2, LABEL);
        video.text(46, y, "IDX", DIM);
        video.number(58, y, u32::from(tone.index), 3, LABEL);
        video.text(74, y, "DEC", DIM);
        video.number(86, y, u32::from(tone.decay), 2, LABEL);
        video.text(98, y, "ATK", DIM);
        video.number(110, y, u32::from(tone.attack), 2, LABEL);

        // While a running track is armed, notes land on the playhead rather
        // than the cursor. Say which, or recording looks like it missed.
        if let Some(index) = self.selected() {
            let live = self.running && self.tracks[index].mode == Mode::Record;
            let (label, step) = if live { ("LIVE", self.step) } else { ("STEP", self.cursor) };
            video.text(124, y, label, if live { ARMED } else { DIM });
            video.number(150, y, step as u32 + 1, 2, LABEL);
        }
    }

    /// Is `note` sounding right now, either from a track or from the pool?
    fn held(&self, note: u8) -> bool {
        self.pool.contains(&note)
            || (self.running && self.tracks.iter().any(|t| t.notes[self.step] == note))
    }

    /// Three octaves, lighting whatever is sounding. Chords are the reason
    /// polyphony exists, and a keyboard is the one picture that shows them.
    fn draw_keyboard(&self, video: &mut Video) {
        /// Semitone of each white key within an octave.
        const WHITE: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
        /// White key each black key sits to the right of, and its semitone.
        const BLACK: [(i32, u8); 5] = [(0, 1), (1, 3), (3, 6), (4, 8), (5, 10)];
        // Four octaves from C2 covers the range the tracks and a played
        // chord actually sit in.
        const OCTAVES: i32 = 4;
        const KEY: i32 = 8;
        const BLACK_W: i32 = 5;

        let left = (WIDTH - OCTAVES * 7 * KEY) / 2;
        let top = HEIGHT - 36;
        let base = 36; // C2

        for w in 0..OCTAVES * 7 {
            let note = base + (w / 7) as u8 * 12 + WHITE[(w % 7) as usize];
            let x = left + w * KEY;
            let lit = self.held(note);
            video.rect(x, top, KEY - 1, 24, if lit { PLAYING } else { LABEL });
            if w % 7 == 0 {
                // Mark the Cs, so the octave a note landed in is readable.
                video.digit(x + 2, top + 18, u32::from(midi::note_octave(note)), BACKGROUND);
            }
        }
        for octave in 0..OCTAVES {
            for (after, semitone) in BLACK {
                let note = base + (octave as u8) * 12 + semitone;
                let x = left + (octave * 7 + after + 1) * KEY - BLACK_W / 2 - 1;
                let lit = self.held(note);
                video.rect(x, top, BLACK_W, 14, if lit { PLAYING } else { BACKGROUND });
            }
        }
    }

    fn draw_grid(&self, video: &mut Video) {
        for (row, track) in self.tracks.iter().enumerate() {
            let y = ROW_TOP + row as i32 * ROW_HEIGHT;
            for (i, note) in track.notes.iter().enumerate() {
                let x = GRID_LEFT + i as i32 * CELL;
                let live = self.running && i == self.step;
                let fill = if live {
                    PLAYING
                } else if *note == REST {
                    STEP_OFF
                } else {
                    STEP_ON
                };
                video.rect(x, y - 3, CELL - 1, ROW_HEIGHT - 2, fill);
                if *note != REST {
                    // Name above, octave below: twelve pixels is not enough for
                    // "C#4" on one line, and without the octave a C4 and a C5
                    // are the same picture.
                    video.text(x + 1, y, midi::note_name(*note), BACKGROUND);
                    video.digit(x + 4, y + 6, u32::from(midi::note_octave(*note)), BACKGROUND);
                }
            }

            // The cursor belongs to the row being edited; drawing it on all
            // four would say the wrong thing about where a note will land. It
            // outlines the cell rather than sitting above and below it, where
            // the top edge would look like it belonged to the row above.
            if row == self.row {
                // Bright while the cursor keys are editing here, dim while
                // they are back on the track column — it still marks where
                // Right will resume, just not as the thing in charge.
                let active = if self.focus == Focus::Grid { CURSOR } else { DIM };
                let x = GRID_LEFT + self.cursor as i32 * CELL;
                let (w, h) = (CELL - 1, ROW_HEIGHT - 2);
                video.rect(x, y - 3, w, 1, active);
                video.rect(x, y - 4 + h, w, 1, active);
                video.vline(x, y - 3, h, active);
                video.vline(x + w - 1, y - 3, h, active);
            }
        }

        // Beat markers under every fourth step.
        let y = ROW_TOP + TRACKS as i32 * ROW_HEIGHT - 4;
        for beat in 0..4 {
            video.rect(GRID_LEFT + beat * CELL * 4, y, CELL * 4 - 2, 1, PANEL);
        }
    }
}

/// How often to poll MIDI between frames. Five times a frame is every 3.3 ms.
const MIDI_POLLS_PER_FRAME: u32 = 5;

fn main(mut console: Console) -> ! {
    console.print("fm seq\n");
    console.video.set_palettes(0, &PALETTE);

    // Voices 0..4 are the tracks; 4..12 are the polyphony pool. That is twelve
    // of the chip's sixteen, so a chord never has to steal a track's voice.
    const _: () = assert!(POOL_BASE as usize + POOL <= FM_VOICES as usize);

    console.audio.enable(13);
    for voice in 0..TRACKS as u32 {
        console.audio.fm(voice).set_volume(13).set_attack(0);
    }
    // The pool is quieter than the tracks: eight of them can sound at once,
    // and a held chord should sit under the pattern rather than bury it.
    for slot in 0..POOL as u32 {
        console.audio.fm(POOL_BASE + slot).set_volume(9).set_attack(1);
    }

    let mut seq = Sequencer::new();

    loop {
        let controls = seq.read_controls(&mut console);
        seq.apply(controls, &mut console);
        seq.tick_internal(&mut console);
        seq.apply_timbre(&mut console);

        seq.draw(&mut console);
        seq.lamp = seq.lamp.saturating_sub(1);

        if controls.quit {
            console.exit();
        }

        // Between frames, listen at audio rate rather than frame rate: a note
        // played live, or a clock pulse from outside, is acted on within a few
        // milliseconds instead of at the next frame.
        for _ in 0..MIDI_POLLS_PER_FRAME {
            console.wait_samples(SAMPLES_PER_FRAME / MIDI_POLLS_PER_FRAME);
            seq.read_midi(&mut console);
        }
    }
}

entry!(main);
