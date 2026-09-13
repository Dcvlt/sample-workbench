use crate::audio::AudioClip;
use std::ops::Range;

// A piece references immutable source frames; None represents inserted silence.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Piece {
    pub source: Option<usize>,
    pub len: usize,
    pub fade_in: usize,
    pub fade_out: usize,
}

pub(crate) fn slice(pieces: &[Piece], range: Range<usize>) -> Vec<Piece> {
    let mut offset = 0;
    let mut result = Vec::new();
    for piece in pieces {
        let start = range.start.max(offset);
        let end = range.end.min(offset + piece.len);
        if start < end {
            let skip = start - offset;
            let len = end - start;
            result.push(Piece {
                source: piece.source.map(|source| source + skip),
                len,
                fade_in: piece.fade_in.saturating_sub(skip).min(len),
                fade_out: piece
                    .fade_out
                    .saturating_sub(offset + piece.len - end)
                    .min(len),
            });
        }
        offset += piece.len;
    }
    result
}

#[derive(Clone, Default, PartialEq)]
pub(crate) struct EditState {
    pub(crate) silences: Vec<Range<usize>>,
    pub(crate) fade_frames: usize,
    pub(crate) deletions: Vec<Range<usize>>,
    pub(crate) timeline: Option<Vec<Piece>>,
    pub(crate) inserted: Vec<f32>,
}

impl EditState {
    // Give inserted silence stable source coordinates too. Markers may sit
    // anywhere on the timeline, including the lead-in before an attack.
    pub(crate) fn materialize_gaps(&mut self, frames: usize, channels: usize) -> bool {
        let Some(pieces) = &mut self.timeline else {
            return false;
        };
        let mut changed = false;
        for piece in pieces {
            if piece.source.is_none() {
                piece.source = Some(frames + self.inserted.len() / channels);
                self.inserted
                    .resize(self.inserted.len() + piece.len * channels, 0.0);
                changed = true;
            }
        }
        changed
    }
    pub(crate) fn apply_edge_fades(&mut self, frames: usize, fade: usize) {
        self.fade_frames = fade;
        let mut pieces = self.pieces(frames);
        for index in 0..pieces.len() {
            if pieces[index].source.is_none() {
                continue;
            }
            let continuous_before = index > 0
                && pieces[index - 1].source.is_some_and(|start| {
                    Some(start + pieces[index - 1].len) == pieces[index].source
                });
            let continuous_after = pieces.get(index + 1).is_some_and(|next| {
                pieces[index].source.map(|start| start + pieces[index].len) == next.source
            });
            let length = fade.min(pieces[index].len / 2);
            if !continuous_before {
                pieces[index].fade_in = length;
            }
            if !continuous_after {
                pieces[index].fade_out = length;
            }
        }
        self.timeline = Some(pieces);
    }
    pub(crate) fn paste(
        &mut self,
        frames: usize,
        channels: usize,
        range: Range<usize>,
        audio: &[f32],
    ) {
        let pieces = self.pieces(frames);
        let total: usize = pieces.iter().map(|p| p.len).sum();
        let start = range.start.min(total);
        let end = range.end.max(start).min(total);
        let source = frames + self.inserted.len() / channels;
        self.inserted.extend_from_slice(audio);
        let mut next = slice(&pieces, 0..start);
        if !audio.is_empty() {
            next.push(Piece {
                source: Some(source),
                len: audio.len() / channels,
                fade_in: 0,
                fade_out: 0,
            });
        }
        next.extend(slice(&pieces, end..total));
        self.timeline = Some(next);
    }
    pub(crate) fn pieces(&self, frames: usize) -> Vec<Piece> {
        self.timeline.clone().unwrap_or_else(|| {
            self.kept_ranges(frames)
                .into_iter()
                .map(|range| Piece {
                    source: Some(range.start),
                    len: range.len(),
                    fade_in: 0,
                    fade_out: 0,
                })
                .collect()
        })
    }

    pub(crate) fn remove_time(&mut self, frames: usize, selection: Range<usize>) {
        let ranges = self.source_ranges(frames, selection.clone());
        if self.timeline.is_some() {
            let pieces = self.pieces(frames);
            let len = pieces.iter().map(|p| p.len).sum();
            let mut kept = slice(&pieces, 0..selection.start);
            kept.extend(slice(&pieces, selection.end..len));
            self.timeline = Some(kept);
        }
        self.deletions.extend(ranges);
    }

    pub(crate) fn visible_frame(&self, frames: usize, source: usize) -> Option<usize> {
        let mut offset = 0;
        for piece in self.pieces(frames) {
            if let Some(start) = piece.source
                && (start..start + piece.len).contains(&source)
            {
                return Some(offset + source - start);
            }
            offset += piece.len;
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
        for piece in self.pieces(frames) {
            let start = selection.start.max(offset);
            let end = selection.end.min(offset + piece.len);
            if start < end
                && let Some(source) = piece.source
            {
                result.push(source + start - offset..source + end - offset);
            }
            offset += piece.len;
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

// Attacks and targets are on the current rendered timeline. Every attack acts
// as a slice boundary, including unselected attacks which remain stationary.
pub(crate) fn resolve_targets(attacks: &[(usize, usize)], frames: usize) -> Vec<(usize, usize)> {
    let mut resolved = attacks.to_vec();
    for (from, to) in &mut resolved {
        if *to >= frames {
            *to = *from;
        }
    }
    // Restoring a conflicting move can expose a conflict with its other
    // neighbor. Each pass restores at least one move, so this terminates.
    loop {
        let mut restore = Vec::new();
        for (index, pair) in resolved.windows(2).enumerate() {
            if pair[0].1 >= pair[1].1 {
                for candidate in [index, index + 1] {
                    if resolved[candidate].0 != resolved[candidate].1 {
                        restore.push(candidate);
                    }
                }
            }
        }
        if restore.is_empty() {
            break;
        }
        for index in restore {
            resolved[index].1 = resolved[index].0;
        }
    }
    resolved
}

pub(crate) fn quantize(
    state: &EditState,
    source_frames: usize,
    attacks: &[(usize, usize)],
    fade: usize,
) -> Result<EditState, String> {
    let pieces = state.pieces(source_frames);
    let frames: usize = pieces.iter().map(|piece| piece.len).sum();
    if attacks.is_empty() {
        return Ok(state.clone());
    }
    if attacks
        .iter()
        .any(|&(from, to)| from >= frames || to >= frames)
        || attacks
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0 || pair[0].1 >= pair[1].1)
    {
        return Err("Grid targets collide or fall outside the clip. Use a finer grid or select fewer attacks.".into());
    }
    if attacks.iter().all(|&(from, to)| from == to) {
        return Ok(state.clone());
    }
    let mut boundaries = Vec::new();
    for (index, &(from, to)) in attacks.iter().enumerate() {
        let previous = if index == 0 { 0 } else { attacks[index - 1].0 };
        let target_room = if index == 0 {
            to
        } else {
            to - attacks[index - 1].1 - 1
        };
        let pre = fade.min((from - previous) / 2).min(target_room);
        boundaries.push((from - pre, to - pre));
    }
    for index in 1..attacks.len() {
        if boundaries[index].1 <= attacks[index - 1].1 {
            return Err("This grid would cut into a neighboring attack. Use a finer grid.".into());
        }
    }
    let mut regions = vec![(0..boundaries[0].0, 0)];
    for (index, &(start, target)) in boundaries.iter().enumerate() {
        let end = boundaries
            .get(index + 1)
            .map_or(frames, |&(start, _)| start);
        regions.push((start..end, target));
    }
    let mut output = Vec::new();
    let mut cursor = 0;
    for (index, (range, target)) in regions.iter().enumerate() {
        let end = (*target + range.len())
            .min(regions.get(index + 1).map_or(frames, |(_, target)| *target))
            .min(frames);
        if cursor < *target {
            output.push(Piece {
                source: None,
                len: *target - cursor,
                fade_in: 0,
                fade_out: 0,
            });
        }
        let len = end.saturating_sub(*target);
        let mut extracted = slice(&pieces, range.start..range.start + len);
        let delta = *target as i128 - range.start as i128;
        let previous_delta = index
            .checked_sub(1)
            .map(|i| regions[i].1 as i128 - regions[i].0.start as i128);
        let next_delta = regions
            .get(index + 1)
            .map(|(r, t)| *t as i128 - r.start as i128);
        if previous_delta.is_some_and(|previous| previous != delta)
            && let Some(first) = extracted.first_mut()
        {
            let pre = if index == 0 {
                0
            } else {
                attacks[index - 1].1 - *target
            };
            first.fade_in = first.fade_in.max(fade.min(pre).min(first.len));
        }
        if (next_delta.is_some_and(|next| next != delta)
            || end < *target + range.len()
            || (next_delta.is_none() && end < frames))
            && let Some(last) = extracted.last_mut()
        {
            let tail = if index == 0 {
                len
            } else {
                end.saturating_sub(attacks[index - 1].1 + 1)
            };
            last.fade_out = last.fade_out.max(fade.min(tail).min(last.len));
        }
        output.extend(extracted);
        cursor = end.max(*target);
    }
    if cursor < frames {
        output.push(Piece {
            source: None,
            len: frames - cursor,
            fade_in: 0,
            fade_out: 0,
        });
    }
    let mut next = state.clone();
    next.timeline = Some(output);
    next.apply_edge_fades(source_frames, fade);
    Ok(next)
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
    if edits.silences.is_empty() && edits.deletions.is_empty() && edits.timeline.is_none() {
        return source.clone();
    }
    let channels = usize::from(source.channels);
    let frames = source.samples.len() / channels;
    let pool_frames = frames + edits.inserted.len() / channels;
    let mut envelope = vec![1.0_f32; pool_frames];
    for range in &edits.silences {
        let start = range.start.min(pool_frames);
        let end = range.end.min(pool_frames);
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
            .take(end.saturating_add(fade).min(pool_frames))
            .skip(end)
        {
            *gain = gain.min((frame - end) as f32 / fade as f32);
        }
    }
    let mut samples = Vec::new();
    for piece in edits.pieces(frames) {
        for index in 0..piece.len {
            let gain_in = if piece.fade_in == 0 {
                1.0
            } else {
                (index as f32 / piece.fade_in as f32).min(1.0)
            };
            let gain_out = if piece.fade_out == 0 {
                1.0
            } else {
                ((piece.len - 1 - index) as f32 / piece.fade_out as f32).min(1.0)
            };
            for channel in 0..channels {
                samples.push(piece.source.map_or(0.0, |start| {
                    let frame = start + index;
                    (if frame < frames {
                        source.samples[frame * channels + channel]
                    } else {
                        edits.inserted[(frame - frames) * channels + channel]
                    }) * envelope[frame]
                        * {
                            let gain = gain_in.min(gain_out);
                            gain * gain * (3.0 - 2.0 * gain)
                        }
                }));
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
    fn repaired_join_reaches_zero_on_both_sides_and_reduces_step() {
        let source = AudioClip {
            channels: 1,
            sample_rate: 1000,
            duration_seconds: 0.3,
            samples: (0..300)
                .map(|i| if i < 100 { 1.0 } else { -1.0 })
                .collect::<Vec<_>>()
                .into(),
        };
        let mut state = EditState {
            timeline: Some(vec![
                Piece {
                    source: Some(0),
                    len: 100,
                    fade_in: 0,
                    fade_out: 0,
                },
                Piece {
                    source: Some(200),
                    len: 100,
                    fade_in: 0,
                    fade_out: 0,
                },
            ]),
            ..Default::default()
        };
        assert_eq!(render(&source, &state).samples[99], 1.0);
        state.apply_edge_fades(300, 5);
        let output = render(&source, &state);
        assert_eq!(&output.samples[99..101], &[0.0, 0.0]);
        assert!(
            output
                .samples
                .windows(2)
                .all(|pair| (pair[1] - pair[0]).abs() < 0.4)
        );
    }
    #[test]
    fn pasted_stereo_is_independent_of_original_silences() {
        let source = impulses();
        let mut state = EditState::default();
        state.paste(100, 2, 100..100, &[1., -0.5, 0.25, -0.125]);
        state.silences.push(20..21);
        let rendered = render(&source, &state);
        assert_eq!(rendered.samples.len(), 204);
        assert_eq!(&rendered.samples[200..], &[1., -0.5, 0.25, -0.125]);
        state.silences.extend(state.source_ranges(100, 100..101));
        assert_eq!(&render(&source, &state).samples[200..202], &[0., 0.]);
    }
    #[test]
    fn crowded_targets_keep_conflicts_and_apply_independent_moves() {
        let targets = resolve_targets(&[(10, 0), (20, 0), (50, 45), (99, 100)], 100);
        assert_eq!(targets, vec![(10, 10), (20, 20), (50, 45), (99, 99)]);
        let state = quantize(&EditState::default(), 100, &targets, 2).unwrap();
        for (source, target) in targets {
            assert_eq!(state.visible_frame(100, source), Some(target));
        }
    }

    #[test]
    fn restored_targets_cannot_cross_stationary_neighbors() {
        let targets = resolve_targets(&[(10, 50), (20, 50), (30, 30), (80, 75)], 100);
        assert_eq!(targets, vec![(10, 10), (20, 20), (30, 30), (80, 75)]);
        assert!(quantize(&EditState::default(), 100, &targets, 2).is_ok());
    }

    #[test]
    fn close_targets_shorten_preroll_instead_of_rejecting() {
        let state = quantize(&EditState::default(), 100, &[(20, 30), (50, 31)], 10).unwrap();
        assert_eq!(state.visible_frame(100, 20), Some(30));
        assert_eq!(state.visible_frame(100, 50), Some(31));
    }
    fn impulses() -> AudioClip {
        let mut samples = vec![0.0; 200];
        for frame in [20, 50, 80] {
            samples[frame * 2] = 1.0;
            samples[frame * 2 + 1] = -0.5;
        }
        AudioClip {
            samples: samples.into(),
            channels: 2,
            sample_rate: 1000,
            duration_seconds: 0.1,
        }
    }

    #[test]
    fn timing_moves_stereo_attacks_without_stretching_and_keeps_length() {
        let source = impulses();
        let state = quantize(
            &EditState::default(),
            100,
            &[(20, 25), (50, 45), (80, 80)],
            2,
        )
        .unwrap();
        let output = render(&source, &state);
        assert_eq!(output.samples.len(), source.samples.len());
        for (from, to) in [(20, 25), (50, 45), (80, 80)] {
            assert_eq!(&output.samples[to * 2..to * 2 + 2], &[1.0, -0.5]);
            assert_eq!(state.visible_frame(100, from), Some(to));
            assert_eq!(state.source_ranges(100, to..to + 1), vec![from..from + 1]);
        }
        assert_eq!(source.samples[40], 1.0);
        assert_eq!(
            output.samples.iter().filter(|&&value| value == 1.0).count(),
            3
        );
    }

    #[test]
    fn timing_rejects_collisions_and_out_of_bounds_without_mutation() {
        let state = EditState::default();
        for targets in [
            vec![(20, 40), (50, 40)],
            vec![(20, 60), (50, 40)],
            vec![(20, 100)],
        ] {
            assert!(quantize(&state, 100, &targets, 2).is_err());
        }
        assert!(state.timeline.is_none());
        assert!(quantize(&state, 100, &[(20, 20)], 2).unwrap() == state);
    }

    #[test]
    fn timing_supports_prior_cuts_later_cuts_and_silence_gaps() {
        let mut state = EditState::default();
        state.remove_time(100, 0..10);
        let mut state = quantize(&state, 100, &[(10, 15), (40, 35), (70, 70)], 2).unwrap();
        assert_eq!(state.visible_frame(100, 20), Some(15));
        // The five inserted silent frames can be removed like ordinary audio.
        assert!(state.source_ranges(100, 8..13).is_empty());
        state.remove_time(100, 8..13);
        assert_eq!(state.visible_frame(100, 20), Some(10));
        assert_eq!(render(&impulses(), &state).samples.len(), 170);
        state.silences.extend(state.source_ranges(100, 10..11));
        assert_eq!(render(&impulses(), &state).samples[20], 0.0);
    }

    #[test]
    fn timing_history_restores_exact_samples() {
        let source = impulses();
        let mut history = EditHistory::default();
        let next = quantize(&history.current, 100, &[(20, 25), (50, 45), (80, 80)], 2).unwrap();
        history.commit(next);
        let applied = render(&source, &history.current);
        history.undo();
        assert_eq!(render(&source, &history.current).samples, source.samples);
        history.redo();
        assert_eq!(render(&source, &history.current).samples, applied.samples);
    }
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
                timeline: None,
                inserted: Vec::new(),
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
            timeline: None,
            inserted: Vec::new(),
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
            timeline: None,
            inserted: Vec::new(),
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
            timeline: None,
            inserted: Vec::new(),
            fade_frames: 0,
            deletions: Vec::new(),
        });
        assert!(!history.can_redo());
    }
}
