use crate::audio::AudioClip;

pub(crate) struct Detection {
    pub(crate) strength: Vec<f32>,
    pub(crate) threshold: Vec<f32>,
    pub(crate) hop: usize,
    pub(crate) fft_size: usize,
    pub(crate) spectral: Vec<usize>,
    pub(crate) legacy: Vec<usize>,
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mid = values.len() / 2;
    let (_, value, _) = values.select_nth_unstable_by(mid, f32::total_cmp);
    *value
}

// Positive log-spectral change against a frequency-neighbourhood maximum.
// The local median/MAD threshold uses surrounding time, never the clip maximum.
pub(crate) fn contextual(
    data: &crate::spectrum::Spectrum,
    clip: &AudioClip,
    sensitivity: f32,
    spacing_ms: f32,
) -> Detection {
    let bins = data.fft_size / 2 + 1;
    let hop_seconds = data.hop as f32 / data.sample_rate as f32;
    let lag = (0.010 / hop_seconds).round().max(1.0) as usize;
    let radius = (0.100 / hop_seconds).round().max(2.0) as usize;
    let exclusion = (0.015 / hop_seconds).round().max(1.0) as usize;
    let mut strength = vec![0.0; data.columns];
    let weight_sum: f32 = (1..bins).map(|bin| 1.0 / (bin as f32)).sum();
    for (column, value) in strength.iter_mut().enumerate() {
        let now = &data.db[column * bins..(column + 1) * bins];
        // Reject silence independently of the onset threshold.
        if now.iter().copied().fold(-120.0, f32::max) < -75.0 {
            continue;
        }
        for (bin, &current) in now.iter().enumerate().skip(1) {
            let previous = if column >= lag {
                let offset = (column - lag) * bins;
                data.db[offset + bin - 1..=offset + (bin + 1).min(bins - 1)]
                    .iter()
                    .copied()
                    .fold(-90.0, f32::max)
            } else {
                -90.0
            };
            *value += (current.max(-90.0) - previous).max(0.0) / bin as f32;
        }
        *value /= weight_sum;
    }
    let mut threshold = vec![0.0; data.columns];
    let mut local = Vec::new();
    for (i, limit) in threshold.iter_mut().enumerate() {
        local.clear();
        for (j, &value) in strength
            .iter()
            .enumerate()
            .take((i + radius + 1).min(data.columns))
            .skip(i.saturating_sub(radius))
        {
            if i.abs_diff(j) > exclusion {
                local.push(value);
            }
        }
        let center = median(&mut local);
        for value in &mut local {
            *value = (*value - center).abs();
        }
        let deviation = median(&mut local);
        let sensitivity = sensitivity.clamp(0.0, 1.0);
        *limit = center
            + (1.0 + 3.0 * (1.0 - sensitivity)) * deviation
            + 0.03
            + 0.5 * (1.0 - sensitivity);
    }
    let peak_radius = (0.010 / hop_seconds).round().max(1.0) as usize;
    let mut candidates = Vec::new();
    for i in 0..data.columns {
        if strength[i] <= threshold[i] {
            continue;
        }
        let start = i.saturating_sub(peak_radius);
        let end = (i + peak_radius + 1).min(data.columns);
        if strength[start..i].iter().any(|&v| v >= strength[i])
            || strength[i + 1..end].iter().any(|&v| v > strength[i])
        {
            continue;
        }
        candidates.push((i * data.hop, strength[i] - threshold[i]));
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let spacing =
        ((spacing_ms.max(1.0) * data.sample_rate as f32 / 1000.0).round() as usize).max(1);
    let mut chosen = std::collections::BTreeSet::new();
    for (frame, _) in candidates {
        if chosen
            .range(frame.saturating_sub(spacing - 1)..=frame.saturating_add(spacing - 1))
            .next()
            .is_none()
        {
            chosen.insert(frame);
        }
    }
    Detection {
        strength,
        threshold,
        hop: data.hop,
        fft_size: data.fft_size,
        spectral: chosen.into_iter().collect(),
        legacy: detect(clip, sensitivity, spacing_ms),
    }
}

// Analyze one-millisecond peak blocks, combining channels without cancellation.
// Positive envelope growth suggests an attack; these are proposals, not cuts.
pub(crate) fn detect(clip: &AudioClip, sensitivity: f32, spacing_ms: f32) -> Vec<usize> {
    let channels = usize::from(clip.channels);
    let block = (clip.sample_rate as usize / 1000).max(1);
    let peaks: Vec<f32> = clip
        .samples
        .chunks(block * channels)
        .map(|chunk| chunk.iter().fold(0.0_f32, |peak, s| peak.max(s.abs())))
        .collect();
    let maximum = peaks.iter().copied().fold(0.0_f32, f32::max);
    if maximum < 0.000_01 {
        return Vec::new();
    }
    let threshold = maximum * (0.015 + 0.3 * (1.0 - sensitivity.clamp(0.0, 1.0)));
    let mut baseline = 0.0_f32;
    let mut envelope = 0.0_f32;
    let growth: Vec<f32> = peaks
        .iter()
        .map(|&peak| {
            envelope = envelope * 0.65 + peak * 0.35;
            let rise = (envelope - baseline).max(0.0);
            baseline = baseline * 0.97 + envelope * 0.03;
            rise
        })
        .collect();
    let mut candidates: Vec<(usize, f32)> = growth
        .iter()
        .enumerate()
        .filter(|&(i, &value)| {
            value >= threshold
                && (i == 0 || value > growth[i - 1])
                && (i + 1 == growth.len() || value >= growth[i + 1])
        })
        .map(|(i, &value)| {
            // Refine the smoothed peak toward the start of its rise (up to 10 ms).
            let mut onset = i;
            while onset > i.saturating_sub(10) && peaks[onset - 1] > peaks[i] * 0.2 {
                onset -= 1;
            }
            (onset * block, value)
        })
        .collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let spacing =
        ((spacing_ms.max(1.0) * clip.sample_rate as f32 / 1000.0).round() as usize).max(1);
    let mut markers = std::collections::BTreeSet::new();
    for (frame, _) in candidates {
        if markers
            .range(frame.saturating_sub(spacing - 1)..=frame.saturating_add(spacing - 1))
            .next()
            .is_none()
        {
            markers.insert(frame);
        }
    }
    markers.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone_clip() -> AudioClip {
        let rate = 16000;
        let samples = (0..32000)
            .map(|i| {
                let (start, amplitude) = if (1600..6400).contains(&i) {
                    (1600, 0.8)
                } else if (19200..24000).contains(&i) {
                    (19200, 0.025)
                } else {
                    return 0.0;
                };
                amplitude * (std::f32::consts::TAU * 500.0 * (i - start) as f32 / rate as f32).sin()
            })
            .collect::<Vec<_>>();
        AudioClip {
            channels: 1,
            sample_rate: rate,
            duration_seconds: 2.0,
            samples: samples.into(),
        }
    }
    #[test]
    fn spectral_detects_quiet_attack_beside_loud_passage() {
        let clip = tone_clip();
        let data = crate::spectrum::analyze(&clip, 512);
        let result = contextual(&data, &clip, 0.65, 60.0);
        assert!(
            result.spectral.iter().any(|&f| f.abs_diff(1600) < 480),
            "{:?}",
            result.spectral
        );
        assert!(
            result.spectral.iter().any(|&f| f.abs_diff(19200) < 480),
            "{:?}",
            result.spectral
        );
        assert_eq!(result.strength.len(), result.threshold.len());
        assert!(
            result
                .strength
                .iter()
                .chain(&result.threshold)
                .all(|n| n.is_finite())
        );
    }
    #[test]
    fn spectral_detects_frequency_change_at_constant_amplitude() {
        let samples = (0..32000)
            .map(|i| {
                let hz = if i < 16000 { 500.0 } else { 2000.0 };
                0.5 * (std::f32::consts::TAU * hz * i as f32 / 16000.0).sin()
            })
            .collect::<Vec<_>>();
        let clip = AudioClip {
            channels: 1,
            sample_rate: 16000,
            duration_seconds: 2.0,
            samples: samples.into(),
        };
        let result = contextual(&crate::spectrum::analyze(&clip, 1024), &clip, 0.65, 60.0);
        assert!(
            result.spectral.iter().any(|&f| f.abs_diff(16000) < 640),
            "{:?}",
            result.spectral
        );
        assert!(
            result.spectral.iter().all(|&f| !(3200..14000).contains(&f)),
            "sustain triggered: {:?}",
            result.spectral
        );
    }
    #[test]
    fn contextual_silence_does_not_trigger() {
        let clip = AudioClip {
            channels: 1,
            sample_rate: 16000,
            duration_seconds: 0.1,
            samples: vec![0.0; 1600].into(),
        };
        assert!(
            contextual(&crate::spectrum::analyze(&clip, 512), &clip, 1.0, 5.0)
                .spectral
                .is_empty()
        );
    }
    #[test]
    fn sensitivity_reveals_quieter_attacks() {
        let mut samples = vec![0.0; 1000];
        samples[100..120].fill(1.0);
        samples[500..520].fill(0.1);
        let clip = AudioClip {
            channels: 1,
            sample_rate: 1000,
            duration_seconds: 1.0,
            samples: samples.into(),
        };
        assert_eq!(detect(&clip, 0.0, 30.0), vec![100]);
        assert_eq!(detect(&clip, 1.0, 30.0), vec![100, 500]);
    }
    #[test]
    fn detects_bursts_preserves_source_and_does_not_cancel_stereo() {
        let mut samples = vec![0.0; 2000];
        for frame in (100..160).chain(500..580) {
            samples[frame * 2] = 0.8;
            samples[frame * 2 + 1] = -0.8;
        }
        let clip = AudioClip {
            channels: 2,
            sample_rate: 1000,
            duration_seconds: 1.0,
            samples: samples.clone().into(),
        };
        assert_eq!(detect(&clip, 0.6, 50.0), vec![100, 500]);
        assert_eq!(clip.samples.as_ref(), samples);
        assert_eq!(detect(&clip, 0.6, 600.0).len(), 1);
    }
    #[test]
    fn silence_and_empty_clips_produce_no_markers() {
        for samples in [vec![], vec![0.0; 100]] {
            let clip = AudioClip {
                channels: 1,
                sample_rate: 48000,
                duration_seconds: 0.0,
                samples: samples.into(),
            };
            assert!(detect(&clip, 1.0, 10.0).is_empty());
        }
    }
}
