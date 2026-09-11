use crate::audio::AudioClip;
use eframe::egui;

pub(crate) fn draw_waveform(ui: &mut egui::Ui, clip: &AudioClip, gain: f32, progress: f32) {
    let size = egui::vec2(ui.available_width(), 180.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, egui::Color32::from_gray(20));

    let center_y = rect.center().y;

    painter.line_segment(
        [
            egui::pos2(rect.left(), center_y),
            egui::pos2(rect.right(), center_y),
        ],
        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(65)),
    );

    let channels = usize::from(clip.channels);
    let frame_count = clip.samples.len() / channels;

    if frame_count == 0 {
        return;
    }

    let columns = (rect.width().max(1.0) as usize).min(frame_count);
    let amplitude_scale = rect.height() * 0.45 * gain;

    for column in 0..columns {
        let start_frame = column * frame_count / columns;
        let end_frame = (column + 1) * frame_count / columns;

        let start = start_frame * channels;
        let end = end_frame * channels;
        let samples = &clip.samples[start..end];

        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;

        for &sample in samples {
            low = low.min(sample);
            high = high.max(sample);
        }

        let x = rect.left() + (column as f32 + 0.5) / columns as f32 * rect.width();

        let top = center_y - high.clamp(-1.0, 1.0) * amplitude_scale;
        let bottom = center_y - low.clamp(-1.0, 1.0) * amplitude_scale;

        painter.line_segment(
            [egui::pos2(x, top), egui::pos2(x, bottom)],
            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(100, 210, 180)),
        );
    }
    let playhead_x = rect.left() + progress * rect.width();

    painter.line_segment(
        [
            egui::pos2(playhead_x, rect.top()),
            egui::pos2(playhead_x, rect.bottom()),
        ],
        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
    );
}
