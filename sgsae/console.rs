//! The console: a memory map, the devices, and a program in ROM.
//!
//! This is the one definition of what "the machine" is, shared by the CLI and
//! the web build. It owns a [`Machine`] and a handle on every device, and it
//! runs on the audio clock: the host asks for samples with [`Console::fill`],
//! the guest paces itself with `ECALL` — "render this many samples of sound
//! and let me continue", `a0` saying how many, a frame at most — and a video
//! frame falls out every [`SAMPLES_PER_FRAME`] samples. Sound has the hard
//! deadline, so sound is the time base; a game that yields a frame at a time
//! never notices, and a sequencer that yields a few milliseconds at a time
//! hears a note played live within a few milliseconds.
//!
//! A program that stops — deliberately, or by panicking — hands the machine
//! back to the supervisor if there is one, which is the whole of the
//! operating system.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::apu::{self, Apu};
use crate::devices::ConsoleOut;
use crate::elf::{self, ElfError};
use crate::input::Input;
use crate::loader::{Loader, Status};
use crate::machine::{Machine, Trap};
use crate::memory::Bus;
use crate::midi::Midi;
use crate::ppu::{self, Ppu};
use crate::types::Address;

/// Boot/cartridge ROM. Execution starts here.
pub const ROM: Address = 0x0000_0000;
/// Work RAM.
pub const RAM: Address = 0x0200_0000;
/// The cartridge slot: where a guest asks for a different program.
pub const LOADER: Address = 0x0300_0000;
/// Text output, drained by the host.
pub const CONSOLE: Address = 0x0400_0000;
/// Gamepads and keyboard, pushed in by the host.
pub const INPUT: Address = 0x0500_0000;
/// The sound chip.
pub const APU: Address = 0x0600_0000;
/// The display's registers.
pub const PPU: Address = 0x0700_0000;
/// MIDI input.
pub const MIDI: Address = 0x0800_0000;
/// Video RAM: the palette and the pixels, laid out as [`crate::ppu`] says.
pub const VIDEO: Address = 0x0900_0000;

/// Bytes of cartridge ROM. Must match `LENGTH(ROM)` in sdk/console.ld.
pub const ROM_SIZE: usize = 1024 * 1024;
/// Bytes of work RAM. Must match `LENGTH(RAM)` in sdk/console.ld.
pub const RAM_SIZE: usize = 64 * 1024;

/// Audio sample rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// Guest frames per second.
pub const FRAME_RATE: u32 = 60;
/// Samples rendered per `ECALL`.
pub const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE / FRAME_RATE) as usize;

/// Instructions to run between checks for a frame boundary.
const CYCLES_PER_SLICE: u64 = 100_000;
/// Instructions a guest may run *between two yields* before we call it hung.
///
/// This is a deadlock detector, not a limit on how long a program may run: a
/// cartridge that yields every frame can run forever, which is what a game or
/// a sequencer needs to do.
const MAX_CYCLES_PER_YIELD: u64 = 50_000_000;

/// What the machine is currently running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The supervisor: the program that decides what else to run.
    Supervisor,
    /// Something the supervisor started.
    Program,
}

/// Why the machine stopped for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// The program executed `EBREAK`: a deliberate stop, or a panic.
    Breakpoint { pc: Address },
    /// The program faulted.
    Trap(Trap),
    /// No yield in `instructions` instructions: the program is hung.
    Hung { instructions: u64 },
}

impl fmt::Display for Halt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Halt::Breakpoint { pc } => write!(f, "halted at {pc:#010x}"),
            Halt::Trap(trap) => write!(f, "trap: {trap}"),
            Halt::Hung { instructions } => {
                write!(f, "no yield after {instructions} instructions; the guest is hung")
            }
        }
    }
}

/// Why [`Console::advance_frame`] returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A frame was drawn and its audio rendered. Call again.
    Presented,
    /// The supervisor asked the host to fetch and run the program at this
    /// address. Answer with [`Console::load_program`].
    Load(String),
    /// A program stopped and the supervisor took over again. Nothing is
    /// required of the host beyond noticing, but it is the moment to update
    /// whatever is showing the current address.
    Returned,
    /// The machine stopped, for the reason given.
    Stopped(Halt),
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Frame::Presented => write!(f, "stopped at the frame limit"),
            Frame::Load(url) => write!(f, "asked for {url}"),
            Frame::Returned => write!(f, "returned to the supervisor"),
            Frame::Stopped(halt) => write!(f, "{halt}"),
        }
    }
}

/// A wired-up machine plus the host's handles on its devices.
pub struct Console {
    pub machine: Machine,
    pub console: Rc<RefCell<ConsoleOut>>,
    pub input: Rc<RefCell<Input>>,
    pub apu: Rc<RefCell<Apu>>,
    pub loader: Rc<RefCell<Loader>>,
    pub midi: Rc<RefCell<Midi>>,
    pub ppu: Rc<RefCell<Ppu>>,
    audio: Vec<f32>,
    frames: u32,
    /// Samples the guest asked to wait for that have not been rendered yet.
    owed: usize,
    /// Samples until the next video frame.
    until_frame: usize,
    /// Instruction count at the last yield, so the hang detector measures a
    /// single stretch of running rather than the whole session.
    cycles_at_yield: u64,
    /// Instructions allowed between yields before the guest is called hung.
    hang_limit: u64,
    /// Why the machine stopped, if it has.
    halted: Option<Halt>,
    /// A program stopped and the supervisor took over, since the host last
    /// presented a frame.
    returned: bool,
    /// The supervisor, kept so a program that stops can be replaced by it
    /// without going back to the network. This is what makes the menu feel
    /// resident even though it is an ordinary program in the same ROM.
    supervisor: Option<Vec<u8>>,
    mode: Mode,
    /// A load the guest asked for and the host has not acted on yet. The
    /// machine is stopped until it does.
    pending_load: Option<String>,
}

impl Console {
    /// Build the console with a linked cartridge in ROM and nothing else.
    ///
    /// The ELF's entry point becomes the reset vector. With no supervisor
    /// behind it, a program that stops stops the machine.
    pub fn with_cartridge(file: &[u8]) -> Result<Console, ElfError> {
        let mut console = Console::empty();
        console.start(file, Mode::Program)?;
        Ok(console)
    }

    /// Build the console with a supervisor in ROM.
    ///
    /// The supervisor runs first and decides what else to run. A copy is kept,
    /// so when a program stops — deliberately, or by panicking — the
    /// supervisor comes back without anything being fetched.
    pub fn with_supervisor(file: &[u8]) -> Result<Console, ElfError> {
        let mut console = Console::empty();
        console.supervisor = Some(file.to_vec());
        console.start(file, Mode::Supervisor)?;
        Ok(console)
    }

    /// The bare machine: memory mapped, devices wired, ROM blank.
    fn empty() -> Console {
        let console = Rc::new(RefCell::new(ConsoleOut::default()));
        let input = Rc::new(RefCell::new(Input::default()));
        let apu = Rc::new(RefCell::new(Apu::new(SAMPLE_RATE)));
        let ppu = Rc::new(RefCell::new(Ppu::new()));
        let loader = Rc::new(RefCell::new(Loader::new()));
        let midi = Rc::new(RefCell::new(Midi::default()));

        let mut bus = Bus::new();
        bus.map_rom(ROM, vec![0u8; ROM_SIZE]).expect("rom");
        bus.map_ram(RAM, RAM_SIZE).expect("ram");
        bus.map_ram(VIDEO, ppu::VIDEO_SIZE).expect("video ram");
        bus.map_device(CONSOLE, console.clone()).expect("console");
        bus.map_device(INPUT, input.clone()).expect("input");
        bus.map_device(APU, apu.clone()).expect("apu");
        bus.map_device(PPU, ppu.clone()).expect("ppu");
        bus.map_device(LOADER, loader.clone()).expect("loader");
        bus.map_device(MIDI, midi.clone()).expect("midi");

        Console {
            machine: Machine::new(bus),
            console,
            input,
            apu,
            ppu,
            loader,
            midi,
            audio: Vec::new(),
            frames: 0,
            owed: 0,
            until_frame: SAMPLES_PER_FRAME,
            cycles_at_yield: 0,
            hang_limit: MAX_CYCLES_PER_YIELD,
            halted: None,
            returned: false,
            supervisor: None,
            mode: Mode::Program,
            pending_load: None,
        }
    }

    /// Put `file` in ROM and point the machine at its entry.
    fn start(&mut self, file: &[u8], mode: Mode) -> Result<(), ElfError> {
        let entry = elf::load(self.machine.bus_mut(), file)?;
        self.machine.set_pc(entry);
        self.machine.reset_registers();
        self.mode = mode;
        self.cycles_at_yield = self.machine.cycles();
        self.halted = None;
        self.pending_load = None;
        Ok(())
    }

    /// Load a program the supervisor asked for, and run it.
    pub fn load_program(&mut self, file: &[u8]) -> Result<(), ElfError> {
        self.loader.borrow_mut().set_status(Status::Idle);
        self.start(file, Mode::Program)
    }

    /// Report that the program the supervisor asked for could not be loaded,
    /// and hand control back to it.
    pub fn load_failed(&mut self) {
        self.return_to_supervisor(Status::Failed);
    }

    /// Stop whatever is running and give the supervisor the machine.
    ///
    /// A program does not get a say: this is the host deciding, which is what
    /// a browser's back button amounts to.
    pub fn enter_supervisor(&mut self) -> bool {
        if self.mode == Mode::Supervisor {
            return true;
        }
        self.return_to_supervisor(Status::Returned)
    }

    /// Reload the supervisor and give it the machine back.
    fn return_to_supervisor(&mut self, status: Status) -> bool {
        let Some(supervisor) = self.supervisor.clone() else {
            return false;
        };
        // A program is allowed to leave the hardware in any state it likes, so
        // silence anything still sounding before the supervisor draws over it.
        self.apu.borrow_mut().silence();
        if self.start(&supervisor, Mode::Supervisor).is_err() {
            return false;
        }
        self.loader.borrow_mut().set_status(status);
        true
    }

    /// The address the supervisor last asked for, or the boot address.
    pub fn url(&self) -> String {
        self.loader.borrow().url()
    }

    /// Put an address where the supervisor will find it, before anything runs.
    pub fn set_boot_url(&mut self, url: &str) {
        self.loader.borrow_mut().set_url(url);
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Audio rendered so far, as mono samples in `-1.0..=1.0`.
    pub fn audio(&self) -> &[f32] {
        &self.audio
    }

    /// Take the audio rendered so far, leaving the buffer empty.
    ///
    /// A live host drains this every frame; letting it accumulate through a
    /// long session would grow without bound.
    pub fn take_audio(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.audio)
    }

    /// Everything rendered so far, as a 16-bit PCM WAV file.
    pub fn wav(&self) -> Vec<u8> {
        apu::wav(&self.audio, SAMPLE_RATE)
    }

    /// Frames presented so far.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The most recently rendered picture, as RGBA8888.
    pub fn framebuffer(&self) -> std::cell::Ref<'_, Ppu> {
        self.ppu.borrow()
    }

    /// The pixels as the guest wrote them: one palette index each, row-major.
    pub fn vram(&self) -> &[u8] {
        &self.machine.bus().bytes_at(VIDEO + ppu::VRAM).expect("video ram")[..ppu::PIXELS]
    }

    /// How many instructions a guest may run between yields before it is
    /// called hung. Lower it to make a test of the detector cheap.
    pub fn set_hang_limit(&mut self, instructions: u64) {
        self.hang_limit = instructions;
    }

    /// Resolve video RAM into the framebuffer.
    fn draw(&self) {
        let video = self.machine.bus().bytes_at(VIDEO).expect("video ram");
        self.ppu.borrow_mut().render(video);
    }

    /// Render `out` samples of sound, running the guest as far as that takes.
    ///
    /// This is the audio pump and the guest's time base. Video frames fall
    /// out of it every [`SAMPLES_PER_FRAME`] samples. Once the machine has
    /// stopped or asked for a program, the rest of `out` is silence: the
    /// number of samples actually rendered is returned, and the host finds out
    /// why from [`Console::advance_frame`] or [`Console::take_load_request`].
    pub fn fill(&mut self, mut out: &mut [f32]) -> usize {
        let wanted = out.len();
        while !out.is_empty() {
            if self.halted.is_some() || self.pending_load.is_some() {
                out.fill(0.0);
                return wanted - out.len();
            }
            if self.owed == 0 {
                self.run_until_yield();
                continue;
            }
            let n = self.owed.min(self.until_frame).min(out.len());
            let (now, rest) = out.split_at_mut(n);
            self.apu.borrow_mut().render(now);
            out = rest;
            self.owed -= n;
            self.until_frame -= n;
            if self.until_frame == 0 {
                self.draw();
                self.frames += 1;
                self.until_frame = SAMPLES_PER_FRAME;
            }
        }
        wanted
    }

    /// Run the guest until it yields, setting `owed`, or until it stops.
    fn run_until_yield(&mut self) {
        loop {
            match self.machine.run(CYCLES_PER_SLICE) {
                // Not at a yield yet — setup code, for instance.
                Ok(()) => {
                    let instructions = self.machine.cycles() - self.cycles_at_yield;
                    if instructions > self.hang_limit {
                        self.halted = Some(Halt::Hung { instructions });
                        return;
                    }
                }
                Err(Trap::EnvironmentCall { .. }) => {
                    self.cycles_at_yield = self.machine.cycles();
                    // A cartridge that asked for another one stops here: it
                    // has already given up the hardware.
                    if let Some(url) = self.loader.borrow_mut().take_request() {
                        self.pending_load = Some(url);
                        return;
                    }
                    // `a0` says how long to wait for; nothing means a frame.
                    let samples = self.machine.register(10) as usize;
                    self.owed = if samples == 0 { SAMPLES_PER_FRAME } else { samples.min(SAMPLES_PER_FRAME) };
                    return;
                }
                // A program that stops — deliberately or by panicking — hands
                // the machine back. Costing a program nothing to exit is the
                // point: `ebreak` is one instruction and every panic already
                // ends in one, so a broken cartridge returns to the menu
                // instead of taking the console down.
                Err(Trap::Breakpoint { pc }) => {
                    if self.mode == Mode::Program && self.return_to_supervisor(Status::Returned) {
                        self.returned = true;
                        continue;
                    }
                    // Show the final picture rather than leaving a stale one.
                    self.draw();
                    self.halted = Some(Halt::Breakpoint { pc });
                    return;
                }
                Err(trap) => {
                    if self.mode == Mode::Program && self.return_to_supervisor(Status::Failed) {
                        self.returned = true;
                        continue;
                    }
                    self.halted = Some(Halt::Trap(trap));
                    return;
                }
            }
        }
    }

    /// Run for one frame of picture and sound, keeping the sound.
    ///
    /// A convenience over [`Console::fill`] for hosts without an audio clock:
    /// the CLI, and tests. Call [`Console::take_audio`] to drain what it
    /// rendered.
    pub fn advance_frame(&mut self) -> Frame {
        let start = self.audio.len();
        let mut audio = std::mem::take(&mut self.audio);
        audio.resize(start + SAMPLES_PER_FRAME, 0.0);
        let rendered = self.fill(&mut audio[start..]);
        audio.truncate(start + rendered);
        self.audio = audio;

        if let Some(halt) = &self.halted {
            return Frame::Stopped(halt.clone());
        }
        if let Some(url) = &self.pending_load {
            return Frame::Load(url.clone());
        }
        if std::mem::take(&mut self.returned) {
            Frame::Returned
        } else {
            Frame::Presented
        }
    }

    /// Run until the machine stops, asks for a program, or `max_frames` have
    /// been presented — in which case the result is [`Frame::Presented`].
    ///
    /// A game never stops on its own, so batch runs need a limit.
    pub fn run_frames(&mut self, max_frames: u32) -> Frame {
        let until = self.frames.saturating_add(max_frames);
        while self.frames < until {
            match self.advance_frame() {
                Frame::Presented | Frame::Returned => {}
                other => return other,
            }
        }
        Frame::Presented
    }

    /// Take a pending request for a different cartridge, if the guest made one.
    ///
    /// The machine is stopped until the host answers with
    /// [`Console::load_program`] or [`Console::load_failed`]: the cartridge
    /// that asked has already given up the hardware.
    pub fn take_load_request(&mut self) -> Option<String> {
        self.pending_load.clone()
    }

    /// Describe the machine's state, given how the last run ended. Drains the
    /// console's text, so call it once.
    pub fn describe(&mut self, outcome: &Frame) -> String {
        let frames = self.frames;
        let text = self.console.borrow_mut().take_string();
        let registers = self.machine.registers();
        let peak = self.audio.iter().fold(0.0f32, |m, s| m.max(s.abs()));

        format!(
            "{outcome} after {} instructions\n\
             console: {text:?}\n\
             video:   {} frames, {}x{}, {} frames drawn by the ppu\n\
             audio:   {} samples at {SAMPLE_RATE} Hz, peak {peak:.3}\n\
             a0 = {} ({:#010x})   a1 = {:#010x}\n\
             registers: {:#010x?}",
            self.machine.cycles(),
            frames,
            ppu::WIDTH,
            ppu::HEIGHT,
            self.ppu.borrow().frame(),
            self.audio.len(),
            registers[10],
            registers[10],
            registers[11],
            registers,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instructions::Instruction as I;

    /// A console running a hand-assembled program from ROM.
    fn console(program: &[I]) -> Console {
        let image: Vec<u8> = program.iter().flat_map(|i| i.encode().to_le_bytes()).collect();
        let mut c = Console::empty();
        c.machine.bus_mut().load(ROM, &image).unwrap();
        c
    }

    /// Does a little work, yields, and repeats for ever.
    const YIELDING: [I; 5] = [
        I::ADDI { rd: 5, rs1: 0, imm: 40 },
        I::ADDI { rd: 5, rs1: 5, imm: -1 },
        I::BNE { rs1: 5, rs2: 0, offset: -4 },
        I::ECALL,
        I::JAL { rd: 0, offset: -16 },
    ];

    #[test]
    fn a_program_that_yields_may_run_for_ever() {
        // The instruction cap is a deadlock detector, not a session limit: a
        // game or a sequencer runs until someone closes the tab.
        let mut c = console(&YIELDING);
        c.set_hang_limit(2_000);

        // Far more instructions in total than the per-frame allowance.
        for frame in 0..500 {
            assert_eq!(c.advance_frame(), Frame::Presented, "stopped on frame {frame}");
        }
        assert!(c.machine.cycles() > 2_000 * 10, "the test did not exceed one frame's budget");
        assert_eq!(c.frames(), 500);
        assert_eq!(c.ppu.borrow().frame(), 500);
        assert_eq!(c.audio().len(), 500 * SAMPLES_PER_FRAME);
    }

    #[test]
    fn a_guest_that_never_yields_is_called_hung() {
        let mut c = console(&[I::JAL { rd: 0, offset: 0 }]);
        c.set_hang_limit(1_000);
        assert!(matches!(c.advance_frame(), Frame::Stopped(Halt::Hung { .. })));
    }

    #[test]
    fn a_guest_may_yield_for_less_than_a_frame() {
        // Wait 160 samples at a time, counting in t0: five yields to a frame.
        let mut c = console(&[
            I::ADDI { rd: 5, rs1: 5, imm: 1 },
            I::ADDI { rd: 10, rs1: 0, imm: 160 },
            I::ECALL,
            I::JAL { rd: 0, offset: -12 },
        ]);
        assert_eq!(c.advance_frame(), Frame::Presented);
        assert_eq!(c.machine.register(5), 5);
        assert_eq!(c.frames(), 1);
        assert_eq!(c.audio().len(), SAMPLES_PER_FRAME);

        // Asking for more than a frame gets a frame, and so does asking for
        // nothing at all.
        for wait in [2000, 0] {
            let mut c = console(&[
                I::ADDI { rd: 5, rs1: 5, imm: 1 },
                I::ADDI { rd: 10, rs1: 0, imm: wait },
                I::ECALL,
                I::JAL { rd: 0, offset: -12 },
            ]);
            c.advance_frame();
            assert_eq!(c.machine.register(5), 1, "a0 = {wait} should be one yield per frame");
        }
    }

    #[test]
    fn frames_fall_out_of_the_audio_pump() {
        let mut c = console(&YIELDING);
        let mut out = [0.0; 300];
        assert_eq!(c.fill(&mut out), 300);
        assert_eq!(c.fill(&mut out), 300);
        assert_eq!(c.frames(), 0, "600 samples is not yet a frame");
        assert_eq!(c.fill(&mut out[..200]), 200);
        assert_eq!(c.frames(), 1);
        assert_eq!(c.fill(&mut out), 300);
        assert_eq!(c.frames(), 1);
        assert_eq!(c.ppu.borrow().frame(), 1);
    }

    #[test]
    fn a_program_that_stops_shows_its_last_picture() {
        // Paint pixel 0 with palette entry 1, make entry 1 red, then stop.
        let red = ppu::rgb(31, 0, 0) as i32;
        let mut c = console(&[
            I::LUI { rd: 5, imm: VIDEO as i32 },
            I::ADDI { rd: 6, rs1: 0, imm: red },
            I::SH { rs1: 5, rs2: 6, offset: (ppu::PALETTE + 2) as i32 },
            I::ADDI { rd: 6, rs1: 0, imm: 1 },
            I::SB { rs1: 5, rs2: 6, offset: ppu::VRAM as i32 },
            I::EBREAK,
        ]);
        assert_eq!(c.advance_frame(), Frame::Stopped(Halt::Breakpoint { pc: 0x14 }));
        assert_eq!(c.vram()[0], 1);
        assert_eq!(&c.framebuffer().framebuffer()[..4], &[255, 0, 0, 255]);
        assert_eq!(c.frames(), 0, "a halt is not a frame");
        assert_eq!(c.audio().len(), 0, "nor does it produce sound");
        // And the pump stays silent afterwards.
        let mut out = [1.0; 64];
        assert_eq!(c.fill(&mut out), 0);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn run_frames_stops_at_the_limit_and_resumes() {
        let mut c = console(&YIELDING);
        assert_eq!(c.run_frames(10), Frame::Presented);
        assert_eq!(c.frames(), 10);
        assert_eq!(c.run_frames(5), Frame::Presented);
        assert_eq!(c.frames(), 15);
    }

    #[test]
    fn the_wav_export_covers_every_sample() {
        let mut c = console(&YIELDING);
        c.run_frames(3);
        let bytes = c.wav();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + c.audio().len() * 2);
    }

    #[test]
    fn the_report_mentions_what_happened() {
        let mut c = console(&[I::EBREAK]);
        let frame = c.advance_frame();
        let report = c.describe(&frame);
        assert!(report.contains("halted at 0x00000000"), "{report}");
        assert!(report.contains("audio:"), "{report}");
        assert!(report.contains("video:"), "{report}");
    }
}
