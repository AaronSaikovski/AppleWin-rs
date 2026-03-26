//! 1-bit Apple II speaker emulation.
//!
//! The speaker is toggled by accessing $C030.  Each toggle inverts the cone
//! position, producing a square wave whose frequency is determined by the
//! inter-toggle interval measured in CPU cycles.
//!
//! Reference: `source/Speaker.cpp`

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
}

impl Speaker {
    pub fn new(sample_rate: u32, cpu_hz: f64) -> Self {
        Self {
            state: false,
            last_toggle_cycle: 0,
            samples: Vec::with_capacity(4096),
            sample_rate,
            cpu_hz,
        }
    }

    /// Toggle the speaker cone (called on $C030 access).
    pub fn toggle(&mut self, cycle: u64) {
        self.state = !self.state;
        self.last_toggle_cycle = cycle;
    }

    /// Generate `n_cycles` worth of samples into `self.samples`.
    pub fn render(&mut self, start_cycle: u64, end_cycle: u64) {
        let cycles = (end_cycle - start_cycle) as f64;
        let n_samples = (cycles / self.cpu_hz * self.sample_rate as f64) as usize;
        let val = if self.state { 0.5f32 } else { -0.5f32 };
        self.samples.extend(std::iter::repeat_n(val, n_samples));
    }

    /// Drain generated samples into a destination buffer.
    pub fn drain_into(&mut self, dst: &mut Vec<f32>) {
        dst.append(&mut self.samples);
    }
}
