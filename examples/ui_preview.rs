// Developer-only native rendering harness: cargo run --example ui_preview -- <wav|empty> <output.bmp> [width height]
#[path = "../src/app.rs"]
mod app;
#[path = "../src/audio.rs"]
mod audio;
#[path = "../src/edits.rs"]
mod edits;
#[path = "../src/export.rs"]
mod export;
#[path = "../src/playback.rs"]
mod playback;
#[path = "../src/project.rs"]
mod project;
#[path = "../src/spectrum.rs"]
mod spectrum;
#[path = "../src/spectrum_view.rs"]
mod spectrum_view;
#[path = "../src/theme.rs"]
mod theme;
#[path = "../src/transients.rs"]
mod transients;
#[path = "../src/waveform.rs"]
mod waveform;
use eframe::egui;
use std::io::Write;
struct Preview {
    app: app::SampleWorkbench,
    frame: usize,
    output: String,
    select_region: bool,
    apply_silence: bool,
    show_attacks: bool,
    show_spectrum: bool,
    show_overlay: bool,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if self.show_spectrum {
            input.focused = true;
            let pressed = match self.frame {
                10 => true,
                11 => false,
                _ => return,
            };
            let pos = egui::pos2(if self.show_overlay { 525.0 } else { 385.0 }, 291.0);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            return;
        }
        if self.show_attacks {
            input.focused = true;
            let (pressed, x, y) = match self.frame {
                10 => (true, 100.0, 135.0),
                11 => (false, 100.0, 135.0),
                20 => (true, 100.0, 320.0),
                21 => (false, 100.0, 320.0),
                30 => (true, 385.0, 291.0),
                31 => (false, 385.0, 291.0),
                _ => return,
            };
            input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(x, y)));
            input.events.push(egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            return;
        }
        if !self.select_region {
            return;
        }
        input.focused = true;
        input.modifiers.shift = self.frame <= 12;
        let (x, y, pressed) = match self.frame {
            10 => (500.0, 400.0, Some(true)),
            11 => (650.0, 400.0, None),
            12 => (650.0, 400.0, Some(false)),
            13 if self.apply_silence => (330.0, 195.0, Some(true)),
            14 if self.apply_silence => (330.0, 195.0, Some(false)),
            _ => return,
        };
        input
            .events
            .push(egui::Event::PointerMoved(egui::pos2(x, y)));
        if let Some(pressed) = pressed {
            input.events.push(egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: input.modifiers,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let width = image.size[0] as u32;
                let height = image.size[1] as i32;
                let mut header = vec![0u8; 54];
                header[0..2].copy_from_slice(b"BM");
                header[2..6].copy_from_slice(&(54 + width * height as u32 * 4).to_le_bytes());
                header[10..14].copy_from_slice(&54u32.to_le_bytes());
                header[14..18].copy_from_slice(&40u32.to_le_bytes());
                header[18..22].copy_from_slice(&width.to_le_bytes());
                header[22..26].copy_from_slice(&(-height).to_le_bytes());
                header[26..28].copy_from_slice(&1u16.to_le_bytes());
                header[28..30].copy_from_slice(&32u16.to_le_bytes());
                let mut file =
                    std::io::BufWriter::new(std::fs::File::create(&self.output).unwrap());
                file.write_all(&header).unwrap();
                for pixel in &image.pixels {
                    let [r, g, b, _] = pixel.to_array();
                    file.write_all(&[b, g, r, 255]).unwrap();
                }
                file.flush().unwrap();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        self.frame += 1;
        if self.frame
            == if self.show_spectrum || self.show_attacks {
                100
            } else {
                15
            }
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let output = args.get(2).expect("output.bmp argument required").clone();
    let width = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1180.0);
    let height = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(780.0);
    eframe::run_native(
        "UI preview",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([width, height]),
            ..Default::default()
        },
        Box::new(move |cc| {
            let app = if args.get(1).is_some_and(|s| s == "empty") {
                theme::apply(&cc.egui_ctx);
                app::SampleWorkbench::default()
            } else {
                app::SampleWorkbench::new(&cc.egui_ctx)
            };
            Ok(Box::new(Preview {
                app,
                frame: 0,
                output,
                select_region: args
                    .get(5)
                    .is_some_and(|s| s == "selected" || s == "edited"),
                apply_silence: args.get(5).is_some_and(|s| s == "edited"),
                show_attacks: args.get(5).is_some_and(|s| s == "attacks"),
                show_spectrum: args
                    .get(5)
                    .is_some_and(|s| s == "spectrum" || s == "overlay"),
                show_overlay: args.get(5).is_some_and(|s| s == "overlay"),
            }))
        }),
    )
}
