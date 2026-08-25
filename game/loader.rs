//! The supervisor: the program that decides what else to run.
//!
//! An ordinary program in every respect — same SDK, same linker script — that
//! happens to be the one the console starts with and the one it comes back to.
//! That is the whole operating system: this, one register to start something,
//! and `ebreak` to end it.
//!
//! It keeps no state between runs. Everything it needs to know on entry is in
//! the address buffer and [`Status`]: a fresh boot with an address means the
//! host was deep-linked and that is where to go; a return means a program
//! stopped, and the address says which, so the cursor can go back to it.
//!
//! There is no address bar because there is nothing to type into: every
//! destination is compiled in below, the way a machine of this era would have
//! shipped its menu in ROM.

#![no_std]
#![no_main]

use console::audio::Duty;
use console::input::{Button, Key, Stick};
use console::loader::Status;
use console::video::{Rgb, Video, HEIGHT, WIDTH};
use console::{entry, Console};

const BACKGROUND: u8 = 0;
const FRAME: u8 = 1;
const TITLE: u8 = 2;
const ENTRY: u8 = 3;
const SELECTED: u8 = 4;
const CURSOR: u8 = 5;
const NOTE: u8 = 6;
const ERROR: u8 = 7;

const PALETTE: [Rgb; 8] = [
    Rgb::new(1, 2, 6),
    Rgb::new(6, 10, 18),
    Rgb::new(31, 27, 10),
    Rgb::new(16, 20, 26),
    Rgb::new(30, 31, 31),
    Rgb::new(8, 30, 20),
    Rgb::new(9, 12, 18),
    Rgb::new(31, 8, 8),
];

/// One link on the page.
struct Link {
    name: &'static str,
    blurb: &'static str,
    /// A bare program name. The host decides what that resolves to, which is
    /// why nothing here mentions a prefix or a file extension.
    url: &'static str,
}

/// The whole site. Adding a page means adding a line here and building it.
const LINKS: [Link; 3] = [
    Link { name: "LANDER", blurb: "LAND ON THE PAD", url: "lander" },
    Link { name: "FM SEQ", blurb: "SEQUENCER AND FM SYNTH", url: "seq" },
    Link { name: "IO TEST", blurb: "SHOW ALL INPUT", url: "io" },
];

/// The last segment of an address: `/system/seq` is `seq`.
fn program_name(address: &[u8]) -> &[u8] {
    match address.iter().rposition(|b| *b == b'/') {
        Some(slash) => &address[slash + 1..],
        None => address,
    }
}

/// Which entry, if any, names this program.
///
/// Matched on the name alone, so an address the host has expanded — adding a
/// prefix, say — still finds its way back to the right row.
fn entry_for(address: &[u8]) -> Option<usize> {
    let name = program_name(address);
    LINKS.iter().position(|link| link.url.as_bytes() == name)
}

/// Row spacing in the list.
const ROW: i32 = 14;
const LIST_TOP: i32 = 44;

/// Below this the stick is treated as centred.
const DEADZONE: i8 = 60;

/// Edge-detected direction, so holding the stick scrolls one row, not forty.
#[derive(Default)]
struct Repeat {
    was_up: bool,
    was_down: bool,
}

impl Repeat {
    fn step(&mut self, up: bool, down: bool) -> i32 {
        let moved = i32::from(down && !self.was_down) - i32::from(up && !self.was_up);
        self.was_up = up;
        self.was_down = down;
        moved
    }
}

fn draw_chrome(video: &mut Video, status: Status) {
    video.clear(BACKGROUND);

    // A title bar, because every page has one.
    video.rect(0, 0, WIDTH, 18, FRAME);
    video.text(6, 6, "SGSAE", TITLE);
    video.text(48, 6, "CARTRIDGE INDEX", ENTRY);

    video.rect(0, HEIGHT - 12, WIDTH, 12, FRAME);
    video.text(6, HEIGHT - 8, "UP-DOWN SELECT", NOTE);
    video.text(112, HEIGHT - 8, "A-ENTER OPEN", NOTE);

    match status {
        Status::Failed => video.text(6, 26, "LAST PROGRAM FAILED TO LOAD", ERROR),
        Status::Returned => video.text(6, 26, "SELECT A PROGRAM", NOTE),
        _ => video.text(6, 26, "SELECT A PROGRAM", NOTE),
    }
}

fn draw_links(video: &mut Video, selected: usize) {
    for (i, link) in LINKS.iter().enumerate() {
        let y = LIST_TOP + i as i32 * ROW;
        let chosen = i == selected;
        if chosen {
            video.rect(4, y - 3, WIDTH - 8, 11, FRAME);
            video.text(8, y, ">", CURSOR);
        }
        video.text(16, y, link.name, if chosen { SELECTED } else { ENTRY });
        video.text(16 + 52, y, link.blurb, NOTE);
    }

    // The address of whatever is selected, shown like a status bar.
    let y = LIST_TOP + LINKS.len() as i32 * ROW + 8;
    video.text(6, y, "OPEN", NOTE);
    video.text(28, y, LINKS[selected].url, CURSOR);
}

fn main(mut console: Console) -> ! {
    console.print("loader\n");

    // What just happened is entirely described by the slot: the status says
    // how we got here, the buffer says what was involved.
    let status = console.loader.status();
    let mut address = [0u8; 64];
    let length = console.loader.url(&mut address);
    let address = &address[..length];

    // A fresh boot with an address in the buffer is a deep link: the page was
    // opened pointing at something, so go straight there without a menu.
    if status == Status::Idle && !address.is_empty() {
        if let Some(index) = entry_for(address) {
            console.print("deep link\n");
            console.loader.run(LINKS[index].url);
        }
    }

    console.video.set_palettes(0, &PALETTE);

    console.audio.enable(9);
    console.audio.channel(0).set_volume(8).set_duty(Duty::Quarter).set_decay(9);

    // Coming back from a program puts the cursor on the one that just ran.
    let mut selected = entry_for(address).unwrap_or(0);
    let mut repeat = Repeat::default();

    loop {
        let pad = console.input.gamepad(0);
        let keys = console.input.keys_pressed();
        let stick: Stick = pad.left.with_deadzone(DEADZONE);

        // The pad and stick report a level, so they need edge detection; the
        // keyboard already reports presses. Feeding the keyboard through both
        // would move the cursor two rows for one keystroke.
        let moved = repeat.step(
            pad.is_held(Button::Up) || stick.y < 0,
            pad.is_held(Button::Down) || stick.y > 0,
        ) + i32::from(keys.contains(Key::Down))
            - i32::from(keys.contains(Key::Up));

        if moved != 0 {
            let count = LINKS.len() as i32;
            selected = ((selected as i32 + moved).rem_euclid(count)) as usize;
            console.audio.channel(0).play(880);
        }

        let open = pad.was_pressed(Button::A)
            || pad.was_pressed(Button::Start)
            || keys.contains(Key::Enter)
            || keys.contains(Key::Space);

        draw_chrome(&mut console.video, status);
        draw_links(&mut console.video, selected);

        if open {
            console.audio.channel(0).play(1320);
            console.print("opening ");
            console.print(LINKS[selected].url);
            console.print("\n");
            // Let the confirmation tone start before the machine is replaced.
            console.wait_frame();
            console.loader.run(LINKS[selected].url);
        }

        console.wait_frame();
    }
}

entry!(main);
