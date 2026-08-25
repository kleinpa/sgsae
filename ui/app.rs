//! wasm entry point for the console.

extern crate sgsae;

use wasm_bindgen::prelude::*;

use sgsae::console::{Console, Frame, Halt, Mode, SAMPLE_RATE};
use sgsae::input::{Pad, PORTS};
use sgsae::ppu;

/// The console, driven one frame at a time by the page.
#[wasm_bindgen]
pub struct Demo {
    console: Console,
    /// A mirror of the PPU's framebuffer. The page reads this directly out of
    /// wasm memory, so it must live somewhere with a stable address rather
    /// than behind the `RefCell` the bus holds the PPU in.
    framebuffer: Vec<u8>,
    outcome: Option<Halt>,
    pending: Option<String>,
    returned: bool,
}

#[wasm_bindgen]
impl Demo {
    /// Boot the console with a supervisor.
    ///
    /// The page fetches the ELF and hands the bytes over, the way a console
    /// reads a cart rather than having the program baked into it. The
    /// supervisor decides what else to run and is what a program returns to
    /// when it stops.
    #[wasm_bindgen(constructor)]
    pub fn new(supervisor: &[u8]) -> Result<Demo, JsError> {
        let console = Console::with_supervisor(supervisor)
            .map_err(|e| JsError::new(&format!("cannot load supervisor: {e}")))?;
        Ok(Demo {
            console,
            framebuffer: vec![0; ppu::WIDTH * ppu::HEIGHT * 4],
            outcome: None,
            pending: None,
            returned: false,
        })
    }

    /// Put an address where the supervisor will find it, before it runs.
    ///
    /// This is how the page's own address bar reaches the guest: a deep link
    /// arrives as data in the slot rather than as a special case in the code.
    #[wasm_bindgen(js_name = setBootUrl)]
    pub fn set_boot_url(&mut self, url: &str) {
        self.console.set_boot_url(url);
    }

    /// The address of whatever is running, for the page to show.
    #[wasm_bindgen(js_name = currentUrl)]
    pub fn current_url(&self) -> String {
        self.console.url()
    }

    /// Whether the supervisor is what is running.
    #[wasm_bindgen(js_name = inSupervisor)]
    pub fn in_supervisor(&self) -> bool {
        self.console.mode() == Mode::Supervisor
    }

    /// Whether a program stopped since this was last asked, handing the
    /// machine back to the supervisor.
    #[wasm_bindgen(js_name = takeReturned)]
    pub fn take_returned(&mut self) -> bool {
        core::mem::take(&mut self.returned)
    }

    pub fn width(&self) -> u32 {
        ppu::WIDTH as u32
    }

    pub fn height(&self) -> u32 {
        ppu::HEIGHT as u32
    }

    /// Report one gamepad, ahead of the next frame.
    ///
    /// `buttons` uses the [`sgsae::input::Button`] bit layout. Sticks are
    /// -127..=127, triggers 0..=255. Ports past the last one are ignored, so a
    /// host can loop over however many pads the browser reports.
    #[wasm_bindgen(js_name = setGamepad)]
    #[allow(clippy::too_many_arguments)]
    pub fn set_gamepad(
        &mut self,
        port: usize,
        connected: bool,
        buttons: u32,
        left_x: i32,
        left_y: i32,
        right_x: i32,
        right_y: i32,
        trigger_l: u32,
        trigger_r: u32,
    ) {
        let axis = |v: i32| v.clamp(-127, 127) as i8;
        let trigger = |v: u32| v.min(255) as u8;
        self.console.input.borrow_mut().set_pad(
            port,
            Pad {
                buttons: buttons as u16,
                left: (axis(left_x), axis(left_y)),
                right: (axis(right_x), axis(right_y)),
                triggers: (trigger(trigger_l), trigger(trigger_r)),
                connected,
            },
        );
    }

    /// Mark every port past `count` as unplugged, so a pad that is disconnected
    /// mid-game stops reading as held.
    #[wasm_bindgen(js_name = clearGamepadsFrom)]
    pub fn clear_gamepads_from(&mut self, count: usize) {
        let mut input = self.console.input.borrow_mut();
        for port in count..PORTS {
            input.set_pad(port, Pad::default());
        }
    }

    /// Deliver one MIDI packet, exactly as it came off the wire.
    ///
    /// Realtime bytes are acted on immediately; everything else queues for the
    /// guest. Packets that are not one to three bytes — a system exclusive
    /// dump — are ignored rather than mangled.
    #[wasm_bindgen(js_name = midiPacket)]
    pub fn midi_packet(&mut self, bytes: &[u8]) {
        self.console.midi.borrow_mut().receive_packet(bytes);
    }

    /// Report whether a MIDI input port is open.
    #[wasm_bindgen(js_name = setMidiConnected)]
    pub fn set_midi_connected(&mut self, connected: bool) {
        self.console.midi.borrow_mut().set_connected(connected);
    }

    /// Report the whole keyboard: a 256-bit map of held keys, a second map of
    /// keys pressed since the last report, and the modifier bits.
    ///
    /// The two maps are not redundant. A key tapped and released between two
    /// frames is in neither the old nor the new held state, so only the second
    /// map can carry it.
    #[wasm_bindgen(js_name = setKeys)]
    pub fn set_keys(&mut self, held: &[u8], pressed: &[u8], modifiers: u32) {
        let bitmap = |slice: &[u8]| {
            let mut map = [0u8; sgsae::input::KEY_BYTES];
            let n = slice.len().min(map.len());
            map[..n].copy_from_slice(&slice[..n]);
            map
        };
        let mut input = self.console.input.borrow_mut();
        input.set_keys(&bitmap(held));
        input.latch_keys(&bitmap(pressed));
        input.set_modifiers(modifiers as u8);
    }

    /// Run until the guest yields, then refresh the framebuffer mirror.
    ///
    /// Returns `true` while the machine is still running.
    pub fn frame(&mut self) -> bool {
        if self.outcome.is_some() || self.pending.is_some() {
            return false;
        }
        match self.console.advance_frame() {
            Frame::Presented => {}
            Frame::Load(url) => self.pending = Some(url),
            Frame::Returned => self.returned = true,
            Frame::Stopped(halt) => self.outcome = Some(halt),
        }
        self.framebuffer.copy_from_slice(self.console.framebuffer().framebuffer());
        self.outcome.is_none() && self.pending.is_none()
    }

    /// The address a cartridge asked the host to load, if it asked.
    ///
    /// The page fetches it and calls [`Demo::swap`]. Until then the machine is
    /// stopped: the cartridge that asked has already given up the hardware.
    #[wasm_bindgen(js_name = takeLoadRequest)]
    pub fn take_load_request(&mut self) -> Option<String> {
        self.pending.take()
    }

    /// Run a program the supervisor asked for.
    ///
    /// The console itself is not replaced, so the frame count, the audio
    /// queue and everything held down on the keyboard carry across unchanged.
    #[wasm_bindgen(js_name = loadProgram)]
    pub fn load_program(&mut self, program: &[u8]) -> Result<(), JsError> {
        self.console
            .load_program(program)
            .map_err(|e| JsError::new(&format!("cannot load program: {e}")))?;
        self.outcome = None;
        self.pending = None;
        Ok(())
    }

    /// Stop whatever is running and hand the machine back to the supervisor.
    ///
    /// The back button uses this: a program does not have to agree to being
    /// left, any more than a web page does.
    #[wasm_bindgen(js_name = returnToSupervisor)]
    pub fn return_to_supervisor(&mut self) {
        self.console.enter_supervisor();
        self.outcome = None;
        self.pending = None;
        self.returned = false;
    }

    /// Report that the program the supervisor asked for could not be fetched,
    /// and hand control back to it.
    #[wasm_bindgen(js_name = reportLoadFailure)]
    pub fn report_load_failure(&mut self) {
        self.console.load_failed();
        self.pending = None;
    }

    /// Take the audio rendered since the last call.
    ///
    /// A game never ends, so the host has to drain this every frame rather
    /// than collecting it for the end.
    #[wasm_bindgen(js_name = takeAudio)]
    pub fn take_audio(&mut self) -> Vec<f32> {
        self.console.take_audio()
    }

    /// Address of the framebuffer inside wasm memory.
    ///
    /// The page wraps this in a `Uint8ClampedArray` rather than copying. Take a
    /// fresh view each frame: growing the wasm heap detaches the old one.
    #[wasm_bindgen(js_name = framebufferPtr)]
    pub fn framebuffer_ptr(&self) -> *const u8 {
        self.framebuffer.as_ptr()
    }

    #[wasm_bindgen(js_name = framebufferLen)]
    pub fn framebuffer_len(&self) -> usize {
        self.framebuffer.len()
    }

    /// What the machine did, once it has stopped. Empty while still running.
    pub fn report(&mut self) -> String {
        let Some(halt) = self.outcome.clone() else {
            return String::new();
        };
        self.console.describe(&Frame::Stopped(halt))
    }

    #[wasm_bindgen(js_name = sampleRate)]
    pub fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }
}
