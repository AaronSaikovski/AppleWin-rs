//! AY-3-8910 / YM2149 Programmable Sound Generator emulation.
//!
//! Used by the Mockingboard and Phasor sound cards.
//! Reference: `source/AY8910.cpp`

/// Number of PSG channels.
const NUM_CHANNELS: usize = 3;

/// AY-3-8910 chip state.
pub struct Ay8910 {
    /// 16 register values ($00–$0F).
    pub regs: [u8; 16],
    /// Selected register index.
    pub selected_reg: u8,
    /// Tone period counters (one per channel).
    tone_counters: [u16; NUM_CHANNELS],
    /// Tone flip-flop states.
    tone_states:   [bool; NUM_CHANNELS],
    /// Noise LFSR state (17-bit).
    noise_lfsr:    u32,
    /// Noise counter.
    noise_counter: u16,
    /// Envelope counter.
    env_counter:   u32,
    /// Envelope step (0–15).
    env_step:      u8,
    /// Envelope direction.
    env_hold:      bool,
    env_alternate: bool,
    env_attack:    bool,
    env_holding:   bool,
    /// Marks that the envelope volume changed this render pass and the volume
    /// caches need refreshing.  Set whenever the envelope state advances;
    /// cleared after `refresh_volume_caches()` is called.
    env_vol_dirty: bool,
    // ── Cached derived values (recomputed only on register write) ─────────────
    /// Cached tone periods for channels 0–2.
    cached_tone_periods:  [u16; NUM_CHANNELS],
    /// Cached noise period.
    cached_noise_period:  u16,
    /// Cached envelope period.
    cached_env_period:    u32,
    /// Cached scaled volumes for channels 0–2 (f32, pre-divided by NUM_CHANNELS).
    cached_volumes:       [f32; NUM_CHANNELS],
}

impl Default for Ay8910 {
    fn default() -> Self {
        Self {
            regs:                 [0u8; 16],
            selected_reg:         0,
            tone_counters:        [0; NUM_CHANNELS],
            tone_states:          [false; NUM_CHANNELS],
            noise_lfsr:           1,
            noise_counter:        0,
            env_counter:          0,
            env_step:             0,
            env_hold:             false,
            env_alternate:        false,
            env_attack:           false,
            env_holding:          false,
            env_vol_dirty:        false,
            cached_tone_periods:  [1; NUM_CHANNELS],
            cached_noise_period:  1,
            cached_env_period:    1,
            cached_volumes:       [0.0; NUM_CHANNELS],
        }
    }
}

impl Ay8910 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Write a value to the currently selected register (via BDIR/BC1 lines).
    pub fn write_reg(&mut self, val: u8) {
        let r = self.selected_reg as usize;
        if r >= 16 {
            return;
        }
        self.regs[r] = val;
        // Envelope shape write triggers restart
        if r == 13 {
            self.env_counter   = 0;
            self.env_step      = 0;
            self.env_hold      = self.regs[13] & 0x01 != 0;
            self.env_alternate = self.regs[13] & 0x02 != 0;
            self.env_attack    = self.regs[13] & 0x04 != 0;
            self.env_holding   = false;
            self.env_vol_dirty = true;
        }
        if r == 6 || r == 7 {
            // Noise period or mixer: mask to valid bits
            self.regs[6] &= 0x1F;
        }
        // Tone period regs: mask upper nibble of coarse to 0x0F
        if r == 1 || r == 3 || r == 5 {
            self.regs[r] &= 0x0F;
        }
        // Refresh caches for any register that affects them.
        self.refresh_caches(r);
    }

    /// Recompute cached derived values for all 16 registers (used after load_state).
    pub fn refresh_all_caches(&mut self) {
        for r in 0..16 {
            self.refresh_caches(r);
        }
    }

    /// Recompute all cached derived values after register `r` was written.
    fn refresh_caches(&mut self, r: usize) {
        // Tone periods: regs 0/1 (ch0), 2/3 (ch1), 4/5 (ch2)
        match r {
            0 | 1 => self.cached_tone_periods[0] = Self::calc_tone_period(&self.regs, 0),
            2 | 3 => self.cached_tone_periods[1] = Self::calc_tone_period(&self.regs, 1),
            4 | 5 => self.cached_tone_periods[2] = Self::calc_tone_period(&self.regs, 2),
            6     => {
                let p = (self.regs[6] & 0x1F) as u16;
                self.cached_noise_period = if p == 0 { 1 } else { p };
            }
            11 | 12 => {
                let lo = self.regs[11] as u32;
                let hi = self.regs[12] as u32;
                let p  = (hi << 8) | lo;
                self.cached_env_period = if p == 0 { 1 } else { p };
            }
            // Regs 8–10: channel volumes, reg 13: envelope (which feeds volume)
            8..=10 | 13 => {
                for ch in 0..NUM_CHANNELS {
                    self.cached_volumes[ch] = self.calc_volume(ch);
                }
            }
            _ => {}
        }
    }

    fn calc_tone_period(regs: &[u8; 16], ch: usize) -> u16 {
        let lo = regs[ch * 2] as u16;
        let hi = (regs[ch * 2 + 1] & 0x0F) as u16;
        let p  = (hi << 8) | lo;
        if p == 0 { 1 } else { p }
    }

    #[inline]
    fn calc_volume(&self, ch: usize) -> f32 {
        let v = self.regs[8 + ch] & 0x1F;
        let raw = if v & 0x10 != 0 {
            self.envelope_volume()
        } else {
            (v & 0x0F) * 2
        };
        raw as f32 / 30.0 / NUM_CHANNELS as f32
    }

    /// Refresh volume caches — call after envelope state changes during render.
    fn refresh_volume_caches(&mut self) {
        for ch in 0..NUM_CHANNELS {
            self.cached_volumes[ch] = self.calc_volume(ch);
        }
    }

    /// Select register address (latch address).
    pub fn select_reg(&mut self, addr: u8) {
        self.selected_reg = addr & 0x0F;
    }

    /// Read the currently selected register.
    pub fn read_reg(&self) -> u8 {
        let r = self.selected_reg as usize;
        if r < 16 { self.regs[r] } else { 0xFF }
    }

    #[inline]
    fn envelope_volume(&self) -> u8 {
        let step = if self.env_holding {
            if self.env_hold {
                if self.env_alternate ^ self.env_attack { 15 } else { 0 }
            } else {
                self.env_step
            }
        } else {
            self.env_step
        };
        if self.env_attack { step } else { 15 - step }
    }

    /// Render `n_samples` of audio into `out`, mixing with existing content.
    /// `clock_rate` is the AY clock in Hz; `sample_rate` is output rate in Hz.
    ///
    /// This is a simple cycle-accurate approach: advance state per sample.
    pub fn render(&mut self, out: &mut [f32], _clock_rate: f64, _sample_rate: u32) {
        let mixer = self.regs[7];

        // Snapshot cached values into locals — avoids repeated struct field
        // indirection inside the hot loop.
        let tone_periods  = self.cached_tone_periods;
        let noise_period  = self.cached_noise_period;
        let env_period    = self.cached_env_period;

        // Pre-extract mixer enable bits — these are constant for the entire render
        // pass (mixer register 7 does not change mid-render).  Avoids recomputing
        // bit shifts inside the double-nested sample × channel loop.
        let tone_enabled = [
            mixer & 0x01 == 0,
            mixer & 0x02 == 0,
            mixer & 0x04 == 0,
        ];
        let noise_enabled = [
            mixer & 0x08 == 0,
            mixer & 0x10 == 0,
            mixer & 0x20 == 0,
        ];

        for sample in out.iter_mut() {
            // Advance tone counters
            #[allow(clippy::needless_range_loop)]
            for ch in 0..NUM_CHANNELS {
                self.tone_counters[ch] = self.tone_counters[ch].wrapping_add(1);
                if self.tone_counters[ch] >= tone_periods[ch] {
                    self.tone_counters[ch] = 0;
                    self.tone_states[ch] = !self.tone_states[ch];
                }
            }

            // Advance noise counter
            self.noise_counter = self.noise_counter.wrapping_add(1);
            if self.noise_counter >= noise_period {
                self.noise_counter = 0;
                // Galois LFSR: x^17 + x^14 + 1
                let feedback = (self.noise_lfsr ^ (self.noise_lfsr >> 3)) & 1;
                self.noise_lfsr = (self.noise_lfsr >> 1) | (feedback << 16);
            }
            let noise_out = self.noise_lfsr & 1 != 0;

            // Advance envelope; mark dirty when the step changes so that
            // volume caches are refreshed once per envelope period (not once
            // per sample).
            self.env_counter += 1;
            if self.env_counter >= env_period {
                self.env_counter = 0;
                if !self.env_holding {
                    self.env_step += 1;
                    if self.env_step >= 16 {
                        if self.env_hold {
                            self.env_holding = true;
                            self.env_step = 15;
                        } else if self.env_alternate {
                            self.env_attack = !self.env_attack;
                            self.env_step = 0;
                        } else {
                            self.env_step = 0;
                        }
                    }
                    self.env_vol_dirty = true;
                }
            }

            // Flush volume cache after envelope state has stabilised for this
            // sample period.  Done once here rather than inside the step block
            // so that multiple step advances (if env_period < 1) collapse into
            // a single cache refresh.
            if self.env_vol_dirty {
                self.refresh_volume_caches();
                self.env_vol_dirty = false;
            }

            // Mix channels using pre-computed volumes and pre-extracted mixer bits.
            let mut mixed = 0.0f32;
            for ch in 0..NUM_CHANNELS {
                let gate = (tone_enabled[ch] && self.tone_states[ch]) || (noise_enabled[ch] && noise_out);
                if gate {
                    mixed += self.cached_volumes[ch];
                }
            }
            *sample += mixed;
        }
    }
}
