// gui.rs

use crate::engine::{DebugInfo, Engine, FrameData};
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
    //
    latest_debug_info: DebugInfo,
    //
    // UPS (Physics) Smoothing
    last_tick_count: u64,   // Snapshot of total ticks 0.5s ago
    last_measure_time: f64, // Timestamp of the last check
    display_ups: u64,
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
            //
            latest_debug_info: DebugInfo::default(),
            //
            last_tick_count: 0,
            last_measure_time: 0.0,
            display_ups: 0,
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
                debug_info: DebugInfo::default(),
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
        // 1. Drain Channel (Get latest, recycle rest)
        let mut latest = None;
        while let Ok(frame) = self.receiver.try_recv() {
            if let Some(old) = latest {
                let _ = self.returner.send(old);
            }
            latest = Some(frame);
        }

        // extracts FrameData. if we fail we still render the last known data
        if let Some(frame) = latest {
            // queries the frame data into an egui image to be render latter
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width, frame.height],
                &frame.pixels,
            );

            match &mut self.texture {
                Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
                None => {
                    self.texture =
                        Some(ctx.load_texture("display", image, egui::TextureOptions::NEAREST));
                }
            };

            // debug info
            self.latest_debug_info = frame.debug_info;

            // Recycle...
            let _ = self.returner.send(frame);
        }

        // Calculate Physics UPS (Snapshot Delta)
        let time = ctx.input(|i| i.time);
        if time - self.last_measure_time >= 0.5 {
            let current_total = self.latest_debug_info.tick_counter; // This is a u64 Counter

            // Calculate Difference
            let ticks_passed = current_total.wrapping_sub(self.last_tick_count);
            let time_passed = (time - self.last_measure_time) as f64;

            // Calculate Rate
            self.display_ups = (ticks_passed as f64 / time_passed) as u64;

            // Save Snapshot
            self.last_tick_count = current_total;
            self.last_measure_time = time;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                // 3. Draw
                if let Some(texture) = &self.texture {
                    ui.image(texture);
                }

                // DEBUG OVERLAY
                // We create a floating window that cannot be collapsed or resized
                egui::Window::new("Debug Info")
                    .resizable(false)
                    .collapsible(false)
                    // .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0]) // Top-Left corner
                    .default_pos([10.0, 10.0]) // Top-Left corner
                    .show(ctx, |ui| {
                        // 1. Render FPS (Calculated by Egui)
                        // stable_dt is the smoothed time between frames
                        let fps = 1.0 / ctx.input(|i| i.stable_dt);
                        ui.label(format!("Render FPS: {:.0}", fps));

                        // 2. Engine Stats (From the latest frame we received)
                        // We access the texture data indirectly or just use the last frame received
                        if let Some(texture) = &self.texture {
                            // Note: You might need to store the 'latest_stats' in Presenter struct
                            // if you want to access them here, or just extract them when you recv().
                            // For now, let's assume you store 'current_frame_data' in Presenter.

                            // Calculate Engine FPS from the render time
                            let engine_fps = 1000.0 / self.latest_debug_info.render_time_ms;
                            ui.label(format!("Potential FPS: {:.0}", engine_fps));
                            ui.label(format!("UPS: {:.0}", self.display_ups));
                            ui.label(format!("tick count: {:.0}", self.latest_debug_info.tick_counter));
                            ui.label(format!("agent count: {}", self.latest_debug_info.agent_count));

                            // Simple placeholder if you haven't stored the struct yet:
                            ui.label(format!(
                                "Texture Size: {}x{}",
                                texture.size()[0],
                                texture.size()[1]
                            ));
                        }

                        ui.separator();
                        ui.label("Phase 1 (Prototype)");
                    });

                ctx.request_repaint();
            });
    }
}
