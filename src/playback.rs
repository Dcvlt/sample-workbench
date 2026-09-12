use crate::audio::AudioClip;
use rodio::{Source, source::SeekError};
use std::{ops::Range, sync::Arc, time::Duration};

// Playback positions inside the source are relative to the selected region.
// The UI uses absolute clip time; conversion happens at this boundary.
#[derive(Clone)]
struct PlaybackRegion {
    frames: Range<usize>,
    sample_rate: u32,
}

impl PlaybackRegion {
    fn new(clip: &AudioClip, frames: Range<usize>) -> Result<Self, String> {
        let total = clip.samples.len() / usize::from(clip.channels);
        if frames.start >= frames.end || frames.end > total {
            return Err("Choose a nonempty playback region inside the clip".to_owned());
        }
        Ok(Self {
            frames,
            sample_rate: clip.sample_rate,
        })
    }
    fn start(&self) -> f64 {
        self.frames.start as f64 / f64::from(self.sample_rate)
    }
    fn end(&self) -> f64 {
        self.frames.end as f64 / f64::from(self.sample_rate)
    }
    fn duration(&self) -> f64 {
        self.frames.len() as f64 / f64::from(self.sample_rate)
    }
    fn absolute_position(&self, elapsed: f64, looping: bool) -> f64 {
        self.start()
            + if looping {
                elapsed % self.duration()
            } else {
                elapsed.min(self.duration())
            }
    }
    fn relative_seek(&self, absolute: f64, looping: bool) -> f64 {
        let relative = (absolute - self.start()).clamp(0.0, self.duration());
        if looping && relative >= self.duration() {
            0.0
        } else {
            relative
        }
    }
}

struct ClipSource {
    samples: Arc<[f32]>,
    channels: u16,
    region: PlaybackRegion,
    cursor: usize,
    looping: bool,
}

impl ClipSource {
    fn new(clip: &AudioClip, region: PlaybackRegion, looping: bool) -> Self {
        Self {
            samples: Arc::clone(&clip.samples),
            channels: clip.channels,
            cursor: region.frames.start * usize::from(clip.channels),
            region,
            looping,
        }
    }
}

impl Iterator for ClipSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let channels = usize::from(self.channels);
        if self.cursor >= self.region.frames.end * channels {
            if !self.looping {
                return None;
            }
            self.cursor = self.region.frames.start * channels;
        }
        let sample = self.samples[self.cursor];
        self.cursor += 1;
        Some(sample)
    }
}

impl Source for ClipSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.region.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        (!self.looping).then(|| Duration::from_secs_f64(self.region.duration()))
    }
    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let channels = usize::from(self.channels);
        let frames = self.region.frames.len();
        let target = (position.as_secs_f64() * f64::from(self.region.sample_rate)).round() as usize;
        // A seek can arrive between left and right samples: keep channel order.
        let next_channel = self.cursor % channels;
        let relative_frame = if self.looping && target >= frames {
            0
        } else {
            target.min(frames)
        };
        self.cursor = ((self.region.frames.start + relative_frame) * channels + next_channel)
            .min(self.region.frames.end * channels);
        Ok(())
    }
}

pub(crate) struct Playback {
    pub(crate) sink: rodio::Sink,
    stream: rodio::OutputStream,
    region: PlaybackRegion,
    looping: bool,
}

impl Playback {
    pub(crate) fn new(
        clip: &AudioClip,
        gain: f32,
        looping: bool,
        start: f64,
        frames: Range<usize>,
    ) -> Result<Self, String> {
        let region = PlaybackRegion::new(clip, frames)?;
        let stream =
            rodio::OutputStreamBuilder::open_default_stream().map_err(|e| e.to_string())?;
        let sink = Self::make_sink(&stream, clip, &region, gain, looping, start, false)?;
        Ok(Self {
            sink,
            stream,
            region,
            looping,
        })
    }
    fn make_sink(
        stream: &rodio::OutputStream,
        clip: &AudioClip,
        region: &PlaybackRegion,
        gain: f32,
        looping: bool,
        start: f64,
        paused: bool,
    ) -> Result<rodio::Sink, String> {
        let sink = rodio::Sink::connect_new(stream.mixer());
        sink.pause();
        sink.set_volume(gain);
        sink.append(ClipSource::new(clip, region.clone(), looping));
        let relative = region.relative_seek(start, looping);
        if relative > 0.0 {
            sink.try_seek(Duration::from_secs_f64(relative))
                .map_err(|e| e.to_string())?;
        }
        if !paused {
            sink.play();
        }
        Ok(sink)
    }
    // Reuse the audio device when the user commits a selection or toggles Loop.
    pub(crate) fn reconfigure(
        &mut self,
        clip: &AudioClip,
        frames: Range<usize>,
        looping: bool,
        gain: f32,
        start: f64,
    ) -> Result<(), String> {
        let region = PlaybackRegion::new(clip, frames)?;
        let paused = self.sink.is_paused();
        // Build the replacement paused so it cannot overlap the old source.
        let sink = Self::make_sink(&self.stream, clip, &region, gain, looping, start, true)?;
        self.sink.stop();
        self.sink = sink;
        self.region = region;
        self.looping = looping;
        if !paused {
            self.sink.play();
        }
        Ok(())
    }
    pub(crate) fn position(&self) -> f64 {
        if self.sink.empty() {
            self.region.end()
        } else {
            self.region
                .absolute_position(self.sink.get_pos().as_secs_f64(), self.looping)
        }
    }
    pub(crate) fn seek(&self, absolute: f64) -> Result<f64, String> {
        let relative = self.region.relative_seek(absolute, self.looping);
        self.sink
            .try_seek(Duration::from_secs_f64(relative))
            .map_err(|e| e.to_string())?;
        Ok(self.region.start() + relative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clip() -> AudioClip {
        AudioClip {
            channels: 2,
            sample_rate: 2,
            duration_seconds: 2.0,
            samples: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8].into(),
        }
    }
    fn source(looping: bool) -> ClipSource {
        let clip = clip();
        ClipSource::new(&clip, PlaybackRegion::new(&clip, 1..3).unwrap(), looping)
    }
    #[test]
    fn looping_source_plays_the_rendered_edits() {
        let preview = crate::edits::render(
            &clip(),
            &crate::edits::EditState {
                silences: std::iter::once(1..2).collect(),
                fade_frames: 0,
                deletions: Vec::new(),
            },
        );
        let source = ClipSource::new(&preview, PlaybackRegion::new(&preview, 1..3).unwrap(), true);
        assert_eq!(
            source.take(8).collect::<Vec<_>>(),
            vec![0., 0., 0.5, 0.6, 0., 0., 0.5, 0.6]
        );
    }
    #[test]
    fn finite_region_excludes_surrounding_samples() {
        assert_eq!(source(false).collect::<Vec<_>>(), vec![0.3, 0.4, 0.5, 0.6]);
    }
    #[test]
    fn selection_loops_exactly_without_adjacent_frames() {
        assert_eq!(
            source(true).take(10).collect::<Vec<_>>(),
            vec![0.3, 0.4, 0.5, 0.6, 0.3, 0.4, 0.5, 0.6, 0.3, 0.4]
        );
    }
    #[test]
    fn seek_preserves_stereo_order_and_wraps_at_selection_end() {
        let mut source = source(true);
        assert_eq!(source.next(), Some(0.3));
        source.try_seek(Duration::from_millis(500)).unwrap();
        assert_eq!(source.next(), Some(0.6));
        assert_eq!(source.next(), Some(0.3));
        source.try_seek(Duration::from_secs(10)).unwrap();
        assert_eq!(source.next(), Some(0.4));
    }
    #[test]
    fn region_timeline_uses_absolute_clip_time() {
        let region = PlaybackRegion::new(&clip(), 1..3).unwrap();
        assert_eq!(region.absolute_position(2.25, true), 0.75);
        assert_eq!(region.absolute_position(2.25, false), 1.5);
        assert_eq!(region.relative_seek(0.0, true), 0.0);
        assert_eq!(region.relative_seek(1.0, true), 0.5);
        assert_eq!(region.relative_seek(1.5, true), 0.0);
        assert_eq!(region.relative_seek(2.0, false), 1.0);
    }
    #[test]
    fn one_frame_region_loops_and_shares_audio() {
        let clip = clip();
        let source = ClipSource::new(&clip, PlaybackRegion::new(&clip, 2..3).unwrap(), true);
        assert!(Arc::ptr_eq(&source.samples, &clip.samples));
        assert_eq!(source.take(4).collect::<Vec<_>>(), vec![0.5, 0.6, 0.5, 0.6]);
    }
    #[test]
    fn invalid_or_empty_regions_are_rejected() {
        for frames in [0..0, 0..5, Range { start: 3, end: 1 }] {
            assert!(PlaybackRegion::new(&clip(), frames).is_err());
        }
        let mut clip = clip();
        clip.samples = vec![].into();
        assert!(PlaybackRegion::new(&clip, 0..0).is_err());
    }
}
