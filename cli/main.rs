//! Runs a cartridge and prints what happened.
//!
//! ```sh
//! cargo run --bin cli -- --cartridge game.elf            # a linked cartridge
//! cargo run --bin cli -- --cartridge game.elf --frames 300 --screenshot f.ppm
//! cargo run --bin cli -- --cartridge game.elf --wav tune.wav
//! ```
//!
//! `--supervisor` boots the program that decides what else to run, and
//! `--cartridge-dir` says where to find whatever address it asks for. A program
//! that stops hands the machine back to the supervisor rather than ending the
//! run — which is the whole of the operating system.

use std::process::ExitCode;

use sgsae::console::{Console, Frame};
use sgsae::input::{Button, Pad};

fn usage() -> &'static str {
    "usage: cli (--cartridge PATH | --supervisor PATH) [--cartridge-dir DIR]\n\
     \x20           [--url ADDRESS] [--frames N] [--wav PATH] [--screenshot PATH]\n\
     \x20           [--hold BUTTON,...]"
}

/// Resolve an address the supervisor asked for against a directory on disk.
///
/// The console has no notion of a network; an address is just a name, and this
/// is the host deciding what names mean — the same job the page does when it
/// turns `/system/seq` into a fetch. Only the last component is used, so a
/// program cannot reach outside the directory, and `.elf` is added because an
/// address never carries an extension.
fn resolve(dir: &str, address: &str) -> std::path::PathBuf {
    let name = address.rsplit('/').next().unwrap_or(address);
    std::path::Path::new(dir).join(format!("{name}.elf"))
}

fn main() -> ExitCode {
    let mut cartridge = None;
    let mut supervisor = None;
    let mut boot_url = None;
    let mut cartridge_dir = None;
    let mut held = 0u16;
    let mut wav = None;
    let mut screenshot = None;
    let mut frames = None;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = match flag.as_str() {
            "--cartridge" | "--supervisor" | "--url" | "--cartridge-dir" | "--wav"
            | "--screenshot" | "--frames" | "--hold" => match args.next() {
                Some(value) => value,
                None => {
                    eprintln!("{flag} needs a path\n{}", usage());
                    return ExitCode::FAILURE;
                }
            },
            "-h" | "--help" => {
                println!("{}", usage());
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument {other:?}\n{}", usage());
                return ExitCode::FAILURE;
            }
        };
        match flag.as_str() {
            "--cartridge" => cartridge = Some(value),
            "--supervisor" => supervisor = Some(value),
            "--url" => boot_url = Some(value),
            "--cartridge-dir" => cartridge_dir = Some(value),
            "--hold" => {
                for name in value.split(',') {
                    match Button::ALL.iter().find(|b| format!("{b:?}").eq_ignore_ascii_case(name)) {
                        Some(button) => held |= button.mask(),
                        None => {
                            eprintln!("unknown button {name:?}; try one of {:?}", Button::ALL);
                            return ExitCode::FAILURE;
                        }
                    }
                }
            }
            "--wav" => wav = Some(value),
            "--screenshot" => screenshot = Some(value),
            _ => match value.parse::<u32>() {
                Ok(n) => frames = Some(n),
                Err(_) => {
                    eprintln!("--frames wants a number, got {value:?}");
                    return ExitCode::FAILURE;
                }
            },
        }
    }

    let Some(path) = supervisor.as_ref().or(cartridge.as_ref()) else {
        eprintln!("nothing to run\n{}", usage());
        return ExitCode::FAILURE;
    };
    let file = match std::fs::read(path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let built = if supervisor.is_some() {
        Console::with_supervisor(&file)
    } else {
        Console::with_cartridge(&file)
    };
    let mut console = match built {
        Ok(console) => console,
        Err(e) => {
            eprintln!("cannot load {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // A deep link: the supervisor finds this already in the slot.
    if let Some(url) = &boot_url {
        console.set_boot_url(url);
    }

    // A gamepad is present but nothing is held. `--hold` names buttons to press
    // for the whole run, which is the only way to drive a cartridge headlessly.
    console.input.borrow_mut().set_pad(0, Pad { buttons: held, connected: true, ..Pad::default() });

    // Follow links between cartridges the way the browser does, until one
    // stops or the frame budget runs out.
    let budget = frames.unwrap_or(u32::MAX);
    let mut remaining = budget;
    let outcome = loop {
        let before = console.frames();
        let outcome = console.run_frames(remaining);
        remaining = remaining.saturating_sub(console.frames() - before);

        let Some(url) = console.take_load_request() else {
            break outcome;
        };
        let Some(dir) = &cartridge_dir else {
            eprintln!("no --cartridge-dir to resolve {url} against");
            break outcome;
        };

        let path = resolve(dir, &url);
        println!("{}\n--- loading {url} ---", console.describe(&outcome));
        // The console is not replaced: the frame count, the audio queue and
        // anything held down all carry across because they never moved.
        match std::fs::read(&path) {
            Ok(file) => {
                if let Err(e) = console.load_program(&file) {
                    eprintln!("cannot load {url}: {e}");
                    console.load_failed();
                }
            }
            Err(e) => {
                eprintln!("cannot read {url}: {e}");
                console.load_failed();
            }
        }
        if remaining == 0 {
            break Frame::Presented;
        }
    };
    println!("{}", console.describe(&outcome));

    if let Some(path) = screenshot {
        let image = sgsae::ppu::ppm(console.ppu.borrow().framebuffer());
        if let Err(e) = std::fs::write(&path, &image) {
            eprintln!("cannot write {path}: {e}");
            return ExitCode::FAILURE;
        }
        println!("wrote {path} ({} bytes)", image.len());
    }

    if let Some(path) = wav {
        let bytes = console.wav();
        if let Err(e) = std::fs::write(&path, &bytes) {
            eprintln!("cannot write {path}: {e}");
            return ExitCode::FAILURE;
        }
        println!("\nwrote {path} ({} bytes)", bytes.len());
    }

    ExitCode::SUCCESS
}
