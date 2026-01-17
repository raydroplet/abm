// gui.rs

use crate::components::{SignalEmitter, Transform};
use crate::engine::{AgentPoint, DebugInfo, Engine, FrameData};
use crate::wave::{LevelMask, Signal};

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
    //
    latest_debug_info: DebugInfo,
    latest_agents: Vec<AgentPoint>,
    //
    // UPS (Physics) Smoothing
    last_tick_count: u64,   // Snapshot of total ticks 0.5s ago
    last_measure_time: f64, // Timestamp of the last check
    display_ups: u64,
    //
    // local "Double Buffer"
    // It stays here so we can draw it even if the engine is busy ticking.
    current_frame: Option<FrameData>,
}

impl Presenter {
    pub fn new(
        frame_receiver: crossbeam::Receiver<FrameData>,
        frame_returner: crossbeam::Sender<FrameData>,
    ) -> Self {
        Self {
            receiver: frame_receiver,
            returner: frame_returner,
            //
            latest_debug_info: DebugInfo::default(),
            latest_agents: Vec::new(),
            //
            last_tick_count: 0,
            last_measure_time: 0.0,
            display_ups: 0,
            //
            current_frame: Option::default(),
        }
    }

    pub fn run(self) -> eframe::Result<()> {
        // frames
        let width: usize = 1024;
        let height: usize = 768;

        for _ in 0..2 {
            let _ = self.returner.send(FrameData {
                agents: Vec::new(),
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

    fn resolve_signal_color(sig: &Signal, alpha: u8) -> egui::Color32 {
        match sig.mask.trailing_ones() {
            0 => egui::Color32::from_rgba_unmultiplied(255, 50, 50, alpha), // Bit 0: Red (Sound)
            1 => egui::Color32::from_rgba_unmultiplied(50, 255, 50, alpha), // Bit 1: Green (Smell)
            2 => egui::Color32::from_rgba_unmultiplied(50, 100, 255, alpha), // Bit 2: Blue (Radio)
            3 => egui::Color32::from_rgba_unmultiplied(255, 255, 0, alpha), // Bit 3: Yellow (Light)
            4 => egui::Color32::from_rgba_unmultiplied(255, 0, 255, alpha), // Bit 4: Magenta (Magic)
            5 => egui::Color32::from_rgba_unmultiplied(0, 255, 255, alpha), // Bit 5: Cyan (Electric)
            6 => egui::Color32::from_rgba_unmultiplied(255, 140, 0, alpha), // Bit 6: Orange (Heat)
            7 => egui::Color32::from_rgba_unmultiplied(255, 255, 255, alpha), // Bit 7: White (Debug)
            _ => panic!(),
        }
    }

    fn render_debug_window(&mut self, ctx: &egui::Context) {
        let padding = 5.0;
        egui::Window::new("Monitor")
            .resizable(false)
            .collapsible(true)
            .default_pos([padding, padding])
            .default_width(0.0)
            .show(ctx, |ui| {
                // 1. Performance Metrics
                let fps = 1.0 / ctx.input(|i| i.stable_dt);
                ui.horizontal(|ui| {
                    ui.label("GUI FPS:");
                    ui.colored_label(egui::Color32::LIGHT_BLUE, format!("{:.0}", fps));
                });

                ui.horizontal(|ui| {
                    let engine_fps = 1000.0 / self.latest_debug_info.render_time_ms;
                    ui.label("Potential FPS:");
                    ui.colored_label(egui::Color32::LIGHT_BLUE, format!("{:.0}", engine_fps));
                });

                ui.horizontal(|ui| {
                    ui.label("Simulation UPS:");
                    ui.colored_label(egui::Color32::LIGHT_GREEN, format!("{}", self.display_ups));
                });

                // ui.separator();

                let engine_ms = self.latest_debug_info.render_time_ms;
                ui.horizontal(|ui| {
                    ui.label("Engine Render Time:");
                    ui.colored_label(
                        if engine_ms > 5.0 {
                            egui::Color32::KHAKI
                        } else {
                            egui::Color32::WHITE
                        },
                        format!("{:.2}ms", engine_ms),
                    );
                });

                ui.horizontal(|ui| {
                    let phys_ms = self.latest_debug_info.tick_time_ms;
                    ui.label("Physics Time:");
                    ui.colored_label(
                        // If physics takes > 8ms, we are dangerously close to
                        // missing the 10ms deadline (100 ticks/sec)
                        if phys_ms > 8.0 {
                            egui::Color32::RED
                        } else {
                            egui::Color32::LIGHT_GREEN
                        },
                        format!("{:.3}ms", phys_ms),
                    );
                });

                ui.separator();

                let wave_count = self.latest_debug_info.agent_count;

                ui.label(format!(
                    "Total Entities: {}",
                    self.latest_debug_info.agent_count
                ));
                ui.label(format!("Visible Waves:  {}", wave_count));

                ui.label(format!(
                    "Ticks Elapsed:  {}",
                    self.latest_debug_info.tick_counter
                ));
            });
    }

    fn render_inspection_window(&mut self, ctx: &egui::Context) {
        let padding = 5.0;
        egui::Window::new("Inspector")
            .default_width(0.0)
            .pivot(egui::Align2::RIGHT_TOP)
            .default_pos(egui::pos2(
                ctx.viewport_rect().max.x - padding,
                ctx.viewport_rect().min.y + padding,
            ))
            .show(ctx, |ui| {
                let mut transform = Transform::default();
                let mut emitter = SignalEmitter::default();

                Self::render_component_transform(ui, &mut transform);
                Self::render_component_emitter(ui, &mut emitter);
            });
    }

    fn render_component_transform(ui: &mut egui::Ui, transform: &mut Transform) {
        egui::CollapsingHeader::new("Transform")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("transform_grid")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        // Position
                        ui.label("Position");
                        ui.horizontal(|ui| {
                            ui.label("X");
                            ui.add(egui::DragValue::new(&mut transform.position.x).speed(1.0));
                            ui.label("Y");
                            ui.add(egui::DragValue::new(&mut transform.position.y).speed(1.0));
                        });
                        ui.end_row();

                        // Scale
                        ui.label("Scale");
                        ui.horizontal(|ui| {
                            ui.label("X");
                            ui.add(egui::DragValue::new(&mut transform.scale).speed(1.0));
                            ui.label("Y");
                            ui.add(egui::DragValue::new(&mut transform.scale).speed(1.0));
                        });
                        ui.end_row();
                    });
            });
    }

    fn render_component_emitter(ui: &mut egui::Ui, emitter: &mut SignalEmitter) {
        egui::CollapsingHeader::new("Signal")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    egui::Grid::new("emitter_grid")
                        .num_columns(2)
                        .spacing([20.0, 1.0])
                        .show(ui, |ui| {
                            ui.label("Radius");
                            ui.add(
                                egui::Slider::new(&mut emitter.radius, 0.0..=500.0)
                                    .drag_value_speed(0.1)
                                    .step_by(0.1),
                            );
                            ui.end_row();

                            ui.label("Aperture");
                            let mut degrees = emitter.cone_angle.to_degrees();
                            if ui
                                .add(egui::Slider::new(&mut degrees, 0.0..=360.0).suffix("°"))
                                .changed()
                            {
                                emitter.cone_angle = degrees.to_radians();
                            }
                            ui.end_row();

                            ui.label("Rotation");
                            let mut rot_deg = emitter.rotation.to_degrees();
                            if ui
                                .add(
                                    egui::Slider::new(&mut rot_deg, 0.0..=360.0)
                                        .drag_value_speed(1.0)
                                        .suffix("°"),
                                )
                                .changed()
                            {
                                emitter.rotation = rot_deg.to_radians();
                            }
                            ui.end_row();
                        });
                });

                ui.separator();

                let mut mask = LevelMask::<1>::ZERO;
                egui::CollapsingHeader::new("Masks")
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::Grid::new("masks_grid")
                            .num_columns(2)
                            .spacing([20.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Levels"); // Column 1
                                if ui.button("Toggle All").clicked() { /* ... */ }
                                ui.end_row();

                                ui.allocate_space(egui::vec2(0.0, 0.0));
                                Self::bitgrid_widget(ui, &mut mask);
                                ui.end_row();

                                ui.label("Signals"); // Column 1
                                ui.horizontal(|ui| {
                                    if ui.small_button("Toggle All").clicked() { /* ... */ }
                                });
                                ui.end_row();

                                ui.allocate_space(egui::vec2(0.0, 0.0));
                                Self::bitgrid_widget(ui, &mut mask);
                                ui.end_row();
                            });
                    });
            });
    }

    fn bitgrid_widget(ui: &mut egui::Ui, mask: &mut LevelMask) {
        let block_size = 16.0;
        let gap = 2.0;
        let grid_dim = 8;
        // rows logic: removed the overwrite "rows = 8"
        // assuming you want the toggle logic to actually work:
        let expand_id = ui.make_persistent_id("level_matrix_expanded");
        let expanded = ui.data(|d| d.get_temp::<bool>(expand_id).unwrap_or(true)); // Default true for visibility?
        let rows = if expanded { 8 } else { 1 };

        // We use a vertical layout so rows stack downward
        ui.vertical(|ui| {
            // Set spacing once for this scope
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);

            for y in 0..rows {
                ui.horizontal(|ui| {
                    for x in 0..grid_dim {
                        let bit_index = y * grid_dim + x;
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(block_size, block_size),
                            egui::Sense::click(),
                        );

                        if response.clicked() {
                            let current = mask[bit_index];
                            mask.set(bit_index, !current);
                        }

                        // Painting logic...
                        if ui.is_rect_visible(rect) {
                            let is_active = mask[bit_index];
                            let color = if is_active {
                                egui::Color32::LIGHT_BLUE
                            } else {
                                egui::Color32::from_gray(30)
                            };

                            // Use lighter rounding for a cleaner grid look
                            ui.painter().rect_filled(rect, 2.0, color);

                            if response.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(1.0, egui::Color32::WHITE),
                                    egui::StrokeKind::Middle,
                                );
                            }
                        }
                        response.on_hover_text(format!("Level {}", bit_index));
                    }
                });
            }
        });
    }

    fn render_agents(&self, painter: &egui::Painter, frame: &FrameData) {
        // --- LAYER 2: AGENTS ---
        for agent in &frame.agents {
            painter.circle_filled(
                egui::pos2(agent.position.x, agent.position.y),
                agent.radius,
                egui::Color32::from_rgba_unmultiplied(
                    agent.color[0],
                    agent.color[1],
                    agent.color[2],
                    agent.color[3],
                ),
            );
        }
    }

    fn render_waves(&self, painter: &egui::Painter, frame: &FrameData) {
        for sig in &frame.signals {
            let alpha = (sig.intensity * 255.0) as u8;
            let color = Self::resolve_signal_color(sig, alpha);

            if sig.inner_radius > 0.5 {
                let thickness = sig.outer_radius - sig.inner_radius;
                let center_radius = sig.inner_radius + (thickness / 2.0);
                painter.circle_stroke(
                    egui::pos2(sig.origin.x, sig.origin.y),
                    center_radius,
                    egui::Stroke::new(thickness, color),
                );
            } else {
                painter.circle_filled(
                    egui::pos2(sig.origin.x, sig.origin.y),
                    sig.outer_radius,
                    color,
                );
            }
        }
    }
}

impl eframe::App for Presenter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // drain the channel to get the newest frame
        let mut newest_arrived = None;
        while let Ok(frame) = self.receiver.try_recv() {
            // if we already pulled a frame this loop but another is waiting,
            // send the intermediate one back immediately so the engine doesn't starve.
            if let Some(old) = newest_arrived.replace(frame) {
                let _ = self.returner.send(old);
            }
        }

        // if a new frame arrived, swap it into our 'current_frame' storage
        if let Some(new_frame) = newest_arrived {
            // replace() returns the old Some(FrameData). We send it back to the engine.
            if let Some(old_buffer) = self.current_frame.replace(new_frame) {
                let _ = self.returner.send(old_buffer);
            }
        }

        // update debug info from whichever frame is currently active
        if let Some(frame) = &self.current_frame {
            self.latest_debug_info = frame.debug_info;
        }

        // ups
        let time = ctx.input(|i| i.time);
        if time - self.last_measure_time >= 0.5 {
            let current_total = self.latest_debug_info.tick_counter;
            let ticks_passed = current_total.wrapping_sub(self.last_tick_count);
            let time_passed = (time - self.last_measure_time) as f64;
            self.display_ups = (ticks_passed as f64 / time_passed) as u64;
            self.last_tick_count = current_total;
            self.last_measure_time = time;
        }

        // render loop
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::from_rgb(10, 10, 15)))
            .show(ctx, |ui| {
                let painter = ui.painter();

                // We only render if we actually have a frame buffer
                if let Some(frame) = &self.current_frame {
                    self.render_waves(painter, frame);
                    self.render_agents(painter, frame);
                }

                self.render_debug_window(ctx);
                self.render_inspection_window(ctx);
                ctx.request_repaint();
            });
    }
}
