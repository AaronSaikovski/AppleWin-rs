//! 1-bit Apple II speaker emulation.
//!
//! The speaker is toggled by accessing $C030.  Each toggle inverts the cone
//! position, producing a square wave whose frequency is determined by the
//! inter-toggle interval measured in CPU cycles.
//!
//! This implementation provides:
//! - Sub-sample interpolation: when a toggle occurs mid-sample, the output is
//!   proportionally blended between the old and new levels.
//! - DC removal high-pass filter to prevent speaker drift when the cone is
//!   held in one position.
//! - Conservative amplitude (0.3) to leave headroom for mixing with other
//!   audio sources (Mockingboard, SSI-263).
//!
//! Reference: `source/Speaker.cpp`

/// Output amplitude for the speaker square wave.
/// Kept below 0.5 to avoid clipping when mixed with other audio sources.
const AMPLITUDE: f32 = 0.3;

/// DC removal filter coefficient (first-order high-pass).
/// alpha ~0.99 gives a cutoff of ~35 Hz at 44100 Hz sample rate.
const DC_ALPHA: f32 = 0.99;

/// State of the speaker.
pub struct Speaker {
    /// Current cone position (true = deflected).
    pub state: bool,
    /// CPU cycle count of last toggle.
    pub last_toggle_cycle: u64,
    /// Pending audio samples buffer (f32, ±1.0).
    pub samples: Vec<f32>,
    /// Output sample rate (Hz).
    pub sample_rate: u32,
    /// Apple II CPU clock rate (Hz).
    pub cpu_hz: f64,

    // DC removal filter state
    dc_prev_input: f32,
    dc_prev_output: f32,
}

impl Speaker {
    pub fn new(sample_rate: u32, cpu_hz: f64) -> Self {
        Self {
            state: false,
            last_toggle_cycle: 0,
            samples: Vec::with_capacity(4096),
            sample_rate,
            cpu_hz,
            dc_prev_input: 0.0,
            dc_prev_output: 0.0,
        }
    }

    /// Toggle the speaker cone (called on $C030 access).
    pub fn toggle(&mut self, cycle: u64) {
        self.state = !self.state;
        self.last_toggle_cycle = cycle;
    }

    /// Return the raw amplitude value for the current speaker state.
    #[inline]
    fn level(&self) -> f32 {
        if self.state { AMPLITUDE } else { -AMPLITUDE }
    }

    /// Apply the DC removal high-pass filter to one sample.
    ///
    /// ```text
    /// y[n] = x[n] - x[n-1] + alpha * y[n-1]
    /// ```
    #[inline]
    fn dc_filter(&mut self, x: f32) -> f32 {
        let y = x - self.dc_prev_input + DC_ALPHA * self.dc_prev_output;
        self.dc_prev_input = x;
        self.dc_prev_output = y;
        y
    }

    /// Generate samples for the cycle range `[start_cycle, end_cycle)`, using
    /// the provided toggle timestamps for sub-sample accurate rendering.
    ///
    /// `toggles` should contain the cycle timestamps of every $C030 access
    /// that occurred during `[start_cycle, end_cycle)`, in ascending order.
    /// These are typically drained from `Bus::speaker_toggles`.
    pub fn render(&mut self, start_cycle: u64, end_cycle: u64, toggles: &[u64]) {
        let total_cycles = (end_cycle - start_cycle) as f64;
        if total_cycles <= 0.0 {
            return;
        }

        let n_samples = (total_cycles / self.cpu_hz * self.sample_rate as f64) as usize;
        if n_samples == 0 {
            // Still consume toggles so speaker state stays correct.
            for _ in toggles {
                self.state = !self.state;
            }
            return;
        }

        let cycles_per_sample = total_cycles / n_samples as f64;
        let mut toggle_idx = 0usize;

        self.samples.reserve(n_samples);

        for i in 0..n_samples {
            let sample_start = start_cycle as f64 + i as f64 * cycles_per_sample;
            let sample_end = sample_start + cycles_per_sample;

            // Walk through any toggles that fall within this sample period
            // and compute the blended (interpolated) value.
            let mut accumulated = 0.0f64;
            let mut seg_start = sample_start;

            while toggle_idx < toggles.len()
                && (toggles[toggle_idx] as f64) < sample_end
            {
                let toggle_cycle = toggles[toggle_idx] as f64;
                // Clamp to sample boundary (toggle may be before sample_start
                // if there is slight overlap from rounding).
                let tc = toggle_cycle.max(sample_start);

                // Accumulate the fraction of this sample at the current level.
                let frac = (tc - seg_start) / cycles_per_sample;
                accumulated += frac * self.level() as f64;

                // Perform the toggle.
                self.state = !self.state;
                seg_start = tc;
                toggle_idx += 1;
            }

            // Remaining fraction of the sample after the last toggle (or the
            // entire sample if there were no toggles).
            let frac = (sample_end - seg_start) / cycles_per_sample;
            accumulated += frac * self.level() as f64;

            let raw = accumulated as f32;

            // Apply DC removal filter.
            let filtered = self.dc_filter(raw);
            self.samples.push(filtered);
        }
    }

    /// Drain generated samples into a destination buffer.
    pub fn drain_into(&mut self, dst: &mut Vec<f32>) {
        dst.append(&mut self.samples);
    }

    /// Reset the DC filter state (useful after long pauses or speed changes).
    pub fn reset_dc_filter(&mut self) {
        self.dc_prev_input = 0.0;
        self.dc_prev_output = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SAMPLE_RATE: u32 = 44100;
    const TEST_CPU_HZ: f64 = 1_020_484.0; // Apple IIe PAL-ish, close to NTSC

    fn make_speaker() -> Speaker {
        Speaker::new(TEST_SAMPLE_RATE, TEST_CPU_HZ)
    }

    #[test]
    fn test_no_toggles_produces_silence_after_dc_filter() {
        // With no toggles, the speaker stays at -AMPLITUDE. The DC filter
        // should converge the output toward zero over time.
        let mut spk = make_speaker();
        // Render 1 second of audio with no toggles.
        let cycles_per_sec = TEST_CPU_HZ as u64;
        spk.render(0, cycles_per_sec, &[]);

        // After enough samples, the DC filter output should be near zero.
        let last_samples = &spk.samples[spk.samples.len() - 100..];
        for &s in last_samples {
            assert!(
                s.abs() < 0.01,
                "DC filter should converge to ~0 for constant input, got {s}"
            );
        }
    }

    #[test]
    fn test_dc_filter_converges_for_constant_input() {
        let mut spk = make_speaker();
        // Feed a constant value through the DC filter many times.
        let constant = 0.3f32;
        let mut last = 0.0f32;
        for _ in 0..10_000 {
            last = spk.dc_filter(constant);
        }
        // Should converge to zero (DC is removed).
        assert!(
            last.abs() < 0.001,
            "DC filter output should approach 0 for constant input, got {last}"
        );
    }

    #[test]
    fn test_toggle_produces_nonzero_output() {
        let mut spk = make_speaker();
        // Create a square wave by toggling every ~500 cycles (~1 kHz).
        let toggle_interval = 500u64;
        let total_cycles = 50_000u64;
        let mut toggles = Vec::new();
        let mut c = toggle_interval;
        while c < total_cycles {
            toggles.push(c);
            c += toggle_interval;
        }

        spk.render(0, total_cycles, &toggles);

        // The output should contain nonzero samples (the square wave passes
        // through the high-pass filter with minimal attenuation at 1 kHz).
        let max_abs = spk
            .samples
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_abs > 0.1,
            "Square wave at ~1 kHz should produce significant output, max_abs = {max_abs}"
        );
    }

    #[test]
    fn test_subsample_interpolation_midpoint_toggle() {
        // Place a single toggle exactly at the midpoint of the first sample.
        // The first sample should blend 50% -AMPLITUDE + 50% +AMPLITUDE = 0.
        let mut spk = make_speaker();
        // state starts false => level = -AMPLITUDE

        // Use enough cycles to guarantee at least 1 sample.
        let cycles_per_sample_f = TEST_CPU_HZ / TEST_SAMPLE_RATE as f64;
        let total_cycles = (cycles_per_sample_f.ceil() + 1.0) as u64;
        let mid = total_cycles / 2;

        spk.render(0, total_cycles, &[mid]);

        assert!(!spk.samples.is_empty(), "Should produce at least 1 sample");

        // The raw (pre-filter) value would be ~0.0. After the DC filter,
        // since prev_input and prev_output start at 0, the first filtered
        // sample should also be ~0.
        let first = spk.samples[0];
        assert!(
            first.abs() < 0.05,
            "Midpoint toggle should produce ~0 blended sample, got {first}"
        );
    }

    #[test]
    fn test_subsample_interpolation_quarter_toggle() {
        // Toggle at 25% through the sample.
        // Expected raw: 0.25 * (-AMPLITUDE) + 0.75 * AMPLITUDE = 0.5 * AMPLITUDE
        let mut spk = make_speaker();
        let cycles_per_sample_f = TEST_CPU_HZ / TEST_SAMPLE_RATE as f64;
        let total_cycles = (cycles_per_sample_f.ceil() + 1.0) as u64;
        let quarter = total_cycles / 4;

        spk.render(0, total_cycles, &[quarter]);

        assert!(!spk.samples.is_empty());

        // Expected raw value: -0.25*A + 0.75*A = 0.5*A = 0.15
        // DC filter on first sample with zero history: y = x - 0 + 0.99*0 = x
        let first = spk.samples[0];
        let expected = 0.5 * AMPLITUDE;
        assert!(
            (first - expected).abs() < 0.02,
            "Quarter toggle: expected ~{expected}, got {first}"
        );
    }

    #[test]
    fn test_multiple_toggles_in_one_sample() {
        // Two toggles in one sample: toggle at 25% and 75%.
        // Segments: [0,25%) = -A (25%), [25%,75%) = +A (50%), [75%,100%) = -A (25%)
        // Blended raw = -0.25*A + 0.50*A - 0.25*A = 0.0
        let mut spk = make_speaker();
        let cycles_per_sample_f = TEST_CPU_HZ / TEST_SAMPLE_RATE as f64;
        let total_cycles = (cycles_per_sample_f.ceil() + 1.0) as u64;
        let q1 = total_cycles / 4;
        let q3 = 3 * total_cycles / 4;

        spk.render(0, total_cycles, &[q1, q3]);

        assert!(!spk.samples.is_empty());
        let first = spk.samples[0];
        assert!(
            first.abs() < 0.05,
            "Two symmetric toggles should produce ~0, got {first}"
        );
    }

    #[test]
    fn test_render_empty_range() {
        let mut spk = make_speaker();
        spk.render(100, 100, &[]);
        assert!(spk.samples.is_empty(), "Zero-length range should produce no samples");
    }

    #[test]
    fn test_drain_into() {
        let mut spk = make_speaker();
        let cycles = (TEST_CPU_HZ / TEST_SAMPLE_RATE as f64 * 10.0) as u64;
        spk.render(0, cycles, &[]);
        assert!(!spk.samples.is_empty());

        let mut dst = Vec::new();
        spk.drain_into(&mut dst);
        assert!(!dst.is_empty());
        assert!(spk.samples.is_empty(), "samples should be drained");
    }

    #[test]
    fn test_amplitude_within_bounds() {
        // Even with rapid toggling, output should stay within [-1, 1].
        let mut spk = make_speaker();
        let total = 100_000u64;
        let mut toggles = Vec::new();
        let mut c = 50u64;
        while c < total {
            toggles.push(c);
            c += 50; // very fast toggling
        }
        spk.render(0, total, &toggles);

        for &s in &spk.samples {
            assert!(
                s.abs() <= 1.0,
                "Output should be within [-1,1], got {s}"
            );
        }
    }
}
