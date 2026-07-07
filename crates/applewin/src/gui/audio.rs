//! Audio output: cpal stream setup, the shared ring buffer, and per-frame
//! synthesis of speaker, Ensoniq DOC, and Mockingboard samples.

use super::*;

// ── Audio ─────────────────────────────────────────────────────────────────

pub(super) type AudioBuf = Arc<Mutex<VecDeque<f32>>>;

/// Apple II CPU clock (Hz) — NTSC.
const CPU_HZ: f64 = 1_023_000.0;
/// Maximum audio ring-buffer size in samples (2 seconds at 48 kHz).
const AUDIO_BUF_MAX: usize = 96_000;

/// Initialise cpal audio output.  Returns `(sample_rate, shared_buf, stream)`.
/// The stream must be kept alive for the duration of the program.
pub(super) fn init_audio() -> Option<(u32, AudioBuf, cpal::Stream)> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    let sr = config.sample_rate().0;
    let ch = config.channels() as usize;

    let buf: AudioBuf = Arc::new(Mutex::new(VecDeque::with_capacity(8192)));
    let buf2 = buf.clone();

    let err_fn = |e: cpal::StreamError| eprintln!("audio stream error: {e}");

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _| {
                    let mut q = buf2.lock().unwrap();
                    for frame in data.chunks_mut(ch) {
                        let s = q.pop_front().unwrap_or(0.0);
                        for c in frame.iter_mut() {
                            *c = s;
                        }
                    }
                },
                err_fn,
                None,
            )
            .ok()?,
        cpal::SampleFormat::I16 => device
            .build_output_stream(
                &config.into(),
                move |data: &mut [i16], _| {
                    let mut q = buf2.lock().unwrap();
                    for frame in data.chunks_mut(ch) {
                        let s = q.pop_front().unwrap_or(0.0);
                        let v = (s * i16::MAX as f32) as i16;
                        for c in frame.iter_mut() {
                            *c = v;
                        }
                    }
                },
                err_fn,
                None,
            )
            .ok()?,
        cpal::SampleFormat::U16 => device
            .build_output_stream(
                &config.into(),
                move |data: &mut [u16], _| {
                    let mut q = buf2.lock().unwrap();
                    for frame in data.chunks_mut(ch) {
                        let s = q.pop_front().unwrap_or(0.0);
                        let v = ((s + 1.0) * 0.5 * u16::MAX as f32) as u16;
                        for c in frame.iter_mut() {
                            *c = v;
                        }
                    }
                },
                err_fn,
                None,
            )
            .ok()?,
        _ => return None,
    };

    stream.play().ok()?;
    Some((sr, buf, stream))
}

impl EmulatorApp {
    pub(super) fn synth_speaker_audio(&mut self) {
        // ── Speaker audio synthesis ───────────────────────────────────────
        // Mirrors AppleWin's UpdateSpkr() / DCFilter() logic from Speaker.cpp.
        {
            // Swap speaker_toggles into our reusable scratch vec; this preserves
            // the bus's preallocated capacity across frames instead of resetting it
            // to 0 as std::mem::take would.
            self.speaker_toggles_scratch.clear();
            let end_cycle = if let Some(ref mut iigs) = self.iigs {
                std::mem::swap(
                    &mut iigs.bus.mega2.speaker_toggles,
                    &mut self.speaker_toggles_scratch,
                );
                iigs.cpu.cycles
            } else {
                std::mem::swap(
                    &mut self.emu.bus.speaker_toggles,
                    &mut self.speaker_toggles_scratch,
                );
                self.emu.cpu.cycles
            };
            let toggles = &self.speaker_toggles_scratch;
            let start_cycle = self.last_audio_cycle;
            self.last_audio_cycle = end_cycle;

            if let Some(buf) = &self.audio_buf {
                let sr = self.audio_sample_rate as f64;
                // Exact (fractional) cycles per output sample.  Flooring this
                // would over-produce samples by ~0.9% and slowly fill the ring
                // buffer to its cap (growing latency, then steady drops).
                let clks_per_sample = (CPU_HZ / sr).max(1.0);

                let delta = end_cycle.saturating_sub(start_cycle) as f64 + self.spkr_cycle_rem;
                let n_samples = (delta / clks_per_sample) as usize;
                self.spkr_cycle_rem = delta - n_samples as f64 * clks_per_sample;

                if n_samples > 0 {
                    let cycles_per_sample = delta / n_samples as f64;
                    let mut toggle_idx = 0usize;
                    let volume_scale = self.config.master_volume as f32 / 100.0;

                    // Synthesize samples into a lock-free scratch buffer first,
                    // then push them to the ring buffer under a single lock.
                    // Keeps the audio callback thread from waiting through a
                    // long sample loop (~735 iterations at 44.1 kHz / 60 fps).
                    self.speaker_scratch.clear();
                    self.speaker_scratch.reserve(n_samples);

                    for i in 0..n_samples {
                        let sample_start = start_cycle as f64 + i as f64 * cycles_per_sample;
                        let sample_end = sample_start + cycles_per_sample;

                        // Duty-cycle averaging: accumulate the time-weighted
                        // speaker level across every toggle inside this sample.
                        // Games drive the speaker faster than one toggle per
                        // output sample (PWM audio — e.g. Airheart's start
                        // sound toggles every 4–22 cycles vs ~23 cycles per
                        // sample); sampling only the final cone state aliases
                        // that ultrasonic carrier into a loud screech.
                        let mut acc = 0.0f64;
                        let mut seg_start = sample_start;
                        while toggle_idx < toggles.len()
                            && (toggles[toggle_idx] as f64) < sample_end
                        {
                            let tc = (toggles[toggle_idx] as f64).max(sample_start);
                            let level = if self.speaker_state { 0.5f64 } else { -0.5f64 };
                            acc += (tc - seg_start) / cycles_per_sample * level;
                            self.speaker_state = !self.speaker_state;
                            self.dc_filter_ctr = 32_768 + 10_000;
                            seg_start = tc;
                            toggle_idx += 1;
                        }
                        let level = if self.speaker_state { 0.5f64 } else { -0.5f64 };
                        acc += (sample_end - seg_start) / cycles_per_sample * level;

                        let raw = acc as f32;

                        let out = if self.dc_filter_ctr == 0 {
                            0.0f32
                        } else if self.dc_filter_ctr >= 32_768 {
                            self.dc_filter_ctr -= 1;
                            raw
                        } else {
                            let gain = self.dc_filter_ctr as f32 / 32_768.0;
                            self.dc_filter_ctr -= 1;
                            raw * gain
                        };

                        self.speaker_scratch.push(out * volume_scale);
                    }

                    // Consume any toggles past the last sample boundary so the
                    // cone position stays in phase for the next frame.
                    while toggle_idx < toggles.len() {
                        self.speaker_state = !self.speaker_state;
                        self.dc_filter_ctr = 32_768 + 10_000;
                        toggle_idx += 1;
                    }

                    let mut locked = buf.lock().unwrap();
                    for s in &self.speaker_scratch {
                        if locked.len() < AUDIO_BUF_MAX {
                            locked.push_back(*s);
                        }
                    }
                } else if !toggles.is_empty() {
                    // No samples this frame — still apply toggle parity so the
                    // speaker state doesn't drift out of phase.
                    if toggles.len() % 2 == 1 {
                        self.speaker_state = !self.speaker_state;
                    }
                    self.dc_filter_ctr = 32_768 + 10_000;
                }
            }
        }
    }

    pub(super) fn synth_ensoniq_audio(&mut self) {
        // ── Ensoniq DOC audio (IIgs only) ─────────────────────────────────
        if let Some(ref mut iigs) = self.iigs
            && let Some(buf) = &self.audio_buf
        {
            let sr = self.audio_sample_rate;
            let volume_scale = self.config.master_volume as f32 / 100.0;
            let delta_cycles =
                (self.config.emulation_speed.max(1) as f64 * 102_300.0 / 60.0) as u64;
            let n_samples = (sr as usize) / 60;
            if n_samples > 0 {
                self.ensoniq_scratch.clear();
                self.ensoniq_scratch.resize(n_samples, 0.0f32);
                iigs.bus
                    .ensoniq
                    .fill_audio(&mut self.ensoniq_scratch, sr, delta_cycles);

                let mut locked = buf.lock().unwrap();
                for &sample in &self.ensoniq_scratch {
                    if locked.len() < AUDIO_BUF_MAX {
                        locked.push_back(sample * volume_scale * 0.5);
                    }
                }
            }
        }
    }

    pub(super) fn synth_mockingboard_audio(&mut self) {
        // ── Mockingboard audio ────────────────────────────────────────────
        // Drain audio from any Mockingboard cards and mix into the ring buffer.
        // Collect all card samples first, then lock the ring buffer once for
        // all slots instead of once per slot.
        if self.audio_buf.is_some() {
            use apple2_core::card::CardType;
            let frame_cycles = self.config.cycles_per_frame();
            let sr = self.audio_sample_rate;
            let volume_scale = self.config.master_volume as f32 / 100.0;

            // Gather samples from every Mockingboard/Phasor slot before locking.
            // Pre-allocate: ~735 samples/frame at 44100 Hz, 2 cards max.
            let mut mb_samples: Vec<f32> = Vec::with_capacity(2048);
            for slot in 0..apple2_core::card::NUM_SLOTS {
                if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                    && (card.card_type() == CardType::Mockingboard
                        || card.card_type() == CardType::Phasor
                        || card.card_type() == CardType::Sam)
                {
                    card.fill_audio(&mut mb_samples, frame_cycles, sr);
                }
            }

            // Single lock acquisition for all collected samples.
            if !mb_samples.is_empty()
                && let Some(buf) = &self.audio_buf
            {
                let mut locked = buf.lock().unwrap();
                for s in mb_samples {
                    if locked.len() < AUDIO_BUF_MAX {
                        locked.push_back(s * volume_scale);
                    }
                }
            }
        }
    }

    pub(super) fn tap_wav_recording(&mut self) {
        // ── WAV audio recording — tap the ring buffer ────────────────────
        // Feed the latest samples to the WAV recorder if active.
        if let Some(ref mut rec) = self.wav_recorder
            && let Some(buf) = &self.audio_buf
        {
            let locked = buf.lock().unwrap();
            // Record the last N samples (approximate frame's worth).
            let n = (self.audio_sample_rate as usize / 60).min(locked.len());
            let start = locked.len().saturating_sub(n);
            self.wav_scratch.clear();
            self.wav_scratch.extend(locked.range(start..).copied());
            drop(locked);
            let _ = rec.write_samples(&self.wav_scratch);
        }
    }
}
