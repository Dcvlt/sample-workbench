use crate::audio::AudioClip;
use std::{
    fs::OpenOptions,
    io::{BufWriter, Seek, Write},
    ops::Range,
    path::Path,
};

// The generic writer supports both real files and in-memory round-trip tests.
fn write_region<W: Write + Seek>(
    output: W,
    clip: &AudioClip,
    frames: Range<usize>,
    gain: f32,
) -> Result<(), hound::Error> {
    let channels = usize::from(clip.channels);
    let total_frames = clip.samples.len() / channels;
    if frames.start >= frames.end || frames.end > total_frames {
        return Err(hound::Error::FormatError(
            "select a nonempty region within the clip",
        ));
    }
    if !gain.is_finite() || !(0.0..=1.0).contains(&gain) {
        return Err(hound::Error::FormatError(
            "gain must be between zero and one",
        ));
    }
    let spec = hound::WavSpec {
        channels: clip.channels,
        sample_rate: clip.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::new(output, spec)?;
    for &sample in &clip.samples[frames.start * channels..frames.end * channels] {
        writer.write_sample(sample * gain)?;
    }
    // Explicit finalization reports errors while updating the WAV header.
    writer.finalize()
}

pub(crate) fn export_region(
    path: &Path,
    clip: &AudioClip,
    frames: Range<usize>,
    gain: f32,
) -> Result<(), String> {
    // Never truncate an existing file (including the original recording).
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Could not create export: {error}. Choose a new filename."))?;
    if let Err(error) = write_region(BufWriter::new(file), clip, frames, gain) {
        // Only remove the new, incomplete output created by this operation.
        let cleanup = std::fs::remove_file(path);
        return Err(match cleanup {
            Ok(()) => format!("Could not export WAV: {error}"),
            Err(cleanup_error) => {
                format!("Could not export WAV: {error}. Incomplete file remains: {cleanup_error}")
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn clip() -> AudioClip {
        AudioClip {
            channels: 2,
            sample_rate: 48000,
            duration_seconds: 3.0 / 48000.0,
            samples: vec![0.1, -0.1, 0.8, -0.6, 1.2, -1.4].into(),
        }
    }
    #[test]
    fn export_round_trip_preserves_channels_range_and_gain() {
        let clip = clip();
        let original = clip.samples.clone();
        let mut bytes = Cursor::new(Vec::new());
        write_region(&mut bytes, &clip, 1..3, 0.5).unwrap();
        bytes.set_position(0);
        let mut reader = hound::WavReader::new(bytes).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 48000);
        assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
        assert_eq!(reader.duration(), 2);
        assert_eq!(
            reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![0.4, -0.3, 0.6, -0.7]
        );
        assert_eq!(clip.samples, original);
    }
    #[test]
    fn exported_edits_match_rendered_preview() {
        let original = clip();
        let preview = crate::edits::render(
            &original,
            &crate::edits::EditState {
                silences: std::iter::once(1..2).collect(),
                fade_frames: 0,
                deletions: Vec::new(),
                timeline: None,
                inserted: Vec::new(),
            },
        );
        let mut bytes = Cursor::new(Vec::new());
        write_region(&mut bytes, &preview, 0..3, 0.5).unwrap();
        bytes.set_position(0);
        let mut reader = hound::WavReader::new(bytes).unwrap();
        let samples = reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(samples, vec![0.05, -0.05, 0.0, 0.0, 0.6, -0.7]);
    }
    #[test]
    fn exported_timing_is_the_auditioned_sample_data() {
        let original = clip();
        let state = crate::edits::quantize(&Default::default(), 3, &[(1, 2)], 0).unwrap();
        let rendered = crate::edits::render(&original, &state);
        let mut bytes = Cursor::new(Vec::new());
        write_region(&mut bytes, &rendered, 0..3, 1.0).unwrap();
        bytes.set_position(0);
        let samples = hound::WavReader::new(bytes)
            .unwrap()
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(samples, rendered.samples.as_ref());
        assert_eq!(&samples[4..6], &original.samples[2..4]);
    }
    #[test]
    fn invalid_regions_and_gain_are_rejected() {
        for range in [0..0, Range { start: 2, end: 1 }, 0..4] {
            assert!(write_region(Cursor::new(Vec::new()), &clip(), range, 1.0).is_err());
        }
        assert!(write_region(Cursor::new(Vec::new()), &clip(), 0..3, f32::NAN).is_err());
    }
}
