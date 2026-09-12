use crate::audio::AudioClip;
use std::ops::Range;

#[derive(Clone, Default, PartialEq)]
pub(crate) struct EditState {
    pub(crate) silences: Vec<Range<usize>>,
    pub(crate) fade_frames: usize,
    pub(crate) deletions: Vec<Range<usize>>,
}

impl EditState {
    pub(crate) fn visible_frame(&self, frames: usize, source: usize) -> Option<usize> {
        let mut offset = 0;
        for kept in self.kept_ranges(frames) {
            if kept.contains(&source) {
                return Some(offset + source - kept.start);
            }
            offset += kept.len();
        }
        None
    }
    fn kept_ranges(&self, frames: usize) -> Vec<Range<usize>> {
        let mut cuts = self.deletions.clone();
        cuts.sort_by_key(|r| r.start);
        let mut cursor = 0;
        let mut kept = Vec::new();
        for cut in cuts {
            let start = cut.start.min(frames);
            let end = cut.end.min(frames);
            if start >= end {
                continue;
            }
            if cursor < start {
                kept.push(cursor..start);
            }
            cursor = cursor.max(end);
        }
        if cursor < frames {
            kept.push(cursor..frames);
        }
        kept
    }

    // Translate the shortened timeline back to immutable source coordinates.
    pub(crate) fn source_ranges(
        &self,
        frames: usize,
        selection: Range<usize>,
    ) -> Vec<Range<usize>> {
        let mut offset = 0;
        let mut result = Vec::new();
        for kept in self.kept_ranges(frames) {
            let start = selection.start.max(offset);
            let end = selection.end.min(offset + kept.len());
            if start < end {
                result.push(kept.start + start - offset..kept.start + end - offset);
            }
            offset += kept.len();
        }
        result
    }
}

#[derive(Default)]
pub(crate) struct EditHistory {
    pub(crate) current: EditState,
    undo: Vec<EditState>,
    redo: Vec<EditState>,
}

impl EditHistory {
    pub(crate) fn commit(&mut self, next: EditState) -> bool {
        if self.current == next {
            return false;
        }
        self.undo.push(std::mem::replace(&mut self.current, next));
        self.redo.clear();
        true
    }
    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub(crate) fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.current, previous));
        true
    }
    pub(crate) fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.current, next));
        true
    }
}

// Render once per committed edit, never in the audio callback. Every consumer
// receives this same immutable clip; the original Arc is retained by the app.
pub(crate) fn render(source: &AudioClip, edits: &EditState) -> AudioClip {
    if edits.silences.is_empty() && edits.deletions.is_empty() {
        return source.clone();
    }
    let channels = usize::from(source.channels);
    let frames = source.samples.len() / channels;
    let mut envelope = vec![1.0_f32; frames];
    for range in &edits.silences {
        let start = range.start.min(frames);
        let end = range.end.min(frames);
        if start >= end {
            continue;
        }
        envelope[start..end].fill(0.0);
        let fade = edits.fade_frames;
        if fade == 0 {
            continue;
        }
        // Fades sit OUTSIDE the selection, which remains completely silent.
        // Minimum gain combines overlaps without making earlier edits louder.
        for (frame, gain) in envelope
            .iter_mut()
            .enumerate()
            .take(start)
            .skip(start.saturating_sub(fade))
        {
            *gain = gain.min((start - frame) as f32 / fade as f32);
        }
        for (frame, gain) in envelope
            .iter_mut()
            .enumerate()
            .take(end.saturating_add(fade).min(frames))
            .skip(end)
        {
            *gain = gain.min((frame - end) as f32 / fade as f32);
        }
    }
    let mut samples = Vec::new();
    for kept in edits.kept_ranges(frames) {
        for frame in kept {
            for channel in 0..channels {
                samples.push(source.samples[frame * channels + channel] * envelope[frame]);
            }
        }
    }
    AudioClip {
        duration_seconds: (samples.len() / channels) as f64 / f64::from(source.sample_rate),
        samples: samples.into(),
        ..source.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_deletions_and_silence_use_original_coordinates() {
        let source = AudioClip {
            channels: 2,
            sample_rate: 10,
            duration_seconds: 1.0,
            samples: (0..10)
                .flat_map(|n| [n as f32, -(n as f32)])
                .collect::<Vec<_>>()
                .into(),
        };
        let mut state = EditState::default();
        state.deletions.extend(state.source_ranges(10, 2..4));
        assert_eq!(state.source_ranges(10, 1..4), vec![1..2, 4..6]);
        state.silences.extend(state.source_ranges(10, 2..3));
        state.deletions.extend(state.source_ranges(10, 3..5));
        let result = render(&source, &state);
        assert_eq!(
            result.samples.as_ref(),
            &[0., 0., 1., -1., 0., 0., 7., -7., 8., -8., 9., -9.]
        );
        assert_eq!(result.duration_seconds, 0.6);
        state.deletions.extend(state.source_ranges(10, 0..6));
        let empty = render(&source, &state);
        assert!(empty.samples.is_empty());
        assert_eq!(empty.duration_seconds, 0.0);
        assert_eq!(source.samples.len(), 20);
    }
    #[test]
    fn silence_preserves_timeline_source_and_stereo_with_external_fades() {
        let source = AudioClip {
            channels: 2,
            sample_rate: 1000,
            duration_seconds: 0.010,
            samples: [1.0, -0.5].repeat(10).into(),
        };
        let edited = render(
            &source,
            &EditState {
                silences: std::iter::once(4..6).collect(),
                fade_frames: 2,
                deletions: Vec::new(),
            },
        );
        let gains = [1.0, 1.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0];
        for (frame, gain) in gains.into_iter().enumerate() {
            assert_eq!(
                &edited.samples[frame * 2..frame * 2 + 2],
                &[gain, -0.5 * gain]
            );
        }
        assert_eq!(source.samples.as_ref(), [1.0, -0.5].repeat(10));
        assert_eq!(edited.duration_seconds, source.duration_seconds);
    }
    #[test]
    fn overlaps_edges_and_zero_fades_are_safe() {
        let source = AudioClip {
            channels: 1,
            sample_rate: 10,
            duration_seconds: 1.0,
            samples: vec![1.0; 10].into(),
        };
        let state = EditState {
            silences: vec![0..3, 2..5, 8..10],
            fade_frames: 0,
            deletions: Vec::new(),
        };
        assert_eq!(
            render(&source, &state).samples.as_ref(),
            &[0., 0., 0., 0., 0., 1., 1., 1., 0., 0.]
        );
        let state = EditState {
            fade_frames: 100,
            deletions: Vec::new(),
            ..state
        };
        assert!(
            render(&source, &state)
                .samples
                .iter()
                .all(|s| (0.0..=1.0).contains(s))
        );
    }
    #[test]
    fn history_restores_edits_and_new_branch_discards_redo() {
        let mut history = EditHistory::default();
        let state = EditState {
            silences: std::iter::once(3..6).collect(),
            fade_frames: 2,
            deletions: Vec::new(),
        };
        assert!(history.commit(state.clone()));
        assert!(!history.commit(state.clone()));
        assert!(history.undo());
        assert!(history.current.silences.is_empty());
        assert!(history.redo());
        assert!(history.current == state);
        history.undo();
        history.commit(EditState {
            silences: std::iter::once(1..2).collect(),
            fade_frames: 0,
            deletions: Vec::new(),
        });
        assert!(!history.can_redo());
    }
}
