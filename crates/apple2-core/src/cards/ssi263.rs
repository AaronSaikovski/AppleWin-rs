//! SSI263 phoneme-based speech synthesizer emulation.
//!
//! Used in Mockingboard Sound/Speech I (model D) and Phasor cards.
//! The SSI263 receives data from the 6522 VIA's Port A register.
//!
//! Reference: source/SSI263.cpp, source/SSI263Phonemes.h

// Phoneme parameters: (F1_hz, F2_hz, is_voiced, base_duration_samples_at_22050hz)
// 64 entries covering the full SSI263 phoneme set.
const PHONEME_PARAMS: [(u16, u16, bool, u32); 64] = [
    // Vowels
    (270, 2290, true,  2000), // 0x00 PA (pause)
    (390, 1990, true,  1800), // 0x01 E
    (530, 1840, true,  1800), // 0x02 EH
    (660, 1720, true,  1800), // 0x03 AE
    (730, 1090, true,  2000), // 0x04 AH
    (570,  840, true,  2000), // 0x05 AW
    (440, 1020, true,  2000), // 0x06 AO
    (300,  870, true,  2000), // 0x07 UH
    (640, 1190, true,  2000), // 0x08 AX
    (490, 1350, true,  2000), // 0x09 IX
    (360, 2220, true,  2000), // 0x0A IH
    (270, 2290, true,  2000), // 0x0B IY
    (300,  870, true,  2000), // 0x0C UX
    (460, 1105, true,  2000), // 0x0D OH
    (400,  800, true,  2000), // 0x0E OW
    (640, 1190, true,  2000), // 0x0F UW
    // Diphthongs and more vowels
    (530, 1840, true,  2200), // 0x10 AY
    (300,  870, true,  2200), // 0x11 OY
    (400,  800, true,  2200), // 0x12 AW2
    (400, 1700, true,  2200), // 0x13 EY
    (300,  870, true,  2200), // 0x14 OW2
    (270, 2290, true,  2200), // 0x15 UW2
    (390, 1990, true,  2200), // 0x16 YU
    (490, 1350, true,  2200), // 0x17 ER
    // Semivowels
    (270, 2290, true,  1500), // 0x18 R
    (270, 2290, true,  1500), // 0x19 L
    (270, 2290, true,  1500), // 0x1A W
    (270, 2290, true,  1200), // 0x1B WH
    (270, 2290, true,  1500), // 0x1C Y
    (270, 2290, true,  1200), // 0x1D HH
    (270, 2290, true,  1200), // 0x1E HX
    (270, 2290, false, 1000), // 0x1F H2
    // Nasals
    (270, 2290, true,  1500), // 0x20 M
    (270, 2290, true,  1500), // 0x21 N
    (270, 2290, true,  1500), // 0x22 NG
    (270, 2290, true,  1500), // 0x23 NX
    (270, 2290, true,  1200), // 0x24 NW
    (270, 2290, true,  1200), // 0x25 RX
    (270, 2290, true,  1000), // 0x26 NG2
    (270, 2290, true,  1000), // 0x27 silence
    // Fricatives
    (270, 2290, false, 1500), // 0x28 S
    (270, 2290, false, 1500), // 0x29 SH
    (270, 2290, false, 1500), // 0x2A F
    (270, 2290, false, 1500), // 0x2B TH
    (270, 2290, true,  1500), // 0x2C Z
    (270, 2290, true,  1500), // 0x2D ZH
    (270, 2290, true,  1500), // 0x2E V
    (270, 2290, true,  1500), // 0x2F DH
    // Stops
    (270, 2290, false,  800), // 0x30 P
    (270, 2290, false,  800), // 0x31 T
    (270, 2290, false,  800), // 0x32 K
    (270, 2290, false,  800), // 0x33 KX
    (270, 2290, true,   800), // 0x34 B
    (270, 2290, true,   800), // 0x35 D
    (270, 2290, true,   800), // 0x36 G
    (270, 2290, true,   800), // 0x37 GX
    // Affricates / misc
    (270, 2290, false, 1000), // 0x38 CH
    (270, 2290, true,  1000), // 0x39 J
    (270, 2290, false, 1000), // 0x3A WH2
    (270, 2290, false,  800), // 0x3B silence2
    (270, 2290, false,  800), // 0x3C silence3
    (270, 2290, false,  800), // 0x3D silence4
    (270, 2290, false,  800), // 0x3E silence5
    (270, 2290, false,  800), // 0x3F silence6
];

// ── Simple biquad bandpass filter ────────────────────────────────────────────

/// Two-pole bandpass biquad filter used for formant synthesis.
#[derive(Clone)]
struct Biquad {
    b0: f32,
    b2: f32, // b1 = 0 for bandpass
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
    /// Cached centre frequency so we know when to recompute coefficients.
    last_freq: f32,
    last_sr: u32,
}

impl Biquad {
    fn new() -> Self {
        Self {
            b0: 0.0, b2: 0.0, a1: 0.0, a2: 0.0,
            z1: 0.0, z2: 0.0,
            last_freq: 0.0,
            last_sr: 0,
        }
    }

    /// Recompute coefficients if freq or sample_rate changed.
    fn update_coeffs(&mut self, freq: f32, sample_rate: u32) {
        if freq == self.last_freq && sample_rate == self.last_sr {
            return;
        }
        self.last_freq = freq;
        self.last_sr   = sample_rate;

        // Constant-peak-gain bandpass (Q = 5 gives a moderately narrow formant)
        let q: f32  = 5.0;
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate as f32;
        let alpha = w0.sin() / (2.0 * q);

        let b0 =  alpha;
        let b1 =  0.0_f32;
        let b2 = -alpha;
        let a0 =  1.0 + alpha;
        let a1 = -2.0 * w0.cos();
        let a2 =  1.0 - alpha;

        self.b0 =  b0 / a0;
        self.b2 =  b2 / a0;
        self.a1 =  b1; // store b1/a0 == 0 in a1 slot; actual a1/a0 below
        self.a2 =  a2 / a0;
        // Overwrite a1 with the correct value
        self.a1 = a1 / a0;
    }

    /// Process one sample through the filter.
    /// Coefficients must have been updated by calling `update_coeffs()` before
    /// the sample loop — do not call `update_coeffs()` here per-sample.
    fn process(&mut self, input: f32) -> f32 {
        // Direct Form II transposed
        let output = self.b0 * input + self.z1;
        self.z1 = /* b1 * input */ - self.a1 * output + self.z2;
        self.z2 = self.b2 * input - self.a2 * output;
        output
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// ── SSI263 ────────────────────────────────────────────────────────────────────

/// SSI263 chip state.
pub struct Ssi263 {
    /// 5 internal registers (indices 0-4).
    pub regs: [u8; 5],
    /// Cycles remaining until /READY is asserted (0 = idle/ready).
    /// Kept for save-state compatibility with version 2 format.
    pub ready_countdown: u64,
    /// True when chip is powered up (CTL bit = 0).
    pub powered: bool,

    // ── synthesis state ──────────────────────────────────────────────────────
    /// Current phoneme index (0-63).
    phoneme: u8,
    /// Samples remaining for the current phoneme (at whatever sample_rate was used).
    samples_remaining: u32,
    /// Phase accumulator (samples produced so far for current phoneme playback).
    phase: u64,
    /// LFSR state for white noise (unvoiced phonemes).
    noise_state: u32,
    /// Formant filter 1 (low formant).
    f1_filt: Biquad,
    /// Formant filter 2 (high formant).
    f2_filt: Biquad,
    /// Sample rate used when filter coefficients were last computed.
    /// Tracked so that a sample-rate change triggers a coefficient refresh.
    last_sample_rate: u32,
}

impl Default for Ssi263 {
    fn default() -> Self {
        Self::new()
    }
}

impl Ssi263 {
    pub fn new() -> Self {
        Self {
            regs: [0u8; 5],
            ready_countdown: 0,
            powered: true,
            phoneme: 0,
            samples_remaining: 0,
            phase: 0,
            noise_state: 0xACE1_u32,
            f1_filt: Biquad::new(),
            f2_filt: Biquad::new(),
            last_sample_rate: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    // ── register access ───────────────────────────────────────────────────────

    /// Write a value to the SSI263.
    /// `reg` = register index (0-4); the caller (mockingboard) already decoded
    /// the register from the address / ORA write context.
    pub fn write(&mut self, val: u8) {
        // The existing mockingboard code calls write(ora_val) where bits[4:3]
        // of val were originally used to select the register.  Re-derive reg
        // the same way the original stub did so existing call-sites are not broken.
        let reg = ((val >> 3) & 0x03) as usize;
        self.write_reg(reg as u8, val);
    }

    /// Write to a specific decoded register (0-4).
    pub fn write_reg(&mut self, reg: u8, val: u8) {
        match reg & 0x07 {
            0 => {
                // Duration / Phoneme register
                self.regs[0] = val;
                // bits[5:0] = phoneme; bits[7:6] = duration multiplier
                self.phoneme = val & 0x3F;
                // duration_factor: 0 = longest (4x), 3 = shortest (1x)
                // from C++ ref: (4 - (val>>6)) * base
                let dur_factor = 4u32.saturating_sub((val >> 6) as u32).max(1);
                let base_dur = PHONEME_PARAMS[self.phoneme as usize].3;
                self.samples_remaining = base_dur.saturating_mul(dur_factor);
                self.ready_countdown = self.samples_remaining as u64;
                self.phase = 0;
                self.f1_filt.reset();
                self.f2_filt.reset();
                // Force coefficient refresh on next fill_audio() call by
                // invalidating the cached sample rate.
                self.last_sample_rate = 0;
            }
            1 => {
                self.regs[1] = val; // Inflection
            }
            2 => {
                self.regs[2] = val; // Rate / Inflection
            }
            3 => {
                // Control / Articulation / Amplitude
                let ctl_was_set = self.regs[3] & 0x80 != 0;
                let ctl_now_set = val & 0x80 != 0;
                self.regs[3] = val;
                // CTL=1 → power-down / standby
                self.powered = !ctl_now_set;
                // CTL H→L: resume playback
                if ctl_was_set && !ctl_now_set {
                    // Re-trigger the current phoneme
                    let phoneme = self.phoneme;
                    let base_dur = PHONEME_PARAMS[phoneme as usize].3;
                    let dur_factor = 4u32.saturating_sub((self.regs[0] >> 6) as u32).max(1);
                    self.samples_remaining = base_dur.saturating_mul(dur_factor);
                    self.ready_countdown = self.samples_remaining as u64;
                    self.phase = 0;
                    self.f1_filt.reset();
                    self.f2_filt.reset();
                    self.last_sample_rate = 0;
                }
            }
            _ => {
                // Reg 4: Filter Frequency (and any aliased higher regs)
                if (reg as usize) < self.regs.len() {
                    self.regs[reg as usize] = val;
                }
            }
        }
    }

    // ── timing ────────────────────────────────────────────────────────────────

    /// Advance time by `delta` CPU cycles. Returns true if READY just fired (IFR CA1).
    pub fn tick(&mut self, delta: u64) -> bool {
        if self.ready_countdown == 0 || !self.powered {
            return false;
        }
        if delta >= self.ready_countdown {
            self.ready_countdown = 0;
            true // READY fired
        } else {
            self.ready_countdown -= delta;
            false
        }
    }

    /// True when not currently speaking (ready for next phoneme).
    pub fn is_ready(&self) -> bool {
        self.ready_countdown == 0
    }

    // ── audio synthesis ───────────────────────────────────────────────────────

    /// Pitch frequency in Hz derived from the Inflection register (reg 1).
    fn pitch_hz(&self) -> f64 {
        // reg1 maps 0-255 → ~65 Hz (low male) .. ~300 Hz (high child)
        65.0 + (self.regs[1] as f64 / 255.0) * 235.0
    }

    /// Fill `out` with mixed-in SSI263 audio samples.
    /// Adds (not replaces) into the existing contents of `out`.
    pub fn fill_audio(&mut self, out: &mut [f32], sample_rate: u32) {
        if !self.powered || self.samples_remaining == 0 {
            return;
        }

        let (f1, f2, voiced, _) = PHONEME_PARAMS[(self.phoneme & 0x3F) as usize];
        let f1 = f1 as f32;
        let f2 = f2 as f32;

        // Amplitude from bits[3:0] of reg 3 (0 = silence, 15 = full)
        let amp_raw = (self.regs[3] & 0x0F) as f32;
        let amplitude = amp_raw / 15.0 * 0.4;

        let n = out.len().min(self.samples_remaining as usize);

        // Update filter coefficients if sample rate changed.
        if sample_rate != self.last_sample_rate {
            self.f1_filt.update_coeffs(f1, sample_rate);
            self.f2_filt.update_coeffs(f2, sample_rate);
            self.last_sample_rate = sample_rate;
        }

        if voiced {
            let pitch_hz = self.pitch_hz();
            let sr = sample_rate as f64;
            for (i, sample) in out.iter_mut().enumerate().take(n) {
                let t = (self.phase + i as u64) as f64 / sr;
                // Simple glottal pulse: sawtooth-ish waveform
                let phase_in_period = (t * pitch_hz).fract();
                let excitation = if phase_in_period < 0.1 {
                    1.0_f32
                } else {
                    -0.1_f32
                };
                let s = self.f1_filt.process(excitation)
                      + self.f2_filt.process(excitation * 0.5);
                *sample += s * amplitude;
            }
        } else {
            for sample in out.iter_mut().take(n) {
                // Linear congruential noise for unvoiced fricatives
                self.noise_state = self.noise_state
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
                let noise = (self.noise_state as i32 as f32) / (i32::MAX as f32) * 0.3;
                let s = self.f1_filt.process(noise)
                      + self.f2_filt.process(noise * 0.5);
                *sample += s * amplitude;
            }
        }

        self.phase += n as u64;
        if n >= self.samples_remaining as usize {
            self.samples_remaining = 0;
        } else {
            self.samples_remaining -= n as u32;
        }
    }
}
