//! The sound chip: four PSG channels and sixteen FM voices.
//!
//! Channels `0..4` are the PSG — two pulse, a triangle, and noise — reached
//! through [`Audio::channel`]. The FM voices, each one modulator driving one
//! carrier, are reached through [`Audio::fm`]. Every voice has the same
//! controls; only what its timbre registers mean differs.
//!
//! Each voice sustains while its gate is open. Striking a note means setting
//! a pitch and retriggering, which restarts the envelope — that attack is what
//! makes a note sound played rather than switched on.
//!
//! For FM, a whole-number [`Ratio`] sounds like an instrument, because the
//! sidebands land on harmonics of the note. Anything else sounds like a bell
//! or a clang, because they do not.

use crate::map;

/// PSG channels.
pub const CHANNELS: u32 = 4;
/// FM voices.
pub const FM_VOICES: u32 = 16;

const CTRL: usize = 0x00;
const VOLUME: usize = 0x01;
const STATUS: usize = 0x04;
const VOICE_BASE: usize = 0x08;
const VOICE_STRIDE: usize = 0x08;
/// Index of the first FM voice.
const FM_BASE: u32 = CHANNELS;

const FREQ: usize = 0x00;
const VOL: usize = 0x02;
const VOICE_CTRL: usize = 0x03;
const ATTACK: usize = 0x04;
const DECAY: usize = 0x05;
const WAVE: usize = 0x06;
const DEPTH: usize = 0x07;

const GATE: u8 = 0b01;
const RETRIGGER: u8 = 0b10;

/// Pulse duty cycle. Narrower duties sound thinner and more nasal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Duty {
    Eighth = 0,
    Quarter = 1,
    Half = 2,
    ThreeQuarters = 3,
}

/// How fast an FM voice's modulator runs relative to its carrier, in eighths.
///
/// The named values are the ones worth starting from; anything else is fair
/// game, and the ones that are not whole numbers are where the interesting
/// noises live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio(pub u8);

impl Ratio {
    /// Modulator at the carrier's pitch: warm, reedy.
    pub const UNISON: Ratio = Ratio(8);
    /// An octave up: hollow, clarinet-like.
    pub const OCTAVE: Ratio = Ratio(16);
    /// An octave down: fat and buzzy.
    pub const SUB: Ratio = Ratio(4);
    /// Two octaves up: bright, brassy.
    pub const BRIGHT: Ratio = Ratio(32);
    /// Deliberately not a whole number, so the sidebands land off-harmonic.
    pub const BELL: Ratio = Ratio(11);

    /// A ratio of `whole`:1.
    pub const fn whole(whole: u8) -> Ratio {
        Ratio(whole.saturating_mul(8))
    }
}

/// The sound chip.
pub struct Audio {
    _private: (),
}

impl Audio {
    pub(crate) const fn new() -> Audio {
        Audio { _private: () }
    }

    fn reg(&self, offset: usize) -> *mut u8 {
        (map::APU + offset) as *mut u8
    }

    /// Open the master output at `volume` (0..=15). Nothing is audible until
    /// this is called.
    pub fn enable(&mut self, volume: u8) {
        unsafe {
            core::ptr::write_volatile(self.reg(CTRL), 1);
            core::ptr::write_volatile(self.reg(VOLUME), volume.min(15));
        }
    }

    pub fn disable(&mut self) {
        unsafe { core::ptr::write_volatile(self.reg(CTRL), 0) }
    }

    /// Bit *n* set while FM voice *n* is sounding.
    pub fn fm_status(&self) -> u16 {
        let all = unsafe { core::ptr::read_volatile(self.reg(STATUS) as *const u32) };
        (all >> FM_BASE) as u16
    }

    fn voice(&mut self, index: u32) -> Voice<'_> {
        Voice { audio: self, base: VOICE_BASE + index as usize * VOICE_STRIDE }
    }

    /// Borrow a PSG channel. Indices at or above [`CHANNELS`] wrap, so a loop
    /// over channels cannot address hardware that is not there.
    pub fn channel(&mut self, index: u32) -> Voice<'_> {
        self.voice(index % CHANNELS)
    }

    /// Borrow an FM voice. Indices at or above [`FM_VOICES`] wrap.
    pub fn fm(&mut self, index: u32) -> Voice<'_> {
        self.voice(FM_BASE + index % FM_VOICES)
    }
}

/// One voice of the sound chip.
pub struct Voice<'a> {
    audio: &'a mut Audio,
    base: usize,
}

impl Voice<'_> {
    fn write(&mut self, offset: usize, value: u8) {
        unsafe { core::ptr::write_volatile(self.audio.reg(self.base + offset), value) }
    }

    /// Pitch in Hz. On the noise channel this is the rate the shift register
    /// is clocked at, so it sets how coarse the hiss is.
    pub fn set_pitch(&mut self, hz: u16) -> &mut Self {
        let at = self.audio.reg(self.base + FREQ) as *mut u16;
        unsafe { core::ptr::write_volatile(at, hz) }
        self
    }

    pub fn set_volume(&mut self, volume: u8) -> &mut Self {
        self.write(VOL, volume.min(15));
        self
    }

    /// Zero starts at full level; larger values swell in more slowly.
    pub fn set_attack(&mut self, attack: u8) -> &mut Self {
        self.write(ATTACK, attack);
        self
    }

    /// How fast a struck note fades. Zero sustains for as long as the gate is
    /// open; larger values decay faster.
    pub fn set_decay(&mut self, decay: u8) -> &mut Self {
        self.write(DECAY, decay);
        self
    }

    /// Pulse channels only.
    pub fn set_duty(&mut self, duty: Duty) -> &mut Self {
        self.write(WAVE, duty as u8);
        self
    }

    /// Noise channel only: `false` is white, `true` the shorter, almost tonal
    /// rasp.
    pub fn set_tonal_noise(&mut self, tonal: bool) -> &mut Self {
        self.write(WAVE, u8::from(tonal));
        self
    }

    /// FM voices only.
    pub fn set_ratio(&mut self, ratio: Ratio) -> &mut Self {
        self.write(WAVE, ratio.0);
        self
    }

    /// FM voices only: how hard the modulator drives the carrier, 0..=255.
    /// Zero is a plain sine; the timbre opens up as this rises.
    pub fn set_index(&mut self, index: u8) -> &mut Self {
        self.write(DEPTH, index);
        self
    }

    /// Strike a note: set the pitch, restart the envelope, and open the gate.
    pub fn play(&mut self, hz: u16) {
        self.set_pitch(hz);
        self.write(VOICE_CTRL, GATE | RETRIGGER);
    }

    /// Hold the gate open without restarting the envelope — the right thing
    /// for a continuous sound like a thruster.
    pub fn set_gate(&mut self, open: bool) {
        self.write(VOICE_CTRL, if open { GATE } else { 0 });
    }

    pub fn stop(&mut self) {
        self.write(VOICE_CTRL, 0);
    }
}
