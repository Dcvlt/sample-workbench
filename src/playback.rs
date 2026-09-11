use crate::audio::AudioClip;
use rodio::Source;

pub(crate) struct Playback {
    pub(crate) sink: rodio::Sink,
    _stream: rodio::OutputStream,
}

impl Playback {
    pub(crate) fn new(
        clip: &AudioClip,
        gain: f32,
        looping: bool,
    ) -> Result<Self, rodio::StreamError> {
        let stream = rodio::OutputStreamBuilder::open_default_stream()?;
        let sink = rodio::Sink::connect_new(stream.mixer());

        let source = rodio::buffer::SamplesBuffer::new(
            clip.channels,
            clip.sample_rate,
            clip.samples.clone(),
        );

        sink.set_volume(gain);
        if looping {
            sink.append(source.repeat_infinite());
        } else {
            sink.append(source);
        }
        Ok(Self {
            sink,
            _stream: stream,
        })
    }
}
