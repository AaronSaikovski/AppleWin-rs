//! Mixing of independently synthesised audio sources into one stream.

/// Add `src * gain` sample-by-sample into `dst`.
///
/// Sources run on their own clocks, so their per-frame sample counts can differ
/// by a sample or two (or `dst` may be empty when the speaker produced nothing).
/// `dst` is extended with silence to the longer of the two lengths so every
/// source sample is mixed rather than appended after the others.
pub fn mix_into(dst: &mut Vec<f32>, src: &[f32], gain: f32) {
    if dst.len() < src.len() {
        dst.resize(src.len(), 0.0);
    }
    for (d, s) in dst.iter_mut().zip(src) {
        *d += s * gain;
    }
}

/// Resample `src` to `dst.len()` samples and add `resampled * gain` into `dst`.
///
/// Used when a source covers the same time span as `dst` but at a different
/// sample count (e.g. card audio rendered per emulated cycle while the
/// emulator runs faster or slower than real time).  Each output sample is the
/// area-weighted average of the source samples its span overlaps, so this
/// box-filters when decimating, sample-and-holds when expanding, and is the
/// identity for equal lengths.  An empty `src` or `dst` is a no-op.
pub fn mix_resampled_into(dst: &mut [f32], src: &[f32], gain: f32) {
    if dst.is_empty() || src.is_empty() {
        return;
    }
    let ratio = src.len() as f64 / dst.len() as f64;
    for (i, d) in dst.iter_mut().enumerate() {
        let start = i as f64 * ratio;
        let end = start + ratio;
        let first = start.floor() as usize;
        let last = (end.ceil() as usize).min(src.len());
        let mut acc = 0.0f64;
        for (j, &s) in src.iter().enumerate().take(last).skip(first) {
            let overlap = end.min((j + 1) as f64) - start.max(j as f64);
            if overlap > 0.0 {
                acc += overlap * s as f64;
            }
        }
        *d += (acc / ratio) as f32 * gain;
    }
}

#[cfg(test)]
mod tests {
    use super::{mix_into, mix_resampled_into};

    #[test]
    fn sums_overlapping_samples_instead_of_appending() {
        let mut dst = vec![0.25, -0.25, 0.5];
        mix_into(&mut dst, &[0.5, 0.5, 0.5], 0.5);
        assert_eq!(dst, vec![0.5, 0.0, 0.75]);
    }

    #[test]
    fn longer_source_extends_with_source_samples() {
        let mut dst = vec![0.1];
        mix_into(&mut dst, &[0.2, 0.4, 0.6], 1.0);
        assert_eq!(dst.len(), 3);
        assert!((dst[0] - 0.3).abs() < 1e-6);
        assert_eq!(&dst[1..], &[0.4, 0.6]);
    }

    #[test]
    fn shorter_source_keeps_destination_length() {
        let mut dst = vec![0.1, 0.2, 0.3];
        mix_into(&mut dst, &[0.1], 1.0);
        assert_eq!(dst.len(), 3);
        assert_eq!(&dst[1..], &[0.2, 0.3]);
    }

    #[test]
    fn resample_equal_length_adds_source_unchanged() {
        let mut dst = vec![0.5, 0.0, -0.5];
        mix_resampled_into(&mut dst, &[0.25, 0.5, 0.75], 2.0);
        assert_eq!(dst, vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn resample_downsample_box_averages() {
        let mut dst = vec![0.0; 2];
        mix_resampled_into(&mut dst, &[1.0, 3.0, -2.0, -4.0], 1.0);
        assert_eq!(dst, vec![2.0, -3.0]);
    }

    #[test]
    fn resample_fractional_downsample_weights_partial_overlap() {
        // Ratio 1.5: output 0 = src[0] + half of src[1]; output 1 = half of src[1] + src[2].
        let mut dst = vec![0.0; 2];
        mix_resampled_into(&mut dst, &[3.0, 6.0, 0.0], 1.0);
        assert_eq!(dst, vec![4.0, 2.0]);
    }

    #[test]
    fn resample_upsample_holds_each_source_sample() {
        let mut dst = vec![0.0; 4];
        mix_resampled_into(&mut dst, &[1.0, -1.0], 0.5);
        assert_eq!(dst, vec![0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn resample_preserves_constant_signal_level() {
        let mut dst = vec![0.0; 7];
        mix_resampled_into(&mut dst, &[0.25; 40], 1.0);
        for d in dst {
            assert!((d - 0.25).abs() < 1e-6);
        }
    }

    #[test]
    fn resample_empty_source_or_destination_is_noop() {
        let mut dst = vec![0.1, 0.2];
        mix_resampled_into(&mut dst, &[], 1.0);
        assert_eq!(dst, vec![0.1, 0.2]);

        let mut empty: Vec<f32> = Vec::new();
        mix_resampled_into(&mut empty, &[1.0, 2.0], 1.0);
        assert!(empty.is_empty());
    }
}
