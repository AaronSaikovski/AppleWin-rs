//! WAV audio recording — captures mixed emulator audio to disk.
//!
//! Mirrors the `-wav-speaker` / `-wav-mockingboard` functionality from
//! the C++ AppleWin.

use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// Streams f32 samples to a WAV file.
pub struct WavRecorder {
    writer: WavWriter<BufWriter<File>>,
}

impl WavRecorder {
    /// Begin recording to `path` at the given mono sample rate.
    pub fn start(path: &Path, sample_rate: u32) -> Result<Self, hound::Error> {
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let writer = WavWriter::create(path, spec)?;
        Ok(WavRecorder { writer })
    }

    /// Append audio samples.
    pub fn write_samples(&mut self, samples: &[f32]) -> Result<(), hound::Error> {
        for &s in samples {
            self.writer.write_sample(s)?;
        }
        Ok(())
    }

    /// Finalise the WAV file (writes the header length fields).
    pub fn stop(self) -> Result<(), hound::Error> {
        self.writer.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_read_back() {
        let dir = std::env::temp_dir();
        let path = dir.join("applewin_test_wav.wav");
        {
            let mut rec = WavRecorder::start(&path, 22050).unwrap();
            rec.write_samples(&[0.0, 0.5, -0.5, 1.0]).unwrap();
            rec.stop().unwrap();
        }
        // Read back and verify
        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 22050);
        let samples: Vec<f32> = reader.into_samples::<f32>().map(|s| s.unwrap()).collect();
        assert_eq!(samples.len(), 4);
        assert!((samples[0] - 0.0).abs() < 1e-6);
        assert!((samples[1] - 0.5).abs() < 1e-6);
        let _ = std::fs::remove_file(&path);
    }
}
