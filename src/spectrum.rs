use crate::audio::AudioClip;

pub(crate) struct Spectrum {
    pub(crate) fft_size: usize,
    pub(crate) hop: usize,
    pub(crate) columns: usize,
    pub(crate) sample_rate: u32,
    // Column-major amplitude dBFS. Channels are combined by power, not phase.
    pub(crate) db: Vec<f32>,
}

// In-place radix-2 FFT. Kept independent of the GUI and audio device.
fn fft(real: &mut [f32], imaginary: &mut [f32]) {
    let n = real.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            real.swap(i, j);
            imaginary.swap(i, j);
        }
    }
    let mut width = 2;
    while width <= n {
        let angle = -std::f32::consts::TAU / width as f32;
        let (step_i, step_r) = angle.sin_cos();
        for start in (0..n).step_by(width) {
            let (mut wr, mut wi) = (1.0, 0.0);
            for offset in 0..width / 2 {
                let a = start + offset;
                let b = a + width / 2;
                let tr = wr * real[b] - wi * imaginary[b];
                let ti = wr * imaginary[b] + wi * real[b];
                real[b] = real[a] - tr;
                imaginary[b] = imaginary[a] - ti;
                real[a] += tr;
                imaginary[a] += ti;
                (wr, wi) = (wr * step_r - wi * step_i, wr * step_i + wi * step_r);
            }
        }
        width *= 2;
    }
}

pub(crate) fn analyze(clip: &AudioClip, fft_size: usize) -> Spectrum {
    assert!(fft_size.is_power_of_two() && fft_size >= 2);
    let channels = usize::from(clip.channels);
    let frames = clip.samples.len() / channels;
    let bins = fft_size / 2 + 1;
    // Bound display-analysis storage to ~64 MiB on long files. Expose the hop
    // in the UI: larger hops trade time detail for bounded memory.
    let max_columns = (64 * 1024 * 1024 / (bins * 4)).max(1);
    let hop = (fft_size / 8).max(frames.div_ceil(max_columns)).max(1);
    let columns = frames.div_ceil(hop);
    let window: Vec<f32> = (0..fft_size)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / fft_size as f32).cos())
        .collect();
    let normalization = 2.0 / window.iter().sum::<f32>();
    let mut real = vec![0.0; fft_size];
    let mut imaginary = vec![0.0; fft_size];
    let mut power = vec![0.0; bins];
    let mut db = Vec::with_capacity(columns * bins);
    for column in 0..columns {
        power.fill(0.0);
        for channel in 0..channels {
            for i in 0..fft_size {
                // Center the analysis window on its timeline timestamp.
                let frame = (column * hop + i).checked_sub(fft_size / 2);
                real[i] = frame
                    .filter(|&f| f < frames)
                    .map_or(0.0, |f| clip.samples[f * channels + channel])
                    * window[i];
            }
            imaginary.fill(0.0);
            fft(&mut real, &mut imaginary);
            for bin in 0..bins {
                let scale = if bin == 0 || bin == bins - 1 {
                    normalization * 0.5
                } else {
                    normalization
                };
                power[bin] +=
                    (real[bin].powi(2) + imaginary[bin].powi(2)) * scale.powi(2) / channels as f32;
            }
        }
        db.extend(
            power
                .iter()
                .map(|&value| (10.0 * value.max(1.0e-12).log10()).max(-120.0)),
        );
    }
    Spectrum {
        fft_size,
        hop,
        columns,
        sample_rate: clip.sample_rate,
        db,
    }
}

impl Spectrum {
    pub(crate) fn magnitude_db(&self, frame: usize, frequency: f32) -> f32 {
        if self.columns == 0 {
            return -120.0;
        }
        let column = (frame / self.hop).min(self.columns - 1);
        let bin = (frequency * self.fft_size as f32 / self.sample_rate as f32).round() as usize;
        self.db[column * (self.fft_size / 2 + 1) + bin.min(self.fft_size / 2)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centered_window_keeps_impulse_on_timeline() {
        let mut samples = vec![0.0; 8192];
        samples[4096] = 1.0;
        let clip = AudioClip {
            channels: 1,
            sample_rate: 48000,
            duration_seconds: 8192.0 / 48000.0,
            samples: samples.into(),
        };
        let data = analyze(&clip, 1024);
        let peak = (0..data.columns)
            .max_by(|&a, &b| data.db[a * 513 + 10].total_cmp(&data.db[b * 513 + 10]))
            .unwrap();
        assert_eq!(peak * data.hop, 4096);
    }
    #[test]
    fn fft_matches_direct_transform() {
        let input = [0.1, 0.2, -0.8, 0.4, 0.2, -0.1, 0.7, 0.0];
        let mut real = input;
        let mut imaginary = [0.0; 8];
        fft(&mut real, &mut imaginary);
        for k in 0..8 {
            let mut expected = (0.0, 0.0);
            for (i, value) in input.iter().enumerate() {
                let (sin, cos) = (-std::f32::consts::TAU * (k * i) as f32 / 8.0).sin_cos();
                expected.0 += value * cos;
                expected.1 += value * sin;
            }
            assert!((real[k] - expected.0).abs() < 0.00001);
            assert!((imaginary[k] - expected.1).abs() < 0.00001);
        }
    }
    #[test]
    fn tone_frequency_level_and_opposite_phase_stereo_are_correct() {
        let samples = (0..8192)
            .flat_map(|i| {
                let v = 0.5 * (std::f32::consts::TAU * i as f32 / 16.0).sin();
                [v, -v]
            })
            .collect::<Vec<_>>();
        let clip = AudioClip {
            channels: 2,
            sample_rate: 16384,
            duration_seconds: 0.5,
            samples: samples.into(),
        };
        let data = analyze(&clip, 1024);
        assert!((data.magnitude_db(4096, 1024.0) + 6.0206).abs() < 0.05);
        assert!(data.magnitude_db(4096, 3000.0) < -90.0);
    }
    #[test]
    fn silence_and_short_clip_stay_finite() {
        for samples in [vec![], vec![0.0; 3]] {
            let clip = AudioClip {
                channels: 1,
                sample_rate: 48000,
                duration_seconds: 0.0,
                samples: samples.into(),
            };
            let data = analyze(&clip, 2048);
            assert!(data.db.iter().all(|n| n.is_finite() && *n == -120.0));
        }
    }
}
