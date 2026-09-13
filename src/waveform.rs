use crate::audio::AudioClip;
use crate::theme;
use eframe::egui;
use std::ops::Range;

pub(crate) enum WaveformAction {
    Seek(f64),
    Select(Range<usize>),
    SelectMarkers(Vec<usize>),
    AddMarkers(Vec<usize>),
}

fn selection_range(anchor: usize, cursor: usize, frames: usize) -> Option<Range<usize>> {
    let start = anchor.min(cursor).min(frames);
    let end = anchor.max(cursor).min(frames);
    (start < end).then_some(start..end)
}

// Keep the frame under the pointer stationary unless we reach a clip edge.
fn zoom_range(view: &Range<usize>, frames: usize, factor: f64, anchor: f64) -> Range<usize> {
    let span = ((view.len() as f64 * factor).round() as usize).clamp(1, frames);
    let pivot = view.start as f64 + anchor * view.len() as f64;
    let start = ((pivot - anchor * span as f64).round().max(0.0) as usize).min(frames - span);
    start..start + span
}

#[derive(Default)]
pub(crate) struct WaveformCache {
    peaks: Vec<(f32, f32)>,
    selection_anchor: Option<usize>,
    view: Option<Range<usize>>,
    cached_view: Option<Range<usize>>,
    pub(crate) markers: Vec<usize>,
    pub(crate) timing_targets: Vec<(usize, usize)>,
    pub(crate) selected_marker: Option<usize>,
    pub(crate) selected_markers: Vec<usize>,
    marker_anchor: Option<usize>,
    spectrum: crate::spectrum_view::SpectrumView,
    pub(crate) detection: Option<std::sync::Arc<crate::transients::Detection>>,
    pub(crate) compare_legacy: bool,
}

impl WaveformCache {
    pub(crate) fn analysis(
        &mut self,
        clip: &AudioClip,
        ctx: &egui::Context,
        required: bool,
    ) -> Option<std::sync::Arc<crate::spectrum::Spectrum>> {
        self.spectrum.required = required;
        self.spectrum.update(clip, ctx);
        if self.spectrum.data.is_some() {
            self.spectrum.required = false;
        }
        self.spectrum.data.clone()
    }
    pub(crate) fn reveal(&mut self, frame: usize, frames: usize) {
        if let Some(view) = &mut self.view
            && !view.contains(&frame)
        {
            let span = view.len().min(frames);
            let start = frame.saturating_sub(span / 2).min(frames - span);
            *view = start..start + span;
        }
    }
    pub(crate) fn invalidate_audio(&mut self) {
        self.spectrum.required = false;
        self.detection = None;
        self.peaks.clear();
        self.cached_view = None;
    }
    // Called when a new clip replaces the source data, even at the same width.
    pub(crate) fn clear(&mut self) {
        self.spectrum.required = false;
        self.detection = None;
        self.peaks.clear();
        self.selection_anchor = None;
        self.view = None;
        self.cached_view = None;
    }

    fn prepare(&mut self, clip: &AudioClip, columns: usize) {
        if self.peaks.len() == columns && self.cached_view == self.view {
            return;
        }
        self.peaks.clear();
        let channels = usize::from(clip.channels);
        let frames = clip.samples.len() / channels;
        let view = self.view.clone().unwrap_or(0..frames);
        self.cached_view = self.view.clone();
        for column in 0..columns {
            let start = (view.start + column * view.len() / columns) * channels;
            let end = (view.start + (column + 1) * view.len() / columns) * channels;
            let mut low = f32::INFINITY;
            let mut high = f32::NEG_INFINITY;
            for &sample in &clip.samples[start..end] {
                low = low.min(sample);
                high = high.max(sample);
            }
            self.peaks.push((low, high));
        }
    }

    pub(crate) fn controls(
        &mut self,
        ui: &mut egui::Ui,
        clip: &AudioClip,
        selection: Option<&Range<usize>>,
    ) {
        let frames = clip.samples.len() / usize::from(clip.channels);
        if frames == 0 {
            return;
        }
        let mut view = self.view.clone().unwrap_or(0..frames);
        self.spectrum.controls(ui);
        ui.horizontal_wrapped(|ui| {
            theme::segments(ui, |ui| {
                if ui.button("Fit clip").clicked() {
                    view = 0..frames;
                }
                if ui
                    .add_enabled(selection.is_some(), egui::Button::new("Fit selection"))
                    .clicked()
                    && let Some(selection) = selection
                {
                    view = selection.clone();
                }
            });
            if ui.button("−").on_hover_text("Zoom out").clicked() {
                view = zoom_range(&view, frames, 2.0, 0.5);
            }
            if ui.button("+").on_hover_text("Zoom in").clicked() {
                view = zoom_range(&view, frames, 0.5, 0.5);
            }
            ui.label(format!("{:.1}×", frames as f64 / view.len() as f64));
            ui.label(
                egui::RichText::new("Scroll: zoom · Middle/right drag: pan")
                    .small()
                    .color(theme::MUTED),
            );
        });
        self.view = Some(view);
    }

    // Emit a typed interaction; the app owns selection and playback state.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        clip: &AudioClip,
        gain: f32,
        progress: f32,
        selection: Option<&Range<usize>>,
        grid_bpm: f32,
        grid_subdivision: u32,
        grid_enabled: bool,
    ) -> Option<WaveformAction> {
        let frames = clip.samples.len() / usize::from(clip.channels);
        if frames == 0 {
            return None;
        }
        let mut view = self.view.clone().unwrap_or(0..frames);
        self.spectrum.update(clip, ui.ctx());
        let span = view.len();
        if span < frames {
            let mut start = view.start;
            if ui
                .add(
                    egui::Slider::new(&mut start, 0..=frames - span)
                        .show_value(false)
                        .text("Pan"),
                )
                .changed()
            {
                view = start..start + span;
            }
        }
        let height = (ui.available_height() - 52.0).clamp(80.0, 640.0);
        let size = egui::vec2(ui.available_width().max(48.0), height);
        let (outer, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, 3.0, egui::Color32::from_rgb(16, 17, 19));
        painter.rect_stroke(
            outer,
            10.0,
            egui::Stroke::new(1.0_f32, theme::LINE),
            egui::StrokeKind::Inside,
        );
        let lane_height = if self.detection.is_some() && outer.height() >= 140.0 {
            (outer.height() * 0.32).clamp(25.0, 100.0)
        } else {
            0.0
        };
        let rect = egui::Rect::from_min_max(
            outer.min + egui::vec2(18.0, 36.0),
            outer.max - egui::vec2(18.0, 24.0 + lane_height),
        );
        if response.dragged_by(egui::PointerButton::Middle)
            || response.dragged_by(egui::PointerButton::Secondary)
        {
            let delta = ui.input(|i| i.pointer.delta().x);
            let start = (view.start as f64 - f64::from(delta / rect.width()) * view.len() as f64)
                .round()
                .clamp(0.0, (frames - view.len()) as f64) as usize;
            view = start..start + view.len();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if response.hovered() && !ui.input(|i| i.pointer.primary_down()) {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0
                && let Some(pointer) = response.hover_pos()
            {
                let anchor = f64::from(((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
                view = zoom_range(&view, frames, (-f64::from(scroll) * 0.005).exp(), anchor);
                ui.ctx().request_repaint();
            }
        }
        self.view = Some(view.clone());
        let frame_x = |frame: f64| {
            rect.left() + ((frame - view.start as f64) / view.len() as f64) as f32 * rect.width()
        };
        if grid_enabled && grid_bpm > 0.0 {
            let step_seconds = 60.0 / f64::from(grid_bpm) / f64::from(grid_subdivision.max(1));
            let rate = f64::from(clip.sample_rate);
            let first = (view.start as f64 / rate / step_seconds).floor() as i64 - 1;
            let last = (view.end as f64 / rate / step_seconds).ceil() as i64 + 1;
            for beat in first..=last {
                let x = frame_x(beat as f64 * step_seconds * rate);
                if x >= rect.left() && x <= rect.right() {
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(
                            1.0_f32,
                            egui::Color32::from_rgba_unmultiplied(255, 97, 58, 72),
                        ),
                    );
                }
            }
            // Show proposed snap destinations without changing the source markers.
            for &(marker, target) in &self.timing_targets {
                let snapped_frame = target as f64;
                let from = frame_x(marker as f64);
                let to = frame_x(snapped_frame);
                if (from - to).abs() > 1.0 && to >= rect.left() && to <= rect.right() {
                    painter.line_segment(
                        [egui::pos2(to, rect.top()), egui::pos2(to, rect.bottom())],
                        egui::Stroke::new(
                            1.0_f32,
                            egui::Color32::from_rgba_unmultiplied(255, 180, 80, 120),
                        ),
                    );
                    painter.line_segment(
                        [
                            egui::pos2(from, rect.top() + 4.0),
                            egui::pos2(to, rect.top() + 4.0),
                        ],
                        egui::Stroke::new(
                            1.0_f32,
                            egui::Color32::from_rgba_unmultiplied(255, 180, 80, 150),
                        ),
                    );
                }
            }
        }
        let ticks = (rect.width() / 100.0).floor().max(2.0) as usize;
        for tick in 0..=ticks {
            let fraction = tick as f32 / ticks as f32;
            let x = rect.left() + fraction * rect.width();
            let align = if tick == 0 {
                egui::Align2::LEFT_CENTER
            } else if tick == ticks {
                egui::Align2::RIGHT_CENTER
            } else {
                egui::Align2::CENTER_CENTER
            };
            painter.text(
                egui::pos2(x, outer.top() + 17.0),
                align,
                format!(
                    "{:.4}s",
                    (view.start as f64 + f64::from(fraction) * view.len() as f64)
                        / f64::from(clip.sample_rate)
                ),
                egui::FontId::monospace(11.0),
                theme::MUTED,
            );
        }
        let show_waveform = self.spectrum.mode != crate::spectrum_view::Mode::Spectrogram;
        let center_y = rect.center().y;
        if show_waveform {
            painter.line_segment(
                [
                    egui::pos2(rect.left(), center_y),
                    egui::pos2(rect.right(), center_y),
                ],
                egui::Stroke::new(1.0_f32, theme::LINE),
            );
        }
        self.spectrum.paint(ui, &painter, rect, &view, gain);
        if let Some(selection) = selection {
            let left = frame_x(selection.start as f64).clamp(rect.left(), rect.right());
            let right = frame_x(selection.end as f64).clamp(rect.left(), rect.right());
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(left, rect.top()),
                    egui::pos2(right, rect.bottom()),
                ),
                0.0,
                egui::Color32::from_rgba_unmultiplied(239, 52, 58, 28),
            );
        }
        if let Some(selection) = selection {
            for frame in [selection.start, selection.end] {
                if frame < view.start || frame > view.end {
                    continue;
                }
                let x = frame_x(frame as f64);
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0_f32, theme::AMBER),
                );
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        egui::pos2(x, rect.top() + 5.0),
                        egui::vec2(5.0, 10.0),
                    ),
                    2.0,
                    theme::AMBER,
                );
            }
        }
        if show_waveform {
            let columns = (rect.width().max(1.0) as usize).min(view.len());
            self.prepare(clip, columns);
            for (column, &(low, high)) in self.peaks.iter().enumerate() {
                let x = rect.left() + (column as f32 + 0.5) / columns as f32 * rect.width();
                // Apply gain before clipping so floating-point audio above 1.0 displays correctly.
                let top = center_y - (high * gain).clamp(-1.0, 1.0) * rect.height() * 0.45;
                let bottom = center_y - (low * gain).clamp(-1.0, 1.0) * rect.height() * 0.45;
                painter.line_segment(
                    [egui::pos2(x, top), egui::pos2(x, bottom)],
                    egui::Stroke::new(1.0_f32, theme::TEXT),
                );
            }
            if view.len() <= rect.width() as usize {
                let channels = usize::from(clip.channels);
                for channel in 0..channels {
                    let points = view
                        .clone()
                        .map(|frame| {
                            let value =
                                (clip.samples[frame * channels + channel] * gain).clamp(-1.0, 1.0);
                            egui::pos2(
                                frame_x(frame as f64),
                                center_y - value * rect.height() * 0.45,
                            )
                        })
                        .collect();
                    painter.add(egui::Shape::line(
                        points,
                        egui::Stroke::new(1.0_f32, theme::TEXT),
                    ));
                }
            }
        }
        let x = frame_x(f64::from(progress.clamp(0.0, 1.0)) * frames as f64);
        if let Some(detection) = &self.detection {
            if self.compare_legacy {
                for &frame in &detection.legacy {
                    if !view.contains(&frame) {
                        continue;
                    }
                    let x = frame_x(frame as f64);
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(120)),
                    );
                    painter.text(
                        egui::pos2(x, rect.top() + 15.0),
                        egui::Align2::LEFT_TOP,
                        "L",
                        egui::FontId::monospace(9.0),
                        theme::MUTED,
                    );
                }
            }
            if lane_height > 0.0 {
                let lane = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.bottom() + 8.0),
                    egui::pos2(rect.right(), outer.bottom() - 8.0),
                );
                painter.rect_filled(lane, 0.0, egui::Color32::from_rgb(18, 19, 21));
                painter.text(
                    lane.left_top() + egui::vec2(4.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    format!(
                        "Spectral strength (red) / local threshold (amber) · log scale · {} FFT",
                        detection.fft_size
                    ),
                    egui::FontId::monospace(10.0),
                    theme::MUTED,
                );
                let first = (view.start / detection.hop).min(detection.strength.len());
                let end = view
                    .end
                    .div_ceil(detection.hop)
                    .min(detection.strength.len());
                let maximum = detection.strength[first..end]
                    .iter()
                    .chain(&detection.threshold[first..end])
                    .copied()
                    .fold(0.001_f32, f32::max);
                let plot = egui::Rect::from_min_max(
                    lane.min + egui::vec2(0.0, 18.0),
                    lane.max - egui::vec2(0.0, 3.0),
                );
                for (values, color) in [
                    (&detection.strength, theme::ACCENT),
                    (&detection.threshold, theme::AMBER),
                ] {
                    let points = (first..end)
                        .map(|i| {
                            egui::pos2(
                                frame_x((i * detection.hop) as f64)
                                    .clamp(rect.left(), rect.right()),
                                plot.bottom() - values[i].ln_1p() / maximum.ln_1p() * plot.height(),
                            )
                        })
                        .collect();
                    painter.add(egui::Shape::line(points, egui::Stroke::new(1.0_f32, color)));
                }
                if x >= lane.left() && x <= lane.right() {
                    painter.line_segment(
                        [egui::pos2(x, lane.top()), egui::pos2(x, lane.bottom())],
                        egui::Stroke::new(1.0_f32, egui::Color32::WHITE),
                    );
                }
            }
        }
        for &marker in &self.markers {
            if marker < view.start || marker >= view.end {
                continue;
            }
            let x = frame_x(marker as f64);
            let color = if self.selected_marker == Some(marker)
                || self.selected_markers.contains(&marker)
            {
                theme::AMBER
            } else {
                egui::Color32::from_rgb(240, 120, 120)
            };
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0_f32, color),
            );
            painter.text(
                egui::pos2(x + 3.0, rect.top() + 3.0),
                egui::Align2::LEFT_TOP,
                "A",
                egui::FontId::monospace(10.0),
                color,
            );
        }
        if x >= rect.left() && x <= rect.right() {
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x - 5.0, rect.top() - 5.0),
                    egui::pos2(x + 5.0, rect.top() - 5.0),
                    egui::pos2(x, rect.top() + 2.0),
                ],
                theme::TEXT,
                egui::Stroke::NONE,
            ));
        }
        let fraction_at = |x: f32| {
            (view.start as f64
                + f64::from(((x - rect.left()) / rect.width()).clamp(0.0, 1.0)) * view.len() as f64)
                / frames as f64
        };
        let nearest_marker = |x: f32| {
            self.markers
                .iter()
                .copied()
                .filter(|&marker| marker >= view.start && marker < view.end)
                .min_by(|&a, &b| {
                    (frame_x(a as f64) - x)
                        .abs()
                        .total_cmp(&(frame_x(b as f64) - x).abs())
                })
                .filter(|&marker| (frame_x(marker as f64) - x).abs() <= 10.0)
        };
        if response.drag_started_by(egui::PointerButton::Primary) && ui.input(|i| i.modifiers.ctrl)
        {
            self.marker_anchor = response
                .interact_pointer_pos()
                .map(|p| (fraction_at(p.x) * frames as f64).round() as usize);
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.selection_anchor = if !ui.input(|input| input.modifiers.ctrl) {
                ui.input(|input| input.pointer.press_origin())
                    .map(|pointer| (fraction_at(pointer.x) * frames as f64).round() as usize)
            } else {
                None
            };
        }
        let mut action = None;
        if (response.clicked_by(egui::PointerButton::Primary)
            || response.dragged_by(egui::PointerButton::Primary)
            || response.drag_stopped_by(egui::PointerButton::Primary))
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let fraction = fraction_at(pointer.x);
            if let Some(anchor) = self.marker_anchor {
                let cursor = (fraction * frames as f64).round() as usize;
                let (start, end) = (anchor.min(cursor), anchor.max(cursor));
                let selected = self
                    .markers
                    .iter()
                    .copied()
                    .filter(|&marker| marker >= start && marker <= end)
                    .collect();
                action = Some(if ui.input(|i| i.modifiers.ctrl) {
                    WaveformAction::AddMarkers(selected)
                } else {
                    WaveformAction::SelectMarkers(selected)
                });
            } else if ui.input(|i| i.modifiers.ctrl)
                && let Some(marker) = nearest_marker(pointer.x)
            {
                action = Some(if ui.input(|i| i.modifiers.ctrl) {
                    WaveformAction::AddMarkers(vec![marker])
                } else {
                    WaveformAction::SelectMarkers(vec![marker])
                });
            } else if let Some(anchor) = self.selection_anchor {
                let cursor = (fraction * frames as f64).round() as usize;
                action = selection_range(anchor, cursor, frames).map(WaveformAction::Select);
            } else if !ui.input(|input| input.modifiers.ctrl) {
                action = Some(WaveformAction::Seek(fraction));
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.selection_anchor = None;
            self.marker_anchor = None;
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_drag_across_markers_selects_audio() {
        let ctx = egui::Context::default();
        let clip = AudioClip {
            channels: 1,
            sample_rate: 100,
            duration_seconds: 10.0,
            samples: vec![0.0; 1000].into(),
        };
        let mut cache = WaveformCache {
            markers: (0..1000).collect(),
            ..Default::default()
        };
        let mut selected = false;
        for (x, down) in [
            (150., None),
            (150., Some(true)),
            (450., None),
            (450., Some(false)),
        ] {
            let pos = egui::pos2(x, 180.);
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if let Some(pressed) = down {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if let Some(action) = cache.show(ui, &clip, 1., 0., None, 120., 1, false) {
                            assert!(!matches!(
                                action,
                                WaveformAction::SelectMarkers(_) | WaveformAction::AddMarkers(_)
                            ));
                            if let WaveformAction::Select(range) = action {
                                assert!(range.len() > 300);
                                selected = true;
                            }
                        }
                    });
                },
            );
        }
        assert!(selected);
    }

    #[test]
    fn middle_and_right_drag_pan_without_emitting_edit_or_seek() {
        for button in [egui::PointerButton::Middle, egui::PointerButton::Secondary] {
            let context = egui::Context::default();
            let mut cache = WaveformCache {
                view: Some(200..600),
                ..Default::default()
            };
            let clip = AudioClip {
                channels: 1,
                sample_rate: 100,
                duration_seconds: 10.0,
                samples: vec![0.0; 1000].into(),
            };
            for (x, pressed) in [
                (400.0, None),
                (400.0, Some(true)),
                (500.0, None),
                (500.0, Some(false)),
            ] {
                let mut events = vec![egui::Event::PointerMoved(egui::pos2(x, 180.0))];
                if let Some(pressed) = pressed {
                    events.push(egui::Event::PointerButton {
                        pos: egui::pos2(x, 180.0),
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                let _ = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800., 600.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            assert!(
                                cache
                                    .show(ui, &clip, 1.0, 0.0, Some(&(250..350)), 120.0, 1, false)
                                    .is_none()
                            );
                        });
                    },
                );
            }
            let view = cache.view.unwrap();
            assert!(view.start < 200);
            assert_eq!(view.len(), 400);
        }
    }

    #[test]
    fn zoom_preserves_anchor_and_clamps_to_clip() {
        assert_eq!(zoom_range(&(200..600), 1000, 0.5, 0.25), 250..450);
        assert_eq!(zoom_range(&(0..100), 1000, 2.0, 1.0), 0..200);
        assert_eq!(zoom_range(&(900..1000), 1000, 100.0, 0.0), 0..1000);
        assert_eq!(zoom_range(&(200..201), 1000, 0.5, 0.5).len(), 1);
    }

    #[test]
    fn shift_drag_emits_selection_and_plain_click_emits_seek() {
        let context = egui::Context::default();
        let mut cache = WaveformCache {
            view: Some(200..800),
            ..Default::default()
        };
        let clip = AudioClip {
            channels: 1,
            sample_rate: 100,
            duration_seconds: 10.0,
            samples: vec![0.0; 1000].into(),
        };
        let mut render = |events: Vec<egui::Event>, shift: bool| {
            let mut action = None;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                modifiers: egui::Modifiers {
                    shift,
                    ..Default::default()
                },
                events,
                ..Default::default()
            };
            let _ = context.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = cache.show(ui, &clip, 1.0, 0.0, None, 120.0, 1, false);
                });
            });
            action
        };
        render(vec![], false);
        let press = |x, pressed, shift| egui::Event::PointerButton {
            pos: egui::pos2(x, 140.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers {
                shift,
                ..Default::default()
            },
        };
        render(
            vec![
                egui::Event::PointerMoved(egui::pos2(100.0, 140.0)),
                press(100.0, true, true),
            ],
            true,
        );
        let action = render(
            vec![egui::Event::PointerMoved(egui::pos2(400.0, 140.0))],
            true,
        );
        match action {
            Some(WaveformAction::Select(range)) => {
                assert!(range.start > 200 && range.start < 350);
                assert!(range.end > 450 && range.end < 550);
            }
            _ => panic!("shift-drag must select rather than seek"),
        }
        render(vec![press(400.0, false, true)], true);
        render(
            vec![
                egui::Event::PointerMoved(egui::pos2(300.0, 140.0)),
                press(300.0, true, false),
            ],
            false,
        );
        assert!(matches!(
            render(vec![press(300.0, false, false)], false),
            Some(WaveformAction::Seek(fraction)) if fraction > 0.4 && fraction < 0.45
        ));
    }

    #[test]
    fn selection_handles_reverse_drag_edges_and_empty_ranges() {
        assert_eq!(selection_range(8, 2, 10), Some(2..8));
        assert_eq!(selection_range(0, 20, 10), Some(0..10));
        assert_eq!(selection_range(4, 4, 10), None);
        assert_eq!(selection_range(0, 1, 0), None);
    }

    #[test]
    fn peaks_preserve_stereo_transients_and_rebuild_after_replacement() {
        let mut clip = AudioClip {
            channels: 2,
            sample_rate: 2,
            duration_seconds: 2.0,
            samples: vec![0.0, 0.0, -0.9, 0.8, 0.2, -0.1, 0.0, 1.0].into(),
        };
        let mut cache = WaveformCache::default();
        cache.prepare(&clip, 2);
        assert_eq!(cache.peaks, vec![(-0.9, 0.8), (-0.1, 1.0)]);
        cache.prepare(&clip, 1);
        assert_eq!(cache.peaks, vec![(-0.9, 1.0)]);
        cache.view = Some(2..4);
        cache.prepare(&clip, 1);
        assert_eq!(cache.peaks, vec![(-0.1, 1.0)]);
        clip.samples = vec![0.0; 8].into();
        cache.clear();
        cache.prepare(&clip, 1);
        assert_eq!(cache.peaks, vec![(0.0, 0.0)]);
    }
}
