// gui.rs

use crate::engine::{Engine, FrameData};
pub use crossbeam_channel as crossbeam;
pub use eframe::egui;
use std::thread;

pub struct Producer {
    returner: crossbeam::Sender<FrameData>,
    receiver: crossbeam::Receiver<FrameData>,
}

impl Producer {
    pub fn new(
        engine_receiver: crossbeam::Receiver<FrameData>,
        engine_returner: crossbeam::Sender<FrameData>,
    ) -> Self {
        Self {
            returner: engine_returner,
            receiver: engine_receiver,
        }
    }

    // Takes ownership of Engine and runs it in a background thread
    pub fn run_thread(self, mut engine: Engine) {
        thread::spawn(move || {
            loop {
                engine.tick();

                match self.receiver.try_recv() {
                    Ok(mut frame) => {
                        println!("Engine: Rendering frame..."); // <--- CHECK THIS
                        // A buffer is available! We can render.
                        engine.render(&mut frame);

                        // Send it to the UI
                        if self.returner.send(frame).is_err() {
                            break; // UI closed
                        }
                    }
                    Err(crossbeam::TryRecvError::Empty) => {
                        // No buffer available yet.
                        // The UI is still drawing the previous frame.
                        // Just loop back and tick again!
                        continue;
                    }
                    Err(crossbeam::TryRecvError::Disconnected) => {
                        break; // UI closed
                    }
                }
            }
        });
    }
}

// Small wrapper for the eframe::App trait
//
// Implements the update() method that queries a frame
// and sends it to eframe to be presented on the window
//
pub struct Presenter {
    receiver: crossbeam::Receiver<FrameData>,
    returner: crossbeam::Sender<FrameData>,
    texture: Option<egui::TextureHandle>,
}

impl Presenter {
    pub fn new(
        frame_receiver: crossbeam::Receiver<FrameData>,
        frame_returner: crossbeam::Sender<FrameData>,
    ) -> Self {
        Self {
            receiver: frame_receiver,
            returner: frame_returner,
            texture: None,
        }
    }

    pub fn run(self) -> eframe::Result<()> {
        // frames
        let width: usize = 1024;
        let height: usize = 768;
        let buffer_size = width * height * 4; // fine for now

        for _ in 0..2 {
            let _ = self.returner.send(FrameData {
                width: width,
                height: height,
                pixels: vec![0; buffer_size], 
            });
        }

        // Options
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([width as f32, height as f32])
                .with_resizable(false),
            run_and_return: false,
            ..Default::default()
        };

        // Launches the app
        eframe::run_native(
            "Phase 1: Prototype",
            options,
            Box::new(move |_context| {
                // context could be used here (in this lambda) if needed
                Ok(Box::new(self))
            }),
        )
    }
}

impl eframe::App for Presenter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                // 1. Drain Channel (Get latest, recycle rest)
                let mut latest = None;
                while let Ok(frame) = self.receiver.try_recv() {
                    if let Some(old) = latest {
                        let _ = self.returner.send(old);
                    }
                    latest = Some(frame);
                }

                if let Some(frame) = latest {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width, frame.height],
                        &frame.pixels,
                    );

                    match &mut self.texture {
                        Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
                        None => {
                            self.texture = Some(ctx.load_texture(
                                "display",
                                image,
                                egui::TextureOptions::NEAREST,
                            ));
                        }
                    };

                    // Recycle...
                    let _ = self.returner.send(frame);
                }

                // 3. Draw
                if let Some(texture) = &self.texture {
                    ui.image(texture);
                }

                ctx.request_repaint();
            });
    }
}
