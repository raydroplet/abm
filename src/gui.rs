// gui.rs

use crate::engine::{DebugInfo, Engine, FrameData, ViewBuffer};
use crate::wave::Signal;

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
    view_texture: Option<egui::TextureHandle>,
    //
    latest_debug_info: DebugInfo,
    latest_signals: Vec<Signal>,
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
            view_texture: None,
            //
            latest_debug_info: DebugInfo::default(),
            latest_signals: Vec::default(),
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
                view_buffer: ViewBuffer {
                    width: width,
                    height: height,
                    pixels: vec![0; buffer_size],
                },
                signals: Vec::new(),
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
        // ---------------------------------------------------------------------
        // 1. DATA SYNC
        // ---------------------------------------------------------------------
        let mut latest = None;

        // Drain channel to get the absolutely newest frame
        while let Ok(frame) = self.receiver.try_recv() {
            if let Some(old) = latest {
                let _ = self.returner.send(old); // Recycle skipped frames
            }
            latest = Some(frame);
        }

        // If we got new data, update our local state
        if let Some(frame) = latest {
            let view = &frame.view_buffer;

            // A. Update Agents Texture (Pixel Buffer)
            let view_image =
                egui::ColorImage::from_rgba_unmultiplied([view.width, view.height], &view.pixels);

            match &mut self.view_texture {
                Some(t) => t.set(view_image, egui::TextureOptions::NEAREST),
                None => {
                    self.view_texture =
                        Some(ctx.load_texture("agents", view_image, egui::TextureOptions::NEAREST));
                }
            };

            // B. Store Wave Data (Vector Data)
            self.latest_signals = frame.signals.clone();

            // C. Store Stats
            self.latest_debug_info = frame.debug_info;

            // D. Recycle the frame buffer back to Engine
            let _ = self.returner.send(frame);
        }

        // ---------------------------------------------------------------------
        // 2. PHYSICS RATE CALCULATION (UPS)
        // ---------------------------------------------------------------------
        let time = ctx.input(|i| i.time);
        if time - self.last_measure_time >= 0.5 {
            let current_total = self.latest_debug_info.tick_counter;
            let ticks_passed = current_total.wrapping_sub(self.last_tick_count);
            let time_passed = (time - self.last_measure_time) as f64;

            self.display_ups = (ticks_passed as f64 / time_passed) as u64;
            self.last_tick_count = current_total;
            self.last_measure_time = time;
        }

        // ---------------------------------------------------------------------
        // 3. RENDER LOOP
        // ---------------------------------------------------------------------
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                // --- LAYER 1: WAVES (Vector Graphics) ---
                // We draw this first so it appears "behind" the agents.
                // Inside egui::CentralPanel...
                let painter = ui.painter();

                for sig in &self.latest_signals {
                    // Renamed from circles

                    // 1. Calculate Opacity based on Strength
                    // We clamp it so it doesn't vanish completely or become solid rock.
                    let alpha = (sig.intensity * 255.0).clamp(0.0, 255.0) as u8;

                    // 2. Resolve Color
                    let base_color = match sig.mask.trailing_zeros() {
                        0 => egui::Color32::from_rgba_unmultiplied(255, 50, 50, alpha), // Bit 0: Red (Sound)
                        1 => egui::Color32::from_rgba_unmultiplied(50, 255, 50, alpha), // Bit 1: Green (Smell)
                        2 => egui::Color32::from_rgba_unmultiplied(50, 100, 255, alpha), // Bit 2: Blue (Radio)
                        3 => egui::Color32::from_rgba_unmultiplied(255, 255, 0, alpha), // Bit 3: Yellow (Light)
                        4 => egui::Color32::from_rgba_unmultiplied(255, 0, 255, alpha), // Bit 4: Magenta (Magic)
                        5 => egui::Color32::from_rgba_unmultiplied(0, 255, 255, alpha), // Bit 5: Cyan (Electric)
                        6 => egui::Color32::from_rgba_unmultiplied(255, 140, 0, alpha), // Bit 6: Orange (Heat)
                        7 => egui::Color32::from_rgba_unmultiplied(255, 255, 255, alpha), // Bit 7: White (Debug)
                        _ => egui::Color32::from_gray(alpha), // Should effectively never happen
                    };

                    // 3. Render Logic: RING vs CIRCLE
                    if sig.inner_radius > 0.5 {
                        // --- IT IS A WAVE (RING) ---
                        // We use a thick stroke to simulate the "body" of the wave.

                        let thickness = sig.outer_radius - sig.inner_radius;
                        let center_radius = sig.inner_radius + (thickness / 2.0);

                        painter.circle_stroke(
                            egui::pos2(sig.origin[0], sig.origin[1]),
                            center_radius,
                            egui::Stroke::new(thickness, base_color),
                        );
                    } else {
                        // --- IT IS A SOURCE (SOLID CIRCLE) ---
                        painter.circle_filled(
                            egui::pos2(sig.origin[0], sig.origin[1]),
                            sig.outer_radius,
                            base_color,
                        );
                    }
                }

                // --- LAYER 2: AGENTS (Texture Overlay) ---
                if let Some(agent_tex) = &self.view_texture {
                    // Draw the image filling the entire window
                    ui.image(agent_tex);
                }

                // --- LAYER 3: DEBUG UI ---
                egui::Window::new("Debug Info")
                    .resizable(false)
                    .collapsible(false)
                    .default_pos([10.0, 10.0])
                    .show(ctx, |ui| {
                        let fps = 1.0 / ctx.input(|i| i.stable_dt);
                        ui.label(format!("Render FPS: {:.0}", fps));

                        let engine_fps = 1000.0 / self.latest_debug_info.render_time_ms;
                        ui.label(format!("Potential FPS: {:.0}", engine_fps));
                        ui.label(format!("UPS: {:.0}", self.display_ups));

                        ui.separator();
                        ui.label(format!("Ticks: {}", self.latest_debug_info.tick_counter));
                        ui.label(format!("Agents: {}", self.latest_debug_info.agent_count));
                        ui.label(format!("Waves: {}", self.latest_signals.len()));
                    });

                // Force constant repaint to hit 60 FPS
                ctx.request_repaint();
            });
    }
}
