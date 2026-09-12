use std::{path::Path, sync::Arc};

#[derive(Clone)]
pub(crate) struct AudioClip {
    pub(crate) channels: u16,
    pub(crate) sample_rate: u32,
    pub(crate) duration_seconds: f64,
    pub(crate) samples: Arc<[f32]>,
}

pub(crate) fn load_audio(path: &Path) -> Result<AudioClip, hound::Error> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    if spec.sample_rate == 0 || spec.channels == 0 {
        return Err(hound::Error::FormatError(
            "sample rate and channel count must be nonzero",
        ));
    }

    let mut samples: Vec<f32> = Vec::new();

    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                samples.push(sample?);
            }
        }

        hound::SampleFormat::Int => {
            if spec.bits_per_sample == 0 || spec.bits_per_sample > 32 {
                return Err(hound::Error::FormatError("unsupported integer bit depth"));
            }

            let scale = 2.0_f32.powi(i32::from(spec.bits_per_sample) - 1);

            for sample in reader.samples::<i32>() {
                samples.push(sample? as f32 / scale);
            }
        }
    }

    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(hound::Error::FormatError(
            "audio contains non-finite samples",
        ));
    }

    if !samples.len().is_multiple_of(usize::from(spec.channels)) {
        return Err(hound::Error::FormatError("incomplete audio frame"));
    }
    let frame_count = samples.len() / usize::from(spec.channels);
    let duration_seconds = frame_count as f64 / f64::from(spec.sample_rate);

    Ok(AudioClip {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        duration_seconds,
        samples: samples.into(),
    })
}
