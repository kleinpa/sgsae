//! Runs the toolchain-built cartridge on the emulator.
//!
//! This is the end-to-end check that the whole chain holds together: rustc
//! builds guest code for //platforms:rv32, lld links it against console.ld, the
//! ELF loader places it, and the interpreter executes it — compressed
//! instructions and all.

use sgsae::console::{Console, Frame, Mode};
use sgsae::input::{Button, Pad};

/// Find a cartridge in the test's runfiles.
fn cartridge_named(name: &str) -> Vec<u8> {
    let candidates = [
        format!("game/{name}.elf"),
        format!("_main/game/{name}.elf"),
        format!("../_main/game/{name}.elf"),
    ];
    for path in &candidates {
        if let Ok(bytes) = std::fs::read(path) {
            return bytes;
        }
    }
    panic!("{name}.elf not found; looked in {candidates:?} from {:?}", std::env::current_dir());
}

fn cartridge() -> Vec<u8> {
    cartridge_named("lander")
}

/// Run the game for `frames`, holding `held` on gamepad 0 the whole time.
fn play(held: u16, frames: u32) -> Console {
    play_with(frames, |console| {
        console.input.borrow_mut().set_pad(
            0,
            Pad { buttons: held, connected: true, ..Pad::default() },
        );
    })
}

/// Run the game for `frames`, letting `feed` set the input before each one.
fn play_with(frames: u32, mut feed: impl FnMut(&mut Console)) -> Console {
    let mut console = Console::with_cartridge(&cartridge()).expect("load cartridge");
    for _ in 0..frames {
        feed(&mut console);
        match console.advance_frame() {
            Frame::Presented => {}
            Frame::Returned => panic!("the game handed the machine back unexpectedly"),
            Frame::Load(url) => panic!("the game asked to load {url}"),
            Frame::Stopped(outcome) => panic!("the game stopped early: {outcome}"),
        }
    }
    console
}

#[test]
fn the_game_boots_and_takes_over_the_hardware() {
    let mut console = play(0, 5);
    let text = console.console.borrow_mut().take_string();
    assert!(text.starts_with("lunar lander"), "console said {text:?}");
    assert_eq!(console.frames(), 5);
}

#[test]
fn gravity_brings_it_down_if_you_do_nothing() {
    let mut console = play(0, 240);
    let text = console.console.borrow_mut().take_string();
    assert!(text.contains("crashed"), "expected a crash, console said {text:?}");
}

#[test]
fn holding_the_thruster_keeps_it_flying() {
    // The same window that ends in a crash with no input should not with the
    // main engine lit — that is the whole game in one assertion.
    let mut console = play(Button::Up.mask(), 240);
    let text = console.console.borrow_mut().take_string();
    assert!(!text.contains("crashed"), "thrusting still crashed: {text:?}");
}

#[test]
fn the_thruster_makes_a_noise() {
    let quiet = play(0, 30);
    let burning = play(Button::Up.mask(), 30);

    let peak = |c: &Console| c.audio().iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert_eq!(peak(&quiet), 0.0, "the game was noisy before anything happened");
    assert!(peak(&burning) > 0.0, "the thruster was silent");
}

#[test]
fn the_analog_stick_flies_it_too() {
    // Pushing the left stick up must thrust, exactly as the d-pad does. The
    // game never learns which one the player used.
    let mut console = play_with(240, |console| {
        console.input.borrow_mut().set_pad(
            0,
            Pad { left: (0, -110), connected: true, ..Pad::default() },
        );
    });
    let text = console.console.borrow_mut().take_string();
    assert!(!text.contains("crashed"), "stick thrust did not fly it: {text:?}");
}

#[test]
fn the_keyboard_flies_it_too() {
    // `Key::W` is 23 in sdk/input.rs.
    let mut console = play_with(240, |console| {
        console.input.borrow_mut().set_key(23, true);
    });
    let text = console.console.borrow_mut().take_string();
    assert!(!text.contains("crashed"), "keyboard thrust did not fly it: {text:?}");
}

#[test]
fn a_stick_inside_the_dead_zone_does_nothing() {
    // A pad that does not quite centre must not fly the lander by itself.
    let mut console = play_with(240, |console| {
        console.input.borrow_mut().set_pad(
            0,
            Pad { left: (0, -30), connected: true, ..Pad::default() },
        );
    });
    let text = console.console.borrow_mut().take_string();
    assert!(text.contains("crashed"), "dead-zone drift was treated as thrust");
}

#[test]
fn restarting_after_a_crash_refuels() {
    // Crash it, then hold Start; the lander should be flying again.
    let mut console = Console::with_cartridge(&cartridge()).expect("load cartridge");
    for _ in 0..240 {
        console.advance_frame();
    }
    assert!(console.console.borrow_mut().take_string().contains("crashed"));

    for _ in 0..120 {
        console.input.borrow_mut().set_pad(
            0,
            Pad {
                buttons: Button::Start.mask() | Button::Up.mask(),
                connected: true,
                ..Pad::default()
            },
        );
        console.advance_frame();
    }
    // A fresh round crashes again only if it runs long enough; within this
    // window, thrusting from the new spawn keeps it alive.
    let text = console.console.borrow_mut().take_string();
    assert!(!text.contains("crashed"), "restart did not take: {text:?}");
}

/// Boot the supervisor and drive it, resolving whatever it asks to load.
///
/// This is the host side of the operating system in miniature: start the
/// supervisor, answer its requests, and let a program that stops hand the
/// machine back on its own.
fn boot(frames: u32, mut feed: impl FnMut(u32, &mut Console)) -> (Console, Vec<String>) {
    let mut console = Console::with_supervisor(&cartridge_named("loader")).expect("supervisor");
    let mut opened = Vec::new();
    for i in 0..frames {
        feed(i, &mut console);
        match console.advance_frame() {
            Frame::Presented | Frame::Returned => {}
            Frame::Load(url) => {
                opened.push(url.clone());
                let name = url.trim_start_matches('/').trim_end_matches(".elf");
                console.load_program(&cartridge_named(name)).expect("load");
            }
            Frame::Stopped(outcome) => panic!("the console stopped: {outcome}"),
        }
    }
    (console, opened)
}

/// Boot `name` and run it, letting `feed` set the input before each frame.
/// Returns the address it asked for, if it asked for one.
fn browse(name: &str, frames: u32, mut feed: impl FnMut(u32, &mut Console)) -> (Console, Option<String>) {
    let mut console = Console::with_cartridge(&cartridge_named(name)).expect("load");
    for i in 0..frames {
        feed(i, &mut console);
        match console.advance_frame() {
            Frame::Presented | Frame::Returned => {}
            Frame::Load(url) => return (console, Some(url)),
            Frame::Stopped(outcome) => panic!("{name} stopped: {outcome}"),
        }
    }
    (console, None)
}

/// Tap key `code` on frame `at`, holding it for two frames.
fn tap(i: u32, at: u32, code: u8, console: &mut Console) {
    if i == at {
        console.input.borrow_mut().set_key(code, true);
    }
    if i == at + 2 {
        console.input.borrow_mut().set_key(code, false);
    }
}

const KEY_ENTER: u8 = 41;
const KEY_DOWN: u8 = 71;
const KEY_ESCAPE: u8 = 40;
const KEY_Q: u8 = 17;
const KEY_SPACE: u8 = 43;
const KEY_R: u8 = 18;
const KEY_UP: u8 = 70;
const KEY_LEFT: u8 = 72;
const KEY_RIGHT: u8 = 73;
const KEY_BACKSPACE: u8 = 44;

/// Run `name` on its own and press Escape. True if it stopped.
///
/// With no supervisor behind it, stopping ends the machine — which is exactly
/// what a program exiting looks like from the outside.
fn quits(name: &str) -> bool {
    let mut console = Console::with_cartridge(&cartridge_named(name)).expect("load");
    for i in 0..60 {
        tap(i, 5, KEY_ESCAPE, &mut console);
        if let Frame::Stopped(_) = console.advance_frame() {
            return true;
        }
    }
    false
}

#[test]
fn the_supervisor_boots_and_draws_its_index() {
    let (mut console, asked) = browse("loader", 10, |_, _| {});
    assert_eq!(asked, None, "the supervisor should sit still until told to move");
    assert!(console.console.borrow_mut().take_string().starts_with("loader"));

    // A menu is mostly text, so the screen should hold several colours.
    let ppu = console.ppu.borrow();
    let distinct: std::collections::HashSet<&[u8]> =
        ppu.framebuffer().chunks_exact(4).collect();
    assert!(distinct.len() >= 4, "the index drew only {} colours", distinct.len());
}

#[test]
fn following_a_link_asks_the_host_to_load_it() {
    let (_, asked) = browse("loader", 40, |i, c| tap(i, 5, KEY_ENTER, c));
    assert_eq!(asked.as_deref(), Some("lander"), "the first link is the lander");
}

#[test]
fn the_cursor_moves_one_row_per_keystroke() {
    // One tap down then open must reach the second link, not skip past it.
    let (_, asked) = browse("loader", 60, |i, c| {
        tap(i, 5, KEY_DOWN, c);
        tap(i, 30, KEY_ENTER, c);
    });
    assert_eq!(asked.as_deref(), Some("seq"));

    // Two taps, two rows.
    let (_, asked) = browse("loader", 80, |i, c| {
        tap(i, 5, KEY_DOWN, c);
        tap(i, 15, KEY_DOWN, c);
        tap(i, 40, KEY_ENTER, c);
    });
    assert_eq!(asked.as_deref(), Some("io"));
}

#[test]
fn the_selection_wraps_around_the_list() {
    // Three entries, so three taps returns to the first.
    let (_, asked) = browse("loader", 110, |i, c| {
        tap(i, 5, KEY_DOWN, c);
        tap(i, 15, KEY_DOWN, c);
        tap(i, 25, KEY_DOWN, c);
        tap(i, 50, KEY_ENTER, c);
    });
    assert_eq!(asked.as_deref(), Some("lander"));
}

#[test]
fn the_io_test_draws_every_input_it_is_given() {
    let (mut console, _) = browse("io", 20, |i, c| {
        if i == 5 {
            let mut input = c.input.borrow_mut();
            input.set_pad(
                1,
                Pad {
                    buttons: Button::A.mask() | Button::R3.mask(),
                    left: (-120, 100),
                    right: (60, -60),
                    triggers: (0, 255),
                    connected: true,
                },
            );
            input.set_key(19, true);
        }
    });
    assert!(console.console.borrow_mut().take_string().starts_with("io test"));

    // Something has to be lit: the pressed cells use their own palette entry.
    let lit = console.vram().iter().filter(|c| **c == 3).count();
    assert!(lit > 0, "nothing on the io test lit up");
}

#[test]
fn pressing_a_button_sounds_a_dtmf_pair() {
    let quiet = browse("io", 12, |_, _| {}).0;
    let struck = browse("io", 12, |i, c| {
        if i == 5 {
            c.input.borrow_mut().set_pad(
                0,
                Pad { buttons: Button::A.mask(), connected: true, ..Pad::default() },
            );
        }
    })
    .0;

    let peak = |c: &Console| c.audio().iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert_eq!(peak(&quiet), 0.0, "the io test made noise before anything happened");
    assert!(peak(&struck) > 0.0, "pressing a button was silent");
}

/// The pixels of the io test's MIDI message log, below its header row.
///
/// Starts at 104: the header itself is drawn at y=98 with 5px glyphs, and its
/// CLK readout changes colour with clock activity, which would make an
/// otherwise-quiet log region look different while pulses are arriving.
fn midi_log(console: &Console) -> Vec<u8> {
    (104..150)
        .flat_map(|y| (0..90).map(move |x| (y, x)))
        .map(|(y, x)| console.vram()[y * sgsae::ppu::WIDTH + x])
        .collect()
}

#[test]
fn the_io_test_logs_what_arrived_over_midi() {
    let quiet = browse("io", 12, |_, _| {}).0;
    let noted = browse("io", 12, |i, c| {
        if i == 4 {
            midi(c, &[MIDI_NOTE_ON, 60, 100]);
        }
    })
    .0;
    assert_ne!(midi_log(&quiet), midi_log(&noted), "the note was not logged");
}

#[test]
fn the_clock_does_not_flood_the_midi_log() {
    // A running controller sends twenty-four pulses a beat. If those went into
    // the log they would push out the message you were trying to read within a
    // single frame — which is the whole reason to look at the log at all.
    let quiet = browse("io", 16, |_, _| {}).0;
    let ticking = browse("io", 16, |i, c| {
        if i >= 2 {
            for _ in 0..PULSES_PER_STEP * 4 {
                midi(c, &[MIDI_CLOCK]);
            }
        }
    })
    .0;
    assert_eq!(midi_log(&quiet), midi_log(&ticking), "clock pulses filled the log");

    // And a real message still gets through, under all that clock.
    let mixed = browse("io", 16, |i, c| {
        if i >= 2 {
            for _ in 0..PULSES_PER_STEP * 4 {
                midi(c, &[MIDI_CLOCK]);
            }
        }
        if i == 8 {
            midi(c, &[0xB0, 0x75, 0x7F]);
        }
    })
    .0;
    assert_ne!(midi_log(&quiet), midi_log(&mixed), "a message was lost under the clock");
}

#[test]
fn an_unrecognised_midi_message_is_still_shown() {
    // The point of a monitor: something the SDK does not model must still
    // appear, or you can never find out what a button sends.
    let quiet = browse("io", 12, |_, _| {}).0;
    let odd = browse("io", 12, |i, c| {
        if i == 4 {
            midi(c, &[0xD0, 0x40, 0x00]); // channel pressure
        }
    })
    .0;
    assert_ne!(midi_log(&quiet), midi_log(&odd), "an unmodelled message vanished");
}

#[test]
fn the_io_test_stops_when_asked() {
    // A program exits by stopping; it never learns where the menu lives.
    assert!(quits("io"), "the io test did not exit");
}

/// The sequencer's step grid: sixteen cells across, one row per track.
const GRID_LEFT: usize = 46;
const CELL: usize = 12;
const ROW_TOP: usize = 22;
const ROW_HEIGHT: usize = 18;

/// Which step the sequencer is sounding, read off the screen.
///
/// The playing cell is drawn in palette entry 6. Sampling below the note name
/// keeps the glyphs out of the way. Reading the picture rather than poking at
/// guest memory keeps the test honest about what the cartridge actually does.
fn playing_step(console: &Console) -> Option<usize> {
    const PLAYING: u8 = 6;
    let vram = console.vram();
    // The bottom line of the cell, below both the note name and its octave.
    let row = (ROW_TOP + 11) * sgsae::ppu::WIDTH;
    (0..16).find(|i| vram[row + GRID_LEFT + i * CELL + 5] == PLAYING)
}

/// The pixels of one step's cell: the note name and the octave under it.
fn step_cell(console: &Console, track: usize, step: usize) -> Vec<u8> {
    let top = ROW_TOP + track * ROW_HEIGHT;
    let left = GRID_LEFT + step * CELL;
    // The interior only: the edge columns carry the cursor outline, which says
    // nothing about what note is on the step.
    (top..top + 11)
        .flat_map(|y| (left + 1..left + CELL - 2).map(move |x| (y, x)))
        .map(|(y, x)| console.vram()[y * sgsae::ppu::WIDTH + x])
        .collect()
}

/// Voices the FM chip is sounding, as a bitmap.
fn sounding(console: &Console) -> u16 {
    // The PSG channels occupy the low bits of the status word; the FM voices
    // start at FM_BASE, so shift them down to get a bitmap of the FM chip.
    (console.apu.borrow().status() >> sgsae::apu::FM_BASE) as u16
}

/// Send a MIDI packet to the console, as a browser would.
fn midi(console: &Console, bytes: &[u8]) {
    console.midi.borrow_mut().receive_packet(bytes);
}

/// Which track (if any) is showing the "recording" lamp beside its row.
fn armed_track(console: &Console) -> Option<usize> {
    const ARMED: u8 = 7;
    let vram = console.vram();
    (0..4).find(|row| {
        let y = ROW_TOP + row * ROW_HEIGHT + 2;
        let x = GRID_LEFT - 6;
        vram[y * sgsae::ppu::WIDTH + x] == ARMED
    })
}

/// A track's notes, read off the grid as "is this step filled".
fn filled_steps(console: &Console, track: usize) -> Vec<bool> {
    // An empty cell is one flat colour; a filled one carries a note name and
    // an octave in the background colour on top of it.
    (0..16)
        .map(|step| {
            let cell = step_cell(console, track, step);
            cell.iter().any(|p| *p != cell[0])
        })
        .collect()
}

const MIDI_START: u8 = 0xFA;
const MIDI_STOP: u8 = 0xFC;
const MIDI_CLOCK: u8 = 0xF8;
const MIDI_NOTE_ON: u8 = 0x90;
/// Clock pulses in one sixteenth-note step.
const PULSES_PER_STEP: usize = 6;

#[test]
fn the_sequencer_boots_and_shows_a_pattern() {
    let (mut console, _) = browse("seq", 10, |_, _| {});
    assert!(console.console.borrow_mut().take_string().starts_with("fm seq"));
    assert!(playing_step(&console).is_some(), "no step is lit");
}

#[test]
fn the_internal_clock_advances_the_pattern() {
    // Eight frames a step, so twenty-four frames is three steps.
    let (early, _) = browse("seq", 10, |_, _| {});
    let (later, _) = browse("seq", 34, |_, _| {});
    assert_ne!(
        playing_step(&early),
        playing_step(&later),
        "the sequence never moved on its own clock"
    );
}

#[test]
fn the_internal_clock_makes_a_sound() {
    let (console, _) = browse("seq", 60, |_, _| {});
    let peak = console.audio().iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.0, "the sequencer ran silently");
}

#[test]
fn an_external_clock_drives_the_steps() {
    // Start rewinds, then every six pulses is one step. After Start plus four
    // steps' worth of pulses the sequencer should be on step three.
    let (console, _) = browse("seq", 40, |i, c| {
        if i == 5 {
            midi(c, &[MIDI_START]);
        }
        // One step per frame, well faster than the internal clock would run.
        if (6..10).contains(&i) {
            for _ in 0..PULSES_PER_STEP {
                midi(c, &[MIDI_CLOCK]);
            }
        }
    });
    assert_eq!(playing_step(&console), Some(3), "external clock did not advance four steps");
}

#[test]
fn an_external_clock_overrides_the_internal_one() {
    // Once MIDI is driving, the internal timer must not also be stepping —
    // otherwise the pattern runs at both tempos at once.
    let (console, _) = browse("seq", 90, |i, c| {
        if i == 5 {
            midi(c, &[MIDI_START]);
        }
        if i == 10 {
            for _ in 0..PULSES_PER_STEP {
                midi(c, &[MIDI_CLOCK]);
            }
        }
    });
    // One step's worth of pulses, and eighty frames of silence after it.
    assert_eq!(playing_step(&console), Some(0), "the internal clock kept running");
}

#[test]
fn stop_halts_an_externally_clocked_sequence() {
    let (console, _) = browse("seq", 60, |i, c| {
        if i == 5 {
            midi(c, &[MIDI_START]);
        }
        if i == 10 {
            for _ in 0..PULSES_PER_STEP * 2 {
                midi(c, &[MIDI_CLOCK]);
            }
        }
        if i == 15 {
            midi(c, &[MIDI_STOP]);
        }
        // Pulses after a stop must be ignored.
        if i == 20 {
            for _ in 0..PULSES_PER_STEP * 4 {
                midi(c, &[MIDI_CLOCK]);
            }
        }
    });
    // Two steps happened before the stop; nothing after.
    assert_eq!(playing_step(&console), None, "a stopped sequence is not lit");
}

#[test]
fn a_midi_note_is_recorded_into_an_armed_track() {
    // Stopped, recording is step entry: the note lands under the cursor, which
    // starts on step zero. That step's default note is middle C, so playing a
    // different one has to change what is drawn there.
    let arm = |i: u32, c: &mut Console| {
        tap(i, 2, KEY_SPACE, c); // stop, so the playhead cannot move the target
        tap(i, 6, KEY_ENTER, c);
    };
    let before = browse("seq", 16, arm).0;
    let after = browse("seq", 16, |i, c| {
        arm(i, c);
        if i == 10 {
            midi(c, &[MIDI_NOTE_ON, 72, 100]); // C5
        }
    })
    .0;
    assert_ne!(
        step_cell(&before, 0, 0),
        step_cell(&after, 0, 0),
        "the played note was not written into the step"
    );
}

#[test]
fn a_note_is_not_recorded_unless_a_track_is_armed() {
    // Playing along should not quietly rewrite the pattern; that is the whole
    // point of having a record button.
    let quiet = browse("seq", 14, |_, _| {}).0;
    let played = browse("seq", 14, |i, c| {
        if i == 8 {
            midi(c, &[MIDI_NOTE_ON, 72, 100]);
        }
    })
    .0;
    assert_eq!(
        step_cell(&quiet, 0, 0),
        step_cell(&played, 0, 0),
        "an unarmed track recorded the note anyway"
    );
}

#[test]
fn recording_follows_the_selected_row() {
    // Down moves to track two, which starts empty; arming it and playing a
    // note has to fill its first step and leave track one alone.
    let select = |i: u32, c: &mut Console| {
        tap(i, 2, KEY_SPACE, c);
        tap(i, 6, KEY_DOWN, c);
        tap(i, 10, KEY_ENTER, c);
    };
    let before = browse("seq", 20, select).0;
    let after = browse("seq", 20, |i, c| {
        select(i, c);
        if i == 14 {
            midi(c, &[MIDI_NOTE_ON, 72, 100]);
        }
    })
    .0;
    assert_ne!(
        step_cell(&before, 1, 0),
        step_cell(&after, 1, 0),
        "the note did not land on the selected track"
    );
    assert_eq!(
        step_cell(&before, 0, 0),
        step_cell(&after, 0, 0),
        "the note also overwrote the track that was not armed"
    );
}

#[test]
fn step_entry_advances_the_cursor() {
    // Stopped, each note lands on the next step, so a run of four notes fills
    // the first four steps of a track that started empty.
    let console = browse("seq", 32, |i, c| {
        tap(i, 2, KEY_SPACE, c);
        tap(i, 6, KEY_DOWN, c);
        tap(i, 10, KEY_ENTER, c);
        for (n, note) in [60u8, 62, 64, 65].iter().enumerate() {
            if i == 14 + n as u32 * 2 {
                midi(c, &[MIDI_NOTE_ON, *note, 100]);
            }
        }
    })
    .0;

    let filled = filled_steps(&console, 1);
    assert_eq!(filled.iter().filter(|f| **f).count(), 4, "four notes did not fill four steps");
    assert!(filled[..4].iter().all(|f| *f), "the notes were not laid down in order");
}

#[test]
fn live_recording_writes_where_the_playhead_is() {
    // Stopped, recording is step entry at the cursor. Running, a note belongs
    // to the step you played it against, wherever the cursor happens to sit.
    // Track two starts empty, so whatever fills is what was recorded.
    let console = browse("seq", 40, |i, c| {
        tap(i, 2, KEY_DOWN, c); // to track two
        tap(i, 6, KEY_ENTER, c); // arm it
        // Leave the cursor on step 0 and play once, well after the playhead
        // has moved on. The internal clock is eight frames a step.
        if i == 30 {
            midi(c, &[MIDI_NOTE_ON, 72, 100]);
        }
    })
    .0;

    let filled = filled_steps(&console, 1);
    assert!(!filled[0], "the note landed on the cursor, not the playhead");
    assert_eq!(filled.iter().filter(|f| **f).count(), 1, "expected exactly one recorded note");
}

#[test]
fn a_step_passed_without_playing_anything_is_erased() {
    // Live recording replaces rather than overlays. Arming track one and
    // letting the sequence run round without playing has to leave it empty,
    // or you could never take a note back out.
    let before = browse("seq", 8, |_, _| {}).0;
    assert!(filled_steps(&before, 0).iter().any(|f| *f), "the default pattern was empty to begin with");

    // Sixteen steps at eight frames each, plus room to arm first.
    let after = browse("seq", 150, |i, c| tap(i, 2, KEY_ENTER, c)).0;
    assert!(
        !filled_steps(&after, 0).iter().any(|f| *f),
        "recording over the pattern without playing did not clear it"
    );
}

#[test]
fn an_unarmed_track_is_left_alone_while_another_records() {
    // The erase applies to the armed track only; track three's bass line has
    // to survive a pass with track one recording.
    let console = browse("seq", 150, |i, c| tap(i, 2, KEY_ENTER, c)).0;
    assert!(filled_steps(&console, 2).iter().any(|f| *f), "an unarmed track was erased too");
}

/// One of a voice's timbre registers, straight off the chip.
fn voice_reg(console: &Console, voice: u32, reg: u32) -> u32 {
    use sgsae::apu::{Apu, FM_BASE};
    use sgsae::memory::{Device, Width};
    console.apu.borrow_mut().read(Apu::voice(FM_BASE + voice as usize) + reg, Width::Byte)
}

#[test]
fn right_enters_the_grid_and_left_leaves_it() {
    // Up/Down move rows on the track column; once the grid is entered they
    // must stop doing that and transpose the note under the cursor instead.
    // Left at step zero has to hand focus back, so Up moves rows again.
    let console = browse("seq", 30, |i, c| {
        tap(i, 2, KEY_RIGHT, c); // enter the grid on track one, step 0
        tap(i, 6, KEY_LEFT, c); // step 0 already: this leaves the grid
        tap(i, 10, KEY_UP, c); // back on the track column: wraps to POLY
        tap(i, 14, KEY_ENTER, c); // toggles polyphony, arms no track
    })
    .0;
    assert_eq!(armed_track(&console), None, "Left did not return focus to the track column");
}

#[test]
fn up_down_move_rows_only_on_the_track_column() {
    let before = browse("seq", 12, |_, _| {}).0;

    // Up/Down here should just walk the row selector; the note at track
    // one's first step must be untouched.
    let untouched = browse("seq", 16, |i, c| tap(i, 2, KEY_UP, c)).0;
    assert_eq!(
        step_cell(&before, 0, 0),
        step_cell(&untouched, 0, 0),
        "Up changed a note before the grid was entered"
    );

    // Track two, then into its grid: Up now has to transpose, not walk back
    // to track one, so arming afterwards must still land on track two.
    let console = browse("seq", 30, |i, c| {
        tap(i, 2, KEY_DOWN, c); // -> track two
        tap(i, 6, KEY_RIGHT, c); // enter its grid
        tap(i, 10, KEY_UP, c);
        tap(i, 14, KEY_UP, c);
        tap(i, 18, KEY_LEFT, c); // back to the track column
        tap(i, 22, KEY_ENTER, c); // arm whichever row is still selected
    })
    .0;
    assert_eq!(armed_track(&console), Some(1), "Up in the grid moved the row selection");
}

#[test]
fn up_down_transpose_the_note_once_in_the_grid() {
    let before = browse("seq", 12, |_, _| {}).0;
    let after = browse("seq", 16, |i, c| {
        tap(i, 2, KEY_RIGHT, c); // enter the grid on track one, step 0
        tap(i, 6, KEY_UP, c);
    })
    .0;
    assert_ne!(
        step_cell(&before, 0, 0),
        step_cell(&after, 0, 0),
        "Up did not transpose the note under the grid cursor"
    );
}

#[test]
fn a_adds_the_default_note_at_the_grid_cursor() {
    // Track two starts empty; A on an empty step should fill it without any
    // MIDI or keyboard input at all.
    let before = browse("seq", 16, |i, c| {
        tap(i, 2, KEY_DOWN, c);
        tap(i, 6, KEY_RIGHT, c);
    })
    .0;
    assert!(!filled_steps(&before, 1)[0], "test setup: the step should start empty");

    let after = browse("seq", 16, |i, c| {
        tap(i, 2, KEY_DOWN, c);
        tap(i, 6, KEY_RIGHT, c);
        tap(i, 10, KEY_ENTER, c);
    })
    .0;
    assert!(filled_steps(&after, 1)[0], "A did not add a note at the grid cursor");
}

#[test]
fn b_clears_the_note_at_the_grid_cursor() {
    let before = browse("seq", 12, |_, _| {}).0;
    assert!(filled_steps(&before, 0)[0], "test setup: the default pattern should start with a note");

    let after = browse("seq", 16, |i, c| {
        tap(i, 2, KEY_RIGHT, c); // enter the grid on track one, step 0
        tap(i, 6, KEY_BACKSPACE, c);
    })
    .0;
    assert!(!filled_steps(&after, 0)[0], "B/backspace did not clear the grid step");
}

#[test]
fn every_row_has_its_own_tone() {
    // Q steps the modulator ratio of whichever row is selected. Turning it on
    // track two must not touch track one or the pool.
    let before = browse("seq", 12, |_, _| {}).0;
    let after = browse("seq", 20, |i, c| {
        tap(i, 2, KEY_DOWN, c); // to track two
        tap(i, 8, KEY_Q, c);
    })
    .0;

    let ratio = |c: &Console, v: u32| voice_reg(c, v, sgsae::apu::Apu::WAVE);
    assert_ne!(ratio(&before, 1), ratio(&after, 1), "the selected row's tone did not change");
    assert_eq!(ratio(&before, 0), ratio(&after, 0), "a neighbouring track's tone changed too");
    assert_eq!(ratio(&before, 4), ratio(&after, 4), "the polyphony pool's tone changed too");
}

#[test]
fn attack_is_set_per_row_and_starts_instant() {
    // The pool answers a played chord in real time; a slow attack there reads
    // as latency, not a swell, so it has to start at zero like the tracks.
    let before = browse("seq", 12, |_, _| {}).0;
    let attack = |c: &Console, v: u32| voice_reg(c, v, sgsae::apu::Apu::ATTACK);
    for voice in 0..12 {
        assert_eq!(attack(&before, voice), 0, "voice {voice} did not start with an instant attack");
    }

    // R steps the attack of whichever row is selected; on the pool row it
    // must reach every pooled voice and no track.
    let after = browse("seq", 30, |i, c| {
        for n in 0..4 {
            tap(i, 2 + n * 3, KEY_DOWN, c);
        }
        tap(i, 20, KEY_R, c);
    })
    .0;
    for slot in 4..12 {
        assert_ne!(attack(&before, slot), attack(&after, slot), "pool voice {slot} kept its attack");
    }
    assert_eq!(attack(&before, 0), attack(&after, 0), "a track's attack changed with the pool's");
}

#[test]
fn the_polyphony_row_has_a_tone_of_its_own() {
    // The fifth row is a voice like the others: its knobs reach every voice in
    // the pool, and none of the tracks.
    let before = browse("seq", 20, |_, _| {}).0;
    let after = browse("seq", 30, |i, c| {
        for n in 0..4 {
            tap(i, 2 + n * 3, KEY_DOWN, c);
        }
        tap(i, 20, KEY_Q, c);
    })
    .0;

    let ratio = |c: &Console, v: u32| voice_reg(c, v, sgsae::apu::Apu::WAVE);
    for slot in 4..12 {
        assert_ne!(ratio(&before, slot), ratio(&after, slot), "pool voice {slot} kept its tone");
    }
    assert_eq!(ratio(&before, 0), ratio(&after, 0), "a track's tone changed with the pool's");
}

#[test]
fn polyphony_gives_each_note_its_own_voice() {
    // Four rows down is the polyphony switch. With it on, a chord has to reach
    // the pool rather than collapse onto one voice.
    let chord = |poly: bool| {
        browse("seq", 20, |i, c| {
            if poly {
                for n in 0..4 {
                    tap(i, 2 + n * 3, KEY_DOWN, c);
                }
                tap(i, 14, KEY_ENTER, c);
            }
            if i == 18 {
                midi(c, &[MIDI_NOTE_ON, 60, 100]);
                midi(c, &[MIDI_NOTE_ON, 64, 100]);
                midi(c, &[MIDI_NOTE_ON, 67, 100]);
            }
        })
        .0
    };

    // Voices 4 and up are the pool; the tracks below them are always busy.
    let pool = |c: &Console| (sounding(c) >> 4).count_ones();
    assert!(pool(&chord(true)) >= 3, "the chord did not spread across the pool");
}

#[test]
fn releasing_a_polyphonic_note_frees_its_voice() {
    let console = browse("seq", 26, |i, c| {
        for n in 0..4 {
            tap(i, 2 + n * 3, KEY_DOWN, c);
        }
        tap(i, 14, KEY_ENTER, c);
        if i == 18 {
            midi(c, &[MIDI_NOTE_ON, 60, 100]);
            midi(c, &[MIDI_NOTE_ON, 64, 100]);
        }
        if i == 22 {
            // Note-on with zero velocity: the release every keyboard sends.
            midi(c, &[MIDI_NOTE_ON, 60, 0]);
        }
    })
    .0;
    // The released note's voice is gated off; the held one is still ringing.
    let pool = (sounding(&console) >> 4).count_ones();
    assert_eq!(pool, 1, "the released note kept its voice, or took the other one with it");
}

#[test]
fn a_midi_note_sounds_immediately() {
    let quiet = browse("seq", 6, |_, _| {}).0;
    let played = browse("seq", 6, |i, c| {
        if i == 2 {
            midi(c, &[MIDI_NOTE_ON, 64, 110]);
        }
    })
    .0;
    let peak = |c: &Console| c.audio().iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak(&played) > peak(&quiet), "playing a note was silent");
}

#[test]
fn the_sequencer_stops_when_asked() {
    assert!(quits("seq"), "the sequencer did not exit");
}

#[test]
fn a_program_that_stops_hands_the_machine_back() {
    // Escape quits the lander. The supervisor must take over again without
    // anything being fetched — it never left memory.
    let (mut console, opened) = boot(200, |i, c| {
        tap(i, 5, KEY_ENTER, c);
        tap(i, 80, KEY_ESCAPE, c);
    });
    assert_eq!(opened, ["lander"], "the lander should be loaded exactly once");
    assert_eq!(console.mode(), Mode::Supervisor, "the supervisor did not get the machine back");

    let text = console.console.borrow_mut().take_string();
    assert!(text.contains("lunar lander"), "the lander never ran: {text:?}");
    assert!(text.ends_with("loader\n"), "the supervisor did not restart: {text:?}");
}

#[test]
fn the_cursor_returns_to_the_program_that_just_ran() {
    // Open the second entry, quit, then open whatever is under the cursor. It
    // should be the same one, because the address left in the slot says so.
    let (_, opened) = boot(260, |i, c| {
        tap(i, 5, KEY_DOWN, c);
        tap(i, 30, KEY_ENTER, c);
        tap(i, 110, KEY_ESCAPE, c);
        tap(i, 190, KEY_ENTER, c);
    });
    assert_eq!(opened, ["seq", "seq"], "the cursor did not come back to it");
}

#[test]
fn a_deep_link_opens_without_showing_the_menu() {
    // The host puts an address in the slot before anything runs; the
    // supervisor finds it there and goes straight to it.
    let mut console = Console::with_supervisor(&cartridge_named("loader")).expect("supervisor");
    // An address the host has already expanded with its default prefix; the
    // supervisor matches on the name, so the prefix does not confuse it.
    console.set_boot_url("/system/io");

    let mut opened = None;
    for _ in 0..40 {
        match console.advance_frame() {
            Frame::Load(url) => {
                opened = Some(url);
                break;
            }
            Frame::Stopped(outcome) => panic!("stopped: {outcome}"),
            _ => {}
        }
    }
    assert_eq!(opened.as_deref(), Some("io"), "the deep link was not followed");
}

#[test]
fn a_load_that_fails_returns_to_the_supervisor() {
    let mut console = Console::with_supervisor(&cartridge_named("loader")).expect("supervisor");
    let mut asked = false;
    for i in 0..80 {
        tap(i, 5, KEY_ENTER, &mut console);
        match console.advance_frame() {
            Frame::Load(_) => {
                // The host cannot find it.
                asked = true;
                console.load_failed();
            }
            Frame::Stopped(outcome) => panic!("stopped: {outcome}"),
            _ => {}
        }
    }
    assert!(asked, "the supervisor never asked for anything");
    assert_eq!(console.mode(), Mode::Supervisor);
    let text = console.console.borrow_mut().take_string();
    assert!(text.matches("loader").count() >= 2, "the supervisor did not restart: {text:?}");
}

#[test]
fn a_key_held_while_a_program_starts_is_not_a_fresh_press() {
    // Holding the key that opened something must not immediately trip
    // whatever that program has under the same key. The console is never
    // rebuilt now, so the input device keeps its state across the change —
    // this is the test that says so.
    let mut console = Console::with_supervisor(&cartridge_named("loader")).expect("supervisor");
    let mut opened = Vec::new();
    for _ in 0..120 {
        console.input.borrow_mut().set_key(KEY_ENTER, true);
        match console.advance_frame() {
            Frame::Load(url) => {
                opened.push(url.clone());
                let name = url.trim_start_matches('/').trim_end_matches(".elf");
                console.load_program(&cartridge_named(name)).expect("load");
            }
            Frame::Stopped(outcome) => panic!("stopped: {outcome}"),
            _ => {}
        }
    }
    assert_eq!(opened, ["lander"], "a held key opened something more than once");
}

#[test]
fn the_loader_reports_a_file_that_is_not_a_cartridge() {
    assert!(Console::with_cartridge(b"not an elf at all").is_err());
    // A real ELF for the wrong machine is rejected by name, not by crashing.
    let mut file = cartridge();
    file[0x12..0x14].copy_from_slice(&62u16.to_le_bytes()); // EM_X86_64
    let message = match Console::with_cartridge(&file) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("an x86 ELF should not load as a cartridge"),
    };
    assert!(message.contains("not RISC-V"), "{message}");
}
