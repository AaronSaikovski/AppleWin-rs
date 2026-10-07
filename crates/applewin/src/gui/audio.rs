//! Audio output: cpal stream setup, the lock-free ring buffer feeding it, and
//! per-frame synthesis of speaker, Ensoniq DOC, and Mockingboard samples.

use super::*;
use apple2_audio::mix::{mix_into, mix_resampled_into};
use apple2_audio::wav_writer::WavRecorder;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// ── Audio ─────────────────────────────────────────────────────────────────

/// Apple II CPU clock (Hz) — NTSC.
const CPU_HZ: f64 = 1_023_000.0;
/// Maximum audio ring-buffer size in samples (2 seconds at 48 kHz).
const AUDIO_BUF_MAX: usize = 96_000;
/// `Config::emulation_speed` value for authentic 1× speed (1.023 MHz).
const SPEED_NORMAL: u32 = 10;

/// GUI-thread side of the audio output: the producer half of the SPSC ring
/// buffer drained by the cpal callback, plus a flush request flag the
/// callback honours (the producer cannot discard queued samples itself).
pub(super) struct AudioOut {
    producer: rtrb::Producer<f32>,
    flush: Arc<AtomicBool>,
}

impl AudioOut {
    /// Discard all queued audio: the cpal callback drains the ring before
    /// playing anything else.
    pub(super) fn flush(&self) {
        self.flush.store(true, Ordering::Release);
    }

    fn is_full(&self) -> bool {
        self.producer.is_full()
    }

    /// Queue as many `samples` as fit and record exactly the queued ones to
    /// the WAV file, so the recording matches what the device plays.
    fn queue(&mut self, samples: &[f32], wav: Option<&mut WavRecorder>) {
        let (queued, _dropped) = self.producer.push_partial_slice(samples);
        if let Some(rec) = wav {
            let _ = rec.write_samples(queued);
        }
    }
}

/// Fill one cpal output buffer: one ring sample per frame, duplicated across
/// all `ch` channels, silence on underrun.  Runs on the audio thread, so it
/// only touches the lock-free consumer and the atomic flush flag.
fn fill_output<T: Copy>(
    data: &mut [T],
    ch: usize,
    consumer: &mut rtrb::Consumer<f32>,
    flush: &AtomicBool,
    convert: impl Fn(f32) -> T,
) {
    if flush.swap(false, Ordering::AcqRel)
        && let Ok(stale) = consumer.read_chunk(consumer.slots())
    {
        stale.commit_all();
    }

    let wanted = data.len().div_ceil(ch);
    let mut frames = data.chunks_mut(ch);
    if let Ok(chunk) = consumer.read_chunk(wanted.min(consumer.slots())) {
        let (first, second) = chunk.as_slices();
        // Samples drive the zip: once they run out, `frames` is not advanced,
        // so the remaining frames are silenced below.
        for (&s, frame) in first.iter().chain(second).zip(&mut frames) {
            frame.fill(convert(s));
        }
        chunk.commit_all();
    }
    let silence = convert(0.0);
    for frame in frames {
        frame.fill(silence);
    }
}

/// Initialise cpal audio output.  Returns `(sample_rate, producer, stream)`.
/// The stream must be kept alive for the duration of the program.
pub(super) fn init_audio() -> Option<(u32, AudioOut, cpal::Stream)> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    let sr = config.sample_rate().0;
    let ch = config.channels() as usize;

    let (producer, mut consumer) = rtrb::RingBuffer::<f32>::new(AUDIO_BUF_MAX);
    let flush = Arc::new(AtomicBool::new(false));
    let flush_rx = flush.clone();

    let err_fn = |e: cpal::StreamError| eprintln!("audio stream error: {e}");

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _| {
                    fill_output(data, ch, &mut consumer, &flush_rx, |s| s);
                },
                err_fn,
                None,
            )
            .ok()?,
        cpal::SampleFormat::I16 => device
            .build_output_stream(
                &config.into(),
                move |data: &mut [i16], _| {
                    fill_output(data, ch, &mut consumer, &flush_rx, |s| {
                        (s * i16::MAX as f32) as i16
                    });
                },
                err_fn,
                None,
            )
            .ok()?,
        cpal::SampleFormat::U16 => device
            .build_output_stream(
                &config.into(),
                move |data: &mut [u16], _| {
                    fill_output(data, ch, &mut consumer, &flush_rx, |s| {
                        ((s + 1.0) * 0.5 * u16::MAX as f32) as u16
                    });
                },
                err_fn,
                None,
            )
            .ok()?,
        _ => return None,
    };

    stream.play().ok()?;
    Some((sr, AudioOut { producer, flush }, stream))
}

impl EmulatorApp {
    pub(super) fn synth_speaker_audio(&mut self) {
        // ── Speaker audio synthesis ───────────────────────────────────────
        // Mirrors AppleWin's UpdateSpkr() / DCFilter() logic from Speaker.cpp.

        // Swap speaker_toggles into our reusable scratch vec; this preserves
        // the bus's preallocated capacity across frames instead of resetting it
        // to 0 as std::mem::take would.
        self.speaker_toggles_scratch.clear();
        // Card audio is mixed into this buffer later in the frame, so it
        // must start empty even when the speaker produces no samples.
        self.speaker_scratch.clear();
        let end_cycle = if let Some(iigs) = &mut self.iigs {
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
        self.audio_frame_cycles = end_cycle.saturating_sub(start_cycle);

        // A full ring means the device isn't keeping up; anything synthesised
        // now would be dropped, so only advance the speaker state.
        let Some(ring_full) = self.audio_out.as_ref().map(AudioOut::is_full) else {
            return;
        };

        let sr = self.audio_sample_rate as f64;
        // Emulated cycles per output sample.  Scaling by the emulation speed
        // keeps the output at real-time rate (e.g. 4× speed packs 4× the
        // cycles into each sample) instead of over-producing samples that
        // pile up at the ring-buffer cap.  The factor is exactly 1.0 at
        // normal speed.  The value stays fractional: flooring it would
        // over-produce samples by ~0.9% and slowly fill the ring buffer.
        let speed = self.config.emulation_speed.max(1) as f64 / SPEED_NORMAL as f64;
        let clks_per_sample = (CPU_HZ * speed / sr).max(1.0);

        let delta = end_cycle.saturating_sub(start_cycle) as f64 + self.spkr_cycle_rem;
        let n_samples = (delta / clks_per_sample) as usize;
        self.spkr_cycle_rem = delta - n_samples as f64 * clks_per_sample;

        if n_samples > 0 && !ring_full {
            let cycles_per_sample = delta / n_samples as f64;
            let mut toggle_idx = 0usize;
            let volume_scale = self.config.master_volume as f32 / 100.0;

            // Synthesize into a scratch buffer; `synth_card_audio` mixes card
            // output into it and queues the result on the ring buffer.
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
                while toggle_idx < toggles.len() && (toggles[toggle_idx] as f64) < sample_end {
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
        } else {
            // No samples synthesised (none due, or the ring is full) — still
            // apply toggle parity so the speaker state doesn't drift out of
            // phase.
            if !toggles.is_empty() {
                if toggles.len() % 2 == 1 {
                    self.speaker_state = !self.speaker_state;
                }
                self.dc_filter_ctr = 32_768 + 10_000;
            }
            // Skipped samples still elapsed: keep the DC-filter fade on
            // schedule (a no-op when no samples were due).
            self.dc_filter_ctr = self.dc_filter_ctr.saturating_sub(n_samples as u32);
        }
    }

    pub(super) fn synth_ensoniq_audio(&mut self) {
        // ── Ensoniq DOC audio (IIgs only) ─────────────────────────────────
        if let Some(iigs) = &mut self.iigs
            && let Some(out) = &mut self.audio_out
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
                for sample in &mut self.ensoniq_scratch {
                    *sample *= volume_scale * 0.5;
                }
                out.queue(&self.ensoniq_scratch, self.wav_recorder.as_mut());
            }
        }
    }

    /// Mix Mockingboard/Phasor/SAM output into the speaker samples built by
    /// `synth_speaker_audio`, then queue the mixed frame on the ring buffer
    /// (and the WAV recorder, if active).
    ///
    /// Cards render the same emulated cycles the speaker consumed this frame,
    /// so both sources cover the same time span.  At normal speed they also
    /// have (nearly) the same sample count and sum sample-by-sample; at other
    /// speeds cards still size their output by emulated time while the
    /// speaker produces real-time samples, so card output is resampled to the
    /// speaker frame length first.  Each card renders into its own scratch
    /// buffer so multiple cards are summed too, rather than concatenated into
    /// a stream running at 2× rate.
    pub(super) fn synth_card_audio(&mut self) {
        let Some(out) = &mut self.audio_out else {
            return;
        };
        use apple2_core::card::CardType;
        let frame_cycles = self.audio_frame_cycles;
        let sr = self.audio_sample_rate;
        let volume_scale = self.config.master_volume as f32 / 100.0;
        let resample = self.config.emulation_speed.max(1) != SPEED_NORMAL;
        let ring_full = out.is_full();

        for slot in 0..apple2_core::card::NUM_SLOTS {
            if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                && matches!(
                    card.card_type(),
                    CardType::Mockingboard | CardType::Phasor | CardType::Sam
                )
            {
                // Always render so the card's pending-cycle accounting
                // advances; the output is only discarded when the ring is full.
                self.card_audio_scratch.clear();
                card.fill_audio(&mut self.card_audio_scratch, frame_cycles, sr);
                if ring_full {
                    continue;
                }
                if resample {
                    mix_resampled_into(
                        &mut self.speaker_scratch,
                        &self.card_audio_scratch,
                        volume_scale,
                    );
                } else {
                    mix_into(
                        &mut self.speaker_scratch,
                        &self.card_audio_scratch,
                        volume_scale,
                    );
                }
            }
        }

        if !self.speaker_scratch.is_empty() {
            out.queue(&self.speaker_scratch, self.wav_recorder.as_mut());
        }
    }
}
