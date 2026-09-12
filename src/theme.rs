use eframe::egui::{self, Color32, FontId, RichText, Stroke};

pub(crate) const BG: Color32 = Color32::from_rgb(16, 17, 19);
pub(crate) const PANEL: Color32 = Color32::from_rgb(26, 28, 32);
pub(crate) const SURFACE: Color32 = Color32::from_rgb(37, 40, 46);
pub(crate) const LINE: Color32 = Color32::from_rgb(56, 61, 69);
pub(crate) const TEXT: Color32 = Color32::from_rgb(242, 242, 239);
pub(crate) const MUTED: Color32 = Color32::from_rgb(165, 171, 179);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(255, 97, 58);
pub(crate) const AMBER: Color32 = Color32::from_rgb(239, 191, 111);

pub(crate) fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.override_text_color = Some(TEXT);

    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = BG;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(49, 52, 58);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(49, 52, 58);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.active.bg_fill = ACCENT;
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.slider_trailing_fill = true;
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.slider_width = 145.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(24.0));
    for widgets in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.open,
    ] {
        widgets.corner_radius = egui::CornerRadius::same(3);
    }
    ctx.set_style(style);
}

pub(crate) fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .monospace()
            .size(11.0)
            .strong()
            .color(MUTED),
    );
}

pub(crate) fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong().color(BG))
        .fill(ACCENT)
        .corner_radius(3)
}

pub(crate) fn time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}.{:03}",
        millis / 60_000,
        millis / 1000 % 60,
        millis % 1000
    )
}

pub(crate) fn mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, SURFACE);
    for (i, height) in [0.25, 0.55, 0.8, 0.4, 0.65].into_iter().enumerate() {
        let x = rect.left() + size * (0.22 + i as f32 * 0.14);
        painter.line_segment(
            [
                egui::pos2(x, rect.center().y - size * height * 0.35),
                egui::pos2(x, rect.center().y + size * height * 0.35),
            ],
            Stroke::new((size * 0.05).max(2.0), ACCENT),
        );
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Icon {
    Silence,
    Delete,
    Undo,
    Redo,
    Add,
    Open,
    Save,
    Previous,
    Next,
    Marker,
    Export,
    Play,
    Pause,
    Stop,
    Select,
    Clear,
}

pub(crate) fn icon_button(label: &str, icon: Icon) -> impl egui::Widget + '_ {
    move |ui: &mut egui::Ui| {
        let primary = matches!(icon, Icon::Export | Icon::Play | Icon::Pause);
        let foreground = if primary {
            BG
        } else {
            ui.visuals().text_color()
        };
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            foreground,
        );
        let response = ui.add_sized(
            egui::vec2(36.0 + galley.size().x + 12.0, galley.size().y.max(30.0)),
            if primary {
                egui::Button::new("").fill(ACCENT).corner_radius(3)
            } else {
                egui::Button::new("")
            },
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
        });
        ui.painter().galley(
            egui::pos2(
                response.rect.left() + 36.0,
                response.rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            foreground,
        );
        let rect = response.rect;
        let origin = egui::pos2(rect.left() + 12.0, rect.center().y - 7.0);
        ui.painter().rect_filled(
            egui::Rect::from_center_size(origin + egui::vec2(7.0, 7.0), egui::vec2(24.0, 24.0)),
            4.0,
            if primary { BG } else { ACCENT },
        );
        let stroke = Stroke::new(1.5_f32, if primary { ACCENT } else { BG });
        let p = |x, y| origin + egui::vec2(x, y);
        let line = |a: (f32, f32), b: (f32, f32)| {
            ui.painter()
                .line_segment([p(a.0, a.1), p(b.0, b.1)], stroke);
        };
        match icon {
            Icon::Select => {
                line((1., 4.), (1., 1.));
                line((1., 1.), (4., 1.));
                line((10., 1.), (13., 1.));
                line((13., 1.), (13., 4.));
                line((1., 10.), (1., 13.));
                line((1., 13.), (4., 13.));
                line((10., 13.), (13., 13.));
                line((13., 13.), (13., 10.));
            }
            Icon::Clear => {
                line((2., 2.), (12., 12.));
                line((12., 2.), (2., 12.));
            }
            Icon::Pause => {
                line((3., 1.), (3., 13.));
                line((11., 1.), (11., 13.));
            }
            Icon::Play => {
                line((3., 1.), (12., 7.));
                line((12., 7.), (3., 13.));
                line((3., 13.), (3., 1.));
            }
            Icon::Stop => {
                for (a, b) in [
                    ((2., 2.), (12., 2.)),
                    ((12., 2.), (12., 12.)),
                    ((12., 12.), (2., 12.)),
                    ((2., 12.), (2., 2.)),
                ] {
                    line(a, b);
                }
            }
            Icon::Previous | Icon::Next => {
                let x = |v| {
                    if matches!(icon, Icon::Next) {
                        14.0 - v
                    } else {
                        v
                    }
                };
                for (a, b) in [
                    ((2., 1.), (2., 13.)),
                    ((11., 2.), (5., 7.)),
                    ((5., 7.), (11., 12.)),
                ] {
                    line((x(a.0), a.1), (x(b.0), b.1));
                }
            }
            Icon::Marker => {
                for (a, b) in [
                    ((2., 0.), (2., 14.)),
                    ((2., 0.), (9., 0.)),
                    ((9., 0.), (9., 6.)),
                    ((9., 6.), (2., 6.)),
                    ((8., 11.), (14., 11.)),
                    ((11., 8.), (11., 14.)),
                ] {
                    line(a, b);
                }
            }
            Icon::Export => {
                for (a, b) in [
                    ((7., 0.), (7., 9.)),
                    ((3., 5.), (7., 9.)),
                    ((7., 9.), (11., 5.)),
                    ((1., 10.), (1., 14.)),
                    ((1., 14.), (13., 14.)),
                    ((13., 14.), (13., 10.)),
                ] {
                    line(a, b);
                }
            }
            Icon::Add => {
                line((0., 7.), (14., 7.));
                line((7., 0.), (7., 14.));
            }
            Icon::Delete => {
                line((2., 3.), (12., 3.));
                line((5., 0.), (9., 0.));
                line((3., 5.), (4., 14.));
                line((4., 14.), (10., 14.));
                line((10., 14.), (11., 5.));
            }
            Icon::Silence => {
                line((0., 7.), (14., 7.));
                line((1., 2.), (1., 12.));
                line((13., 2.), (13., 12.));
            }
            Icon::Undo | Icon::Redo => {
                let flip = |x| {
                    if matches!(icon, Icon::Redo) {
                        14. - x
                    } else {
                        x
                    }
                };
                for (a, b) in [
                    ((5., 1.), (1., 5.)),
                    ((1., 5.), (5., 9.)),
                    ((1., 5.), (10., 5.)),
                    ((10., 5.), (13., 8.)),
                    ((13., 8.), (13., 13.)),
                ] {
                    line((flip(a.0), a.1), (flip(b.0), b.1));
                }
            }
            Icon::Open => {
                for (a, b) in [
                    ((0., 3.), (5., 3.)),
                    ((5., 3.), (7., 5.)),
                    ((7., 5.), (14., 5.)),
                    ((14., 5.), (12., 13.)),
                    ((12., 13.), (0., 13.)),
                    ((0., 13.), (0., 3.)),
                ] {
                    line(a, b);
                }
            }
            Icon::Save => {
                for (a, b) in [
                    ((1., 1.), (11., 1.)),
                    ((11., 1.), (14., 4.)),
                    ((14., 4.), (14., 14.)),
                    ((14., 14.), (1., 14.)),
                    ((1., 14.), (1., 1.)),
                    ((4., 1.), (4., 6.)),
                    ((4., 6.), (10., 6.)),
                    ((10., 6.), (10., 1.)),
                    ((4., 14.), (4., 10.)),
                    ((4., 10.), (11., 10.)),
                    ((11., 10.), (11., 14.)),
                ] {
                    line(a, b);
                }
            }
        }
        response
    }
}

/// A button-backed switch retains keyboard activation and focus behavior.
pub(crate) fn toggle<'a>(value: &'a mut bool, label: &'a str) -> impl egui::Widget + 'a {
    move |ui: &mut egui::Ui| {
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        // Reserve actual geometry: inset + track + gap + measured label + inset.
        let size = egui::vec2(
            9.0 + 34.0 + 10.0 + galley.size().x + 9.0,
            galley.size().y.max(30.0),
        );
        let mut response = ui.add_sized(size, egui::Button::new("").frame(false));
        ui.painter().galley(
            egui::pos2(
                response.rect.left() + 53.0,
                response.rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            ui.visuals().text_color(),
        );
        if response.clicked() {
            *value = !*value;
            response.mark_changed();
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
        });
        let t = ui
            .ctx()
            .animate_bool_with_time(response.id.with("switch"), *value, 0.14);
        let hover =
            ui.ctx()
                .animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.10);
        if response.has_focus() {
            ui.painter().rect_stroke(
                response.rect,
                3.0,
                Stroke::new(1.0_f32, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let track = egui::Rect::from_center_size(
            egui::pos2(response.rect.left() + 26.0, response.rect.center().y),
            egui::vec2(34.0, 18.0),
        );
        let color = egui::Color32::from_rgb(
            (LINE.r() as f32 + (ACCENT.r() as f32 - LINE.r() as f32) * t) as u8,
            (LINE.g() as f32 + (ACCENT.g() as f32 - LINE.g() as f32) * t) as u8,
            (LINE.b() as f32 + (ACCENT.b() as f32 - LINE.b() as f32) * t) as u8,
        );
        ui.painter().rect_filled(track, 9.0, color);
        let center = egui::pos2(track.left() + 9.0 + t * 16.0, track.center().y);
        ui.painter().circle_filled(center, 6.0 + hover * 0.5, TEXT);
        response
    }
}

pub(crate) fn segments(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(BG)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(5)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                contents(ui);
            });
        });
}

pub(crate) fn segment(label: &str, active: bool) -> impl egui::Widget + '_ {
    move |ui: &mut egui::Ui| {
        let group = ui.id();
        let background = ui.painter().add(egui::Shape::Noop);
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            TEXT,
        );
        let response = ui.add_sized(
            egui::vec2(galley.size().x + 42.0, 30.0),
            egui::Button::new("").frame(false),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                ui.is_enabled(),
                active,
                label,
            )
        });
        if active {
            let rect = response.rect;
            let x = ui
                .ctx()
                .animate_value_with_time(group.with("selection_x"), rect.left(), 0.18);
            let width =
                ui.ctx()
                    .animate_value_with_time(group.with("selection_w"), rect.width(), 0.18);
            ui.painter().set(
                background,
                egui::Shape::rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, rect.top()),
                        egui::vec2(width, rect.height()),
                    ),
                    3.0,
                    ACCENT,
                ),
            );
        }
        let color = if active { BG } else { TEXT };
        ui.painter().galley_with_override_text_color(
            egui::pos2(
                response.rect.left() + 30.0,
                response.rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
        let origin = egui::pos2(response.rect.left() + 9.0, response.rect.center().y);
        let stroke = Stroke::new(1.5_f32, color);
        for i in 0..5 {
            let height = match label {
                "Envelope" => [2., 5., 7., 4., 1.][i],
                "Waveform" => [3., 6., 2., 7., 4.][i],
                "Both" | "Overlay" => [6., 3., 6., 3., 6.][i],
                _ => [2., 4., 7., 5., 3.][i],
            };
            let x = origin.x + i as f32 * 3.0;
            ui.painter().line_segment(
                [
                    egui::pos2(x, origin.y - height),
                    egui::pos2(x, origin.y + height),
                ],
                stroke,
            );
        }
        response
    }
}
