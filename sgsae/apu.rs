//! The sound chip: a four-channel PSG and sixteen FM voices behind one
//! register map.
//!
//! Voices `0..4` are the programmable sound generator in the cartridge-era
//! idiom — two pulse voices with selectable duty, a stepped triangle for bass,
//! and an LFSR noise channel for percussion. Voices `4..20` are two-operator
//! FM, each one modulator driving one carrier:
//!
//! ```text
//!   output = sin(carrier_phase + depth * sin(carrier_phase * ratio))
//! ```
//!
//! `ratio` decides the character — a whole number sounds harmonic and
//! instrument-like, anything else sounds like a bell or a clang — and `depth`
//! decides how much of it you hear. The depth follows the envelope, which is
//! what makes an FM note sound plucked rather than merely switched on.
//!
//! Every voice has the same registers and the same envelope; only what `WAVE`
//! and `DEPTH` mean differs by kind. The guest writes registers; the host
//! pulls rendered samples.
//!
//! # Register map
//!
//! | offset | access | register                                          |
//! |--------|--------|---------------------------------------------------|
//! | `0x00` | rw     | `CTRL` — bit 0: master enable                     |
//! | `0x01` | rw     | `VOLUME` — master volume, 0..=15                  |
//! | `0x04` | r      | `STATUS` (u32) — bit *n*: voice *n* is sounding   |
//!
//! Then each voice *n* occupies eight bytes at `0x08 + n * 8`:
//!
//! | offset  | access | register                                                    |
//! |---------|--------|-------------------------------------------------------------|
//! | `+0x00` | rw     | `FREQ` (u16) — pitch in Hz; 0 is silent                     |
//! | `+0x02` | rw     | `VOL` — voice volume, 0..=15                                |
//! | `+0x03` | rw     | `CTRL` — bit 0: gate; bit 1: retrigger (self-clearing)      |
//! | `+0x04` | rw     | `ATTACK` — 0 is instant, larger swells in more slowly       |
//! | `+0x05` | rw     | `DECAY` — 0 sustains, larger fades faster                   |
//! | `+0x06` | rw     | `WAVE` — pulse: duty 0..=3; noise: 0 white, 1 tonal; FM: modulator ratio in eighths |
//! | `+0x07` | rw     | `DEPTH` — FM modulation depth, 0..=255; the PSG ignores it  |
//!
//! Writing the retrigger bit restarts the voice's envelope and phase, which is
//! what gives a note its attack. Without it a gated voice just sustains.
//!
//! # Rendering
//!
//! [`Apu::render`] fills a buffer with mono `f32` samples in `-1.0..=1.0`,
//! which is the shape Web Audio's `AudioWorkletProcessor` wants. Registers are
//! sampled as rendering proceeds, so the guest changes pitch between frames,
//! not between individual samples — plenty for tracker-style music.
//!
//! ```
//! use sgsae::apu::Apu;
//! use sgsae::memory::{Device, Width};
//!
//! let mut apu = Apu::new(48_000);
//! apu.write(Apu::CTRL, Width::Byte, 1);       // master on
//! apu.write(Apu::VOLUME, Width::Byte, 15);
//!
//! let v0 = Apu::voice(0);
//! apu.write(v0 + Apu::FREQ, Width::Half, 440);
//! apu.write(v0 + Apu::VOL, Width::Byte, 12);
//! apu.write(v0 + Apu::VOICE_CTRL, Width::Byte, Apu::GATE | Apu::RETRIGGER);
//!
//! let mut buffer = [0.0; 512];
//! apu.render(&mut buffer);
//! assert!(buffer.iter().any(|s| *s != 0.0));
//! ```

use crate::memory::Device;

/// PSG voices: two pulse, one triangle, one noise.
pub const PSG_VOICES: usize = 4;
/// FM voices.
pub const FM_VOICES: usize = 16;
/// Voices in total.
pub const VOICES: usize = PSG_VOICES + FM_VOICES;
/// Index of the first FM voice.
pub const FM_BASE: usize = PSG_VOICES;

const VOICE_BASE: u32 = 0x08;
const VOICE_STRIDE: u32 = 0x08;

/// Bytes of address space the device occupies.
pub const SIZE: u32 = VOICE_BASE + VOICES as u32 * VOICE_STRIDE;

/// How often envelopes advance, in Hz. Independent of the sample rate so a
/// note decays over the same wall-clock time regardless of output quality.
const ENVELOPE_HZ: u32 = 256;

/// Full-scale envelope level.
const LEVEL_MAX: u16 = 4095;

/// Voices are mixed as if this many were sounding at once.
///
/// Dividing by [`VOICES`] would make a single note almost inaudible, so the
/// scale is set to a realistic chord instead and the sum is clamped. Playing
/// every voice flat out will distort; playing eight will not.
const MIX_SCALE: f32 = 8.0;

/// Phase thresholds for the four pulse duties: 12.5%, 25%, 50%, 75%.
const DUTY: [u32; 4] = [0x2000_0000, 0x4000_0000, 0x8000_0000, 0xC000_0000];

/// Quarter of a sine, at 8-bit resolution. The other three quarters come from
/// symmetry, which is how the real chips did it too.
const SINE_STEPS: usize = 256;

/// A quarter-period sine table, built once.
fn sine_quarter() -> &'static [f32; SINE_STEPS] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[f32; SINE_STEPS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0; SINE_STEPS];
        for (i, slot) in table.iter_mut().enumerate() {
            let turn = (i as f32 + 0.5) / (SINE_STEPS as f32 * 4.0);
            *slot = (turn * core::f32::consts::TAU).sin();
        }
        table
    })
}

/// Sine of a Q32 phase, where a full turn is 2^32.
fn sine(phase: u32) -> f32 {
    let table = sine_quarter();
    // Top two bits select the quadrant; the next eight index the table.
    let quadrant = phase >> 30;
    let index = ((phase >> 22) & 0xFF) as usize;
    match quadrant {
        0 => table[index],
        1 => table[SINE_STEPS - 1 - index],
        2 => -table[index],
        _ => -table[SINE_STEPS - 1 - index],
    }
}

/// What a voice sounds like. Fixed per voice, as on real hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Square wave with selectable duty.
    Pulse,
    /// Stepped triangle — quantised to 16 levels, which is what gives the
    /// classic buzzy bass rather than a clean triangle.
    Triangle,
    /// Pseudo-random noise from a 15-bit LFSR.
    Noise,
    /// One modulator driving one carrier.
    Fm,
}

/// The kind of voice `n`.
pub const fn kind(n: usize) -> Kind {
    match n {
        0 | 1 => Kind::Pulse,
        2 => Kind::Triangle,
        3 => Kind::Noise,
        _ => Kind::Fm,
    }
}

/// A linear attack to full scale, then a linear decay.
#[derive(Debug, Clone, Copy)]
struct Envelope {
    level: u16,
    /// Rising while the note attacks, falling once it has peaked.
    attacking: bool,
}

impl Envelope {
    fn restart(&mut self, attack: u8) {
        self.attacking = attack > 0;
        self.level = if self.attacking { 0 } else { LEVEL_MAX };
    }

    fn tick(&mut self, attack: u8, decay: u8) {
        if self.attacking {
            self.level = self.level.saturating_add(u16::from(attack) * 16);
            if self.level >= LEVEL_MAX {
                self.level = LEVEL_MAX;
                self.attacking = false;
            }
        } else if decay != 0 {
            self.level = self.level.saturating_sub(u16::from(decay) * 16);
        }
    }

    fn gain(&self) -> f32 {
        f32::from(self.level) / f32::from(LEVEL_MAX)
    }
}

#[derive(Debug, Clone, Copy)]
struct Voice {
    kind: Kind,
    // Registers.
    freq: u16,
    volume: u8,
    gate: bool,
    attack: u8,
    decay: u8,
    wave: u8,
    depth: u8,
    // Running state.
    envelope: Envelope,
    /// Oscillator phase, or the carrier's for FM. Q32: a full turn is 2^32.
    phase: u32,
    modulator: u32,
    lfsr: u16,
}

impl Voice {
    fn new(kind: Kind) -> Voice {
        Voice {
            kind,
            freq: 0,
            volume: 0,
            gate: false,
            attack: 0,
            decay: 0,
            wave: 0,
            depth: 0,
            envelope: Envelope { level: LEVEL_MAX, attacking: false },
            phase: 0,
            modulator: 0,
            lfsr: 1,
        }
    }

    fn sounding(&self) -> bool {
        self.gate && self.freq != 0 && self.volume > 0 && self.envelope.level > 0
    }

    /// Restart the note: envelope from the top, phase and noise state reset.
    fn retrigger(&mut self) {
        self.phase = 0;
        self.modulator = 0;
        self.lfsr = 1;
        self.envelope.restart(self.attack);
    }

    /// Advance one sample and return its amplitude in `-1.0..=1.0`.
    fn sample(&mut self, sample_rate: u32) -> f32 {
        if !self.sounding() {
            return 0.0;
        }

        // The phase increment is exact for any frequency/sample-rate pair
        // without accumulating float error.
        let step = ((u64::from(self.freq) << 32) / u64::from(sample_rate)) as u32;
        let previous = self.phase;
        self.phase = self.phase.wrapping_add(step);

        let level = match self.kind {
            Kind::Pulse => {
                if self.phase < DUTY[usize::from(self.wave & 0b11)] {
                    1.0
                } else {
                    -1.0
                }
            }
            Kind::Triangle => {
                // Fold the top 5 bits of phase into a 16-step ramp up and down.
                let step = self.phase >> 27;
                let folded = if step < 16 { step } else { 31 - step };
                folded as f32 / 7.5 - 1.0
            }
            Kind::Noise => {
                // The LFSR advances once per cycle of FREQ, so FREQ sets the
                // noise's pitch rather than a waveform frequency.
                if self.phase < previous {
                    self.clock_lfsr();
                }
                if self.lfsr & 1 == 0 {
                    1.0
                } else {
                    -1.0
                }
            }
            Kind::Fm => {
                // The ratio is in eighths, so 8 is a 1:1 modulator.
                let modulator_step =
                    (u64::from(step) * u64::from(self.wave) / 8).min(u64::from(u32::MAX)) as u32;
                self.modulator = self.modulator.wrapping_add(modulator_step);
                // The depth follows the envelope, so the timbre opens and
                // closes with the note. A static depth sounds like an organ.
                let depth = f32::from(self.depth) / 32.0 * self.envelope.gain();
                let offset = depth * sine(self.modulator);
                sine(self.phase.wrapping_add((offset * (u32::MAX as f32 / 8.0)) as i32 as u32))
            }
        };

        level * (f32::from(self.volume) / 15.0) * self.envelope.gain()
    }

    /// 15-bit Fibonacci LFSR. The short tap gives a periodic, almost tonal
    /// rasp; the long one gives white noise.
    fn clock_lfsr(&mut self) {
        let tap = if self.wave & 1 != 0 { 6 } else { 1 };
        let bit = (self.lfsr ^ (self.lfsr >> tap)) & 1;
        self.lfsr = (self.lfsr >> 1) | (bit << 14);
    }
}

/// The sound chip.
#[derive(Debug, Clone)]
pub struct Apu {
    voices: [Voice; VOICES],
    sample_rate: u32,
    master_volume: u8,
    enabled: bool,
    /// Fractional accumulator for the envelope clock.
    envelope_clock: u32,
}

impl Apu {
    // Global registers.
    pub const CTRL: u32 = 0x00;
    pub const VOLUME: u32 = 0x01;
    pub const STATUS: u32 = 0x04;

    // Per-voice register offsets, relative to `voice(n)`.
    pub const FREQ: u32 = 0x00;
    pub const VOL: u32 = 0x02;
    pub const VOICE_CTRL: u32 = 0x03;
    pub const ATTACK: u32 = 0x04;
    pub const DECAY: u32 = 0x05;
    pub const WAVE: u32 = 0x06;
    pub const DEPTH: u32 = 0x07;

    /// Bit in a voice's `CTRL` that opens the gate.
    pub const GATE: u32 = 0b01;
    /// Bit in a voice's `CTRL` that restarts the note. Reads back as 0.
    pub const RETRIGGER: u32 = 0b10;

    /// Offset of voice `n`'s register block.
    pub const fn voice(n: usize) -> u32 {
        VOICE_BASE + n as u32 * VOICE_STRIDE
    }

    pub fn new(sample_rate: u32) -> Apu {
        assert!(sample_rate > 0, "sample rate must be positive");
        Apu {
            voices: std::array::from_fn(|n| Voice::new(kind(n))),
            sample_rate,
            master_volume: 0,
            enabled: false,
            envelope_clock: 0,
        }
    }

    /// Bit *n* is set while voice *n* is producing sound.
    pub fn status(&self) -> u32 {
        self.voices.iter().enumerate().filter(|(_, v)| v.sounding()).map(|(i, _)| 1 << i).sum()
    }

    /// Silence every voice and close the master output.
    ///
    /// Used when one program replaces another: whatever was ringing belonged
    /// to the program that is gone.
    pub fn silence(&mut self) {
        for voice in &mut self.voices {
            voice.gate = false;
            voice.envelope.level = 0;
        }
        self.enabled = false;
    }

    /// Fill `out` with mono samples, advancing the chip by `out.len()` frames.
    pub fn render(&mut self, out: &mut [f32]) {
        let master = if self.enabled { f32::from(self.master_volume) / 15.0 } else { 0.0 };
        if master == 0.0 {
            out.fill(0.0);
            return;
        }

        for slot in out.iter_mut() {
            // Advance envelopes on their own clock, not per sample.
            self.envelope_clock += ENVELOPE_HZ;
            if self.envelope_clock >= self.sample_rate {
                self.envelope_clock -= self.sample_rate;
                for voice in &mut self.voices {
                    voice.envelope.tick(voice.attack, voice.decay);
                }
            }

            let mix: f32 = self.voices.iter_mut().map(|v| v.sample(self.sample_rate)).sum();
            *slot = (mix / MIX_SCALE).clamp(-1.0, 1.0) * master;
        }
    }

    /// Split an offset into a voice index and a register within it.
    fn locate(offset: u32) -> Option<(usize, u32)> {
        let relative = offset.checked_sub(VOICE_BASE)?;
        let index = (relative / VOICE_STRIDE) as usize;
        (index < VOICES).then_some((index, relative % VOICE_STRIDE))
    }
}

impl Device for Apu {
    fn size(&self) -> u32 {
        SIZE
    }

    fn read_byte(&mut self, offset: u32) -> u8 {
        match offset {
            Self::CTRL => u8::from(self.enabled),
            Self::VOLUME => self.master_volume,
            0x04..=0x07 => self.status().to_le_bytes()[(offset - Self::STATUS) as usize],
            _ => {
                let Some((index, register)) = Self::locate(offset) else { return 0 };
                let voice = &self.voices[index];
                match register {
                    Self::FREQ => voice.freq as u8,
                    0x01 => (voice.freq >> 8) as u8,
                    Self::VOL => voice.volume,
                    // The retrigger bit is a strobe, so it never reads back.
                    Self::VOICE_CTRL => u8::from(voice.gate),
                    Self::ATTACK => voice.attack,
                    Self::DECAY => voice.decay,
                    Self::WAVE => voice.wave,
                    _ => voice.depth,
                }
            }
        }
    }

    fn write_byte(&mut self, offset: u32, value: u8) {
        match offset {
            Self::CTRL => self.enabled = value & 1 != 0,
            Self::VOLUME => self.master_volume = value.min(15),
            _ => {
                let Some((index, register)) = Self::locate(offset) else { return };
                let voice = &mut self.voices[index];
                match register {
                    Self::FREQ => voice.freq = (voice.freq & 0xFF00) | u16::from(value),
                    0x01 => voice.freq = (voice.freq & 0x00FF) | (u16::from(value) << 8),
                    Self::VOL => voice.volume = value.min(15),
                    Self::VOICE_CTRL => {
                        voice.gate = u32::from(value) & Self::GATE != 0;
                        if u32::from(value) & Self::RETRIGGER != 0 {
                            voice.retrigger();
                        }
                    }
                    Self::ATTACK => voice.attack = value,
                    Self::DECAY => voice.decay = value,
                    Self::WAVE => voice.wave = value,
                    _ => voice.depth = value,
                }
            }
        }
    }
}

/// Encode mono `f32` samples as a 16-bit PCM WAV file.
///
/// Handy for listening to what the guest played without wiring up an audio
/// backend. Samples outside `-1.0..=1.0` are clamped.
pub fn wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
    fmt.extend_from_slice(&sample_rate.to_le_bytes());
    fmt.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // bytes per second
    fmt.extend_from_slice(&2u16.to_le_bytes()); // block align
    fmt.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    let mut data = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        data.extend_from_slice(&((clamped * f32::from(i16::MAX)) as i16).to_le_bytes());
    }

    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    // Everything after this field: "WAVE" plus both chunks with their headers.
    out.extend_from_slice(&((4 + 8 + fmt.len() + 8 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    for (id, body) in [(b"fmt ", &fmt), (b"data", &data)] {
        out.extend_from_slice(id);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Width;

    const RATE: u32 = 48_000;
    const FM0: usize = FM_BASE;

    /// A chip with master output open and one voice struck at `freq`.
    fn playing(voice: usize, freq: u16, wave: u8, depth: u8) -> Apu {
        let mut a = Apu::new(RATE);
        a.write(Apu::CTRL, Width::Byte, 1);
        a.write(Apu::VOLUME, Width::Byte, 15);
        let base = Apu::voice(voice);
        a.write(base + Apu::FREQ, Width::Half, u32::from(freq));
        a.write(base + Apu::VOL, Width::Byte, 15);
        a.write(base + Apu::WAVE, Width::Byte, u32::from(wave));
        a.write(base + Apu::DEPTH, Width::Byte, u32::from(depth));
        a.write(base + Apu::VOICE_CTRL, Width::Byte, Apu::GATE | Apu::RETRIGGER);
        a
    }

    fn render(a: &mut Apu, samples: usize) -> Vec<f32> {
        let mut buffer = vec![0.0; samples];
        a.render(&mut buffer);
        buffer
    }

    /// Count sign changes, which for a periodic wave is twice the frequency.
    fn zero_crossings(buffer: &[f32]) -> usize {
        buffer.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    }

    fn peak(buffer: &[f32]) -> f32 {
        buffer.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn voices_have_the_documented_kinds() {
        assert_eq!(kind(0), Kind::Pulse);
        assert_eq!(kind(1), Kind::Pulse);
        assert_eq!(kind(2), Kind::Triangle);
        assert_eq!(kind(3), Kind::Noise);
        for n in FM_BASE..VOICES {
            assert_eq!(kind(n), Kind::Fm);
        }
    }

    #[test]
    fn silent_until_master_and_gate_are_both_open() {
        let mut a = Apu::new(RATE);
        assert!(render(&mut a, 256).iter().all(|x| *x == 0.0));

        // Gate a voice but leave the master off.
        let base = Apu::voice(0);
        a.write(base + Apu::FREQ, Width::Half, 440);
        a.write(base + Apu::VOL, Width::Byte, 15);
        a.write(base + Apu::VOICE_CTRL, Width::Byte, Apu::GATE);
        assert!(render(&mut a, 256).iter().all(|x| *x == 0.0));

        a.write(Apu::CTRL, Width::Byte, 1);
        a.write(Apu::VOLUME, Width::Byte, 15);
        assert!(render(&mut a, 256).iter().any(|x| *x != 0.0));
    }

    #[test]
    fn a_pulse_voice_oscillates_at_the_requested_pitch() {
        // One second at 440 Hz should cross zero twice per cycle.
        let mut a = playing(0, 440, 2, 0);
        let crossings = zero_crossings(&render(&mut a, RATE as usize));
        assert!((crossings as i32 - 880).abs() <= 2, "got {crossings} crossings, want ~880");
    }

    #[test]
    fn duty_changes_the_proportion_of_the_cycle_spent_high() {
        for (wave, expected) in [(0u8, 0.125), (1, 0.25), (2, 0.5), (3, 0.75)] {
            let mut a = playing(0, 100, wave, 0);
            let buffer = render(&mut a, RATE as usize);
            let high = buffer.iter().filter(|x| **x > 0.0).count() as f32 / buffer.len() as f32;
            assert!((high - expected).abs() < 0.01, "duty {wave}: {high} high, want {expected}");
        }
    }

    #[test]
    fn the_triangle_voice_ramps_rather_than_switching() {
        let mut a = playing(2, 100, 0, 0);
        let buffer = render(&mut a, 2000);
        // A stepped triangle visits many distinct levels; a square visits two.
        let mut levels: Vec<i32> = buffer.iter().map(|x| (x * 10_000.0) as i32).collect();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() > 8, "triangle only produced {} levels", levels.len());
    }

    #[test]
    fn the_noise_voice_is_aperiodic_but_deterministic() {
        let first = render(&mut playing(3, 8000, 0, 0), 4096);
        let again = render(&mut playing(3, 8000, 0, 0), 4096);
        assert_eq!(first, again, "noise must be reproducible for the same setup");

        // White noise should not repeat within a short window the way a
        // periodic waveform does.
        let period = 480; // one cycle of a 100 Hz tone at 48 kHz
        let repeats =
            first[..period].iter().zip(&first[period..period * 2]).filter(|(a, b)| a == b).count();
        assert!(repeats < period, "noise repeated exactly over one period");

        // The short tap is meant to sound different from the long one.
        let tonal = render(&mut playing(3, 8000, 1, 0), 4096);
        assert_ne!(first, tonal, "white and tonal noise modes are identical");
    }

    #[test]
    fn an_fm_voice_oscillates_at_its_carrier_pitch() {
        // With no modulation the output is a plain sine, so zero crossings
        // count the frequency directly.
        let mut a = playing(FM0, 440, 8, 0);
        let crossings = zero_crossings(&render(&mut a, RATE as usize));
        assert!((crossings as i32 - 880).abs() <= 4, "got {crossings}, want ~880");
    }

    #[test]
    fn modulation_adds_harmonics_without_moving_the_pitch() {
        // The whole point of FM: the note stays where it is, but its timbre
        // gains content that a pure sine does not have.
        let a = render(&mut playing(FM0, 220, 8, 0), 4096);
        let b = render(&mut playing(FM0, 220, 8, 200), 4096);
        assert_ne!(a, b, "the depth made no difference");
        // A modulated wave changes direction more often than a sine does.
        let turns = |s: &[f32]| {
            s.windows(3).filter(|w| (w[1] - w[0]).signum() != (w[2] - w[1]).signum()).count()
        };
        assert!(turns(&b) > turns(&a), "modulation did not add harmonics");
    }

    #[test]
    fn the_ratio_changes_the_timbre() {
        let harmonic = render(&mut playing(FM0, 220, 16, 128), 2048);
        let clangorous = render(&mut playing(FM0, 220, 11, 128), 2048);
        assert_ne!(harmonic, clangorous, "the ratio had no effect");
    }

    #[test]
    fn decay_fades_a_note_and_zero_sustains_it() {
        // With no decay the amplitude is unchanged a second later.
        let mut sustained = playing(0, 440, 2, 0);
        let start = peak(&render(&mut sustained, 4800));
        let later = peak(&render(&mut sustained, 4800));
        assert!((start - later).abs() < 1e-6, "sustained note changed: {start} -> {later}");

        // With decay it fades to nothing. Measure after the fade has had time
        // to finish, not across it, or the window still holds the attack.
        let mut plucked = playing(0, 440, 2, 0);
        plucked.write(Apu::voice(0) + Apu::DECAY, Width::Byte, 8);
        let attack = peak(&render(&mut plucked, 2400));
        render(&mut plucked, RATE as usize);
        let tail = peak(&render(&mut plucked, 2400));
        assert!(tail < attack, "decaying note did not fade: {attack} -> {tail}");
        assert_eq!(tail, 0.0, "decaying note never reached silence");
        assert_eq!(plucked.status() & 1, 0);
    }

    #[test]
    fn an_attack_ramps_the_note_up_on_any_voice() {
        for voice in [0, FM0] {
            let mut a = playing(voice, 440, 8, 64);
            a.write(Apu::voice(voice) + Apu::ATTACK, Width::Byte, 8);
            a.write(Apu::voice(voice) + Apu::VOICE_CTRL, Width::Byte, Apu::GATE | Apu::RETRIGGER);
            let start = peak(&render(&mut a, 400));
            let later = peak(&render(&mut a, 4000));
            assert!(later > start, "voice {voice} did not swell: {start} -> {later}");
        }
    }

    #[test]
    fn retrigger_restarts_the_envelope() {
        let base = Apu::voice(0);
        let mut a = playing(0, 440, 2, 0);
        a.write(base + Apu::DECAY, Width::Byte, 16);
        render(&mut a, RATE as usize);
        assert_eq!(a.status() & 1, 0, "note should have decayed to silence");

        a.write(base + Apu::VOICE_CTRL, Width::Byte, Apu::GATE | Apu::RETRIGGER);
        assert_eq!(a.status() & 1, 1, "retrigger should revive the voice");
        assert!(render(&mut a, 256).iter().any(|x| *x != 0.0));
    }

    #[test]
    fn every_voice_at_once_stays_in_range_and_shows_in_status() {
        let mut a = Apu::new(RATE);
        a.write(Apu::CTRL, Width::Byte, 1);
        a.write(Apu::VOLUME, Width::Byte, 15);
        for n in 0..VOICES {
            let base = Apu::voice(n);
            a.write(base + Apu::FREQ, Width::Half, 110 + n as u32 * 37);
            a.write(base + Apu::VOL, Width::Byte, 15);
            a.write(base + Apu::WAVE, Width::Byte, 8);
            a.write(base + Apu::DEPTH, Width::Byte, 255);
            a.write(base + Apu::VOICE_CTRL, Width::Byte, Apu::GATE | Apu::RETRIGGER);
        }
        let buffer = render(&mut a, RATE as usize / 4);
        assert!(buffer.iter().all(|x| x.is_finite()), "the mix produced a non-number");
        assert!(buffer.iter().all(|x| (-1.0..=1.0).contains(x)), "the mix left the valid range");
        assert_eq!(a.status(), (1 << VOICES) - 1, "every voice should be sounding");
        assert_eq!(a.read(Apu::STATUS, Width::Word), (1 << VOICES) - 1);
    }

    #[test]
    fn silence_closes_everything() {
        let mut a = playing(FM0, 440, 8, 0);
        assert_ne!(a.status(), 0);
        a.silence();
        assert_eq!(a.status(), 0);
        assert!(render(&mut a, 256).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn envelope_timing_is_independent_of_sample_rate() {
        // The same note should take the same wall-clock time to fade at any rate.
        let fade_time = |rate: u32| {
            let mut a = Apu::new(rate);
            a.write(Apu::CTRL, Width::Byte, 1);
            a.write(Apu::VOLUME, Width::Byte, 15);
            let base = Apu::voice(0);
            a.write(base + Apu::FREQ, Width::Half, 440);
            a.write(base + Apu::VOL, Width::Byte, 15);
            a.write(base + Apu::DECAY, Width::Byte, 1);
            a.write(base + Apu::VOICE_CTRL, Width::Byte, Apu::GATE | Apu::RETRIGGER);

            let mut elapsed = 0usize;
            let mut buffer = [0.0; 64];
            while a.status() & 1 != 0 && elapsed < rate as usize * 10 {
                a.render(&mut buffer);
                elapsed += buffer.len();
            }
            elapsed as f32 / rate as f32
        };

        let (a, b) = (fade_time(48_000), fade_time(22_050));
        assert!((a - b).abs() < 0.05, "fade took {a}s at 48k but {b}s at 22.05k");
    }

    // -- register behaviour --------------------------------------------------

    #[test]
    fn registers_read_back_what_was_written() {
        let mut a = Apu::new(RATE);
        let base = Apu::voice(FM0 + 2);
        a.write(Apu::VOLUME, Width::Byte, 9);
        a.write(base + Apu::FREQ, Width::Half, 1234);
        a.write(base + Apu::VOL, Width::Byte, 7);
        a.write(base + Apu::ATTACK, Width::Byte, 3);
        a.write(base + Apu::DECAY, Width::Byte, 42);
        a.write(base + Apu::WAVE, Width::Byte, 24);
        a.write(base + Apu::DEPTH, Width::Byte, 200);
        a.write(base + Apu::VOICE_CTRL, Width::Byte, Apu::GATE | Apu::RETRIGGER);

        assert_eq!(a.read(Apu::VOLUME, Width::Byte), 9);
        assert_eq!(a.read(base + Apu::FREQ, Width::Half), 1234);
        assert_eq!(a.read(base + Apu::VOL, Width::Byte), 7);
        assert_eq!(a.read(base + Apu::ATTACK, Width::Byte), 3);
        assert_eq!(a.read(base + Apu::DECAY, Width::Byte), 42);
        assert_eq!(a.read(base + Apu::WAVE, Width::Byte), 24);
        assert_eq!(a.read(base + Apu::DEPTH, Width::Byte), 200);
        // The retrigger bit is a strobe and never reads back.
        assert_eq!(a.read(base + Apu::VOICE_CTRL, Width::Byte), Apu::GATE);
    }

    #[test]
    fn a_byte_at_a_time_write_matches_a_halfword_write() {
        let base = Apu::voice(0);
        let mut halfword = Apu::new(RATE);
        halfword.write(base + Apu::FREQ, Width::Half, 0xABCD);

        let mut bytes = Apu::new(RATE);
        bytes.write(base + Apu::FREQ, Width::Byte, 0xCD);
        bytes.write(base + Apu::FREQ + 1, Width::Byte, 0xAB);

        assert_eq!(
            halfword.read(base + Apu::FREQ, Width::Half),
            bytes.read(base + Apu::FREQ, Width::Half)
        );
    }

    #[test]
    fn volume_registers_saturate_rather_than_wrapping() {
        let mut a = Apu::new(RATE);
        a.write(Apu::VOLUME, Width::Byte, 200);
        assert_eq!(a.read(Apu::VOLUME, Width::Byte), 15);
        a.write(Apu::voice(5) + Apu::VOL, Width::Byte, 99);
        assert_eq!(a.read(Apu::voice(5) + Apu::VOL, Width::Byte), 15);
    }

    #[test]
    fn every_voice_block_is_distinct_and_inside_the_device() {
        for n in 0..VOICES {
            assert_eq!(Apu::voice(n), 0x08 + n as u32 * 8);
            assert!(Apu::voice(n) + Apu::DEPTH < SIZE);
        }
        // Voices 0 and 19 are independent.
        let mut a = Apu::new(RATE);
        a.write(Apu::voice(VOICES - 1) + Apu::FREQ, Width::Half, 440);
        assert_eq!(a.read(Apu::voice(0) + Apu::FREQ, Width::Half), 0);
    }

    // -- wav -----------------------------------------------------------------

    #[test]
    fn wav_has_a_well_formed_header() {
        let bytes = wav(&[0.0, 1.0, -1.0, 0.5], 48_000);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(bytes.len(), 44 + 4 * 2);

        // The declared RIFF size must match the bytes that follow it.
        let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(declared, bytes.len() - 8);

        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 48_000);
    }

    #[test]
    fn samples_are_clamped_not_wrapped() {
        let bytes = wav(&[2.0, -2.0], 8000);
        let first = i16::from_le_bytes(bytes[44..46].try_into().unwrap());
        let second = i16::from_le_bytes(bytes[46..48].try_into().unwrap());
        assert_eq!(first, i16::MAX);
        assert_eq!(second, -i16::MAX);
    }
}
