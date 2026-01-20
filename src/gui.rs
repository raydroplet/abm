// gui.rs

use crate::components::Transform;
use crate::engine::{DebugInfo, Engine, FrameData, InspectionData, InspectionState};
use crate::wave::{LevelMask, Signal};

pub use crossbeam_channel as crossbeam;
pub use eframe::egui;
use hecs::Entity;
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
                        engine.handle(frame.inspection_command);
                        engine.render(&mut frame);

                        // resets the command
                        frame.inspection_command = InspectionState::Idle;

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
                inspection_command: InspectionState::Idle,
                inspection_view: InspectionData {
                    entity: Entity::DANGLING,
                    xform: Transform::default(),
                    emitters: Vec::new(),
                },
                agents: Vec::new(),
                // signals: Vec::new(),
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

    fn render_debug_window(ctx: &egui::Context, frame: &FrameData, display_ups: u64) {
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

                let debug_info = &frame.debug_info;

                ui.horizontal(|ui| {
                    let engine_fps = 1000.0 / debug_info.render_time_ms;
                    ui.label("Potential FPS:");
                    ui.colored_label(egui::Color32::LIGHT_BLUE, format!("{:.0}", engine_fps));
                });

                ui.horizontal(|ui| {
                    ui.label("Simulation UPS:");
                    ui.colored_label(egui::Color32::LIGHT_GREEN, format!("{}", display_ups));
                });

                // ui.separator();

                let engine_ms = debug_info.render_time_ms;
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
                    let phys_ms = debug_info.tick_time_ms;
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

                let wave_count = debug_info.agent_count;

                ui.label(format!("Total Entities: {}", debug_info.agent_count));
                ui.label(format!("Visible Waves:  {}", wave_count));

                ui.label(format!("Ticks Elapsed:  {}", debug_info.tick_counter));
            });
    }

    fn render_inspection_window(
        ctx: &egui::Context,
        view: &mut InspectionData,
        mut command: &mut InspectionState,
    ) {
        let padding = 5.0;
        egui::Window::new("Inspector")
            .default_width(0.0)
            .pivot(egui::Align2::RIGHT_TOP)
            .default_pos(egui::pos2(
                ctx.viewport_rect().max.x - padding,
                ctx.viewport_rect().min.y + padding,
            ))
            .show(ctx, |ui| {
                if view.entity == Entity::DANGLING {
                    ui.label(egui::RichText::new("Entity: None").monospace());
                    return;
                }

                ui.label(egui::RichText::new(format!("Entity #{:?}", view.entity)).monospace());
                ui.separator();

                Self::render_component_transform(ui, view, &mut command);
                Self::render_component_emitter(ui, view, &mut command);
            });
    }

    fn render_component_transform(
        ui: &mut egui::Ui,
        view: &mut InspectionData,
        command: &mut InspectionState,
    ) {
        let transform = &mut view.xform;
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
                            let response =
                                ui.add(egui::DragValue::new(&mut transform.position.x).speed(1.0));
                            if response.changed() {
                                *command =
                                    InspectionState::UpdateTransform(view.entity, *transform);
                            }
                            ui.label("Y");
                            let response =
                                ui.add(egui::DragValue::new(&mut transform.position.y).speed(1.0));
                            if response.changed() {
                                *command =
                                    InspectionState::UpdateTransform(view.entity, *transform);
                            }
                        });
                        ui.end_row();

                        // Scale
                        ui.label("Scale");
                        ui.horizontal(|ui| {
                            ui.label("X");
                            let response =
                                ui.add(egui::DragValue::new(&mut transform.scale).speed(1.0));
                            if response.changed() {
                                *command =
                                    InspectionState::UpdateTransform(view.entity, *transform);
                            }
                            ui.label("Y");
                            let response =
                                ui.add(egui::DragValue::new(&mut transform.scale).speed(1.0));
                            if response.changed() {
                                *command =
                                    InspectionState::UpdateTransform(view.entity, *transform);
                            }
                        });
                        ui.end_row();

                        // Rotation
                        ui.label("Rotation");
                        ui.horizontal(|ui| {
                            let mut rot_deg = transform.rotation.to_degrees();
                            ui.label("Z");
                            let response = ui.add(egui::DragValue::new(&mut rot_deg).speed(1.0));
                            if response.changed() {
                                transform.rotation = rot_deg.to_radians();
                                *command =
                                    InspectionState::UpdateTransform(view.entity, *transform);
                            }
                        });
                        ui.end_row();
                        // ui.end_row(); // Double check if you actually need this second end_row(), it creates a double-height gap.
                    });
            });
    }

    fn render_component_emitter(
        ui: &mut egui::Ui,
        view: &mut InspectionData,
        command: &mut InspectionState,
    ) {
        for emitter in &mut view.emitters {
            egui::CollapsingHeader::new("Signal")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        egui::Grid::new("emitter_grid")
                            .num_columns(2)
                            .spacing([20.0, 1.0])
                            .show(ui, |ui| {
                                ui.label("Outer Radius");
                                let mut radius_max = emitter.radius_max;
                                if ui
                                    .add(egui::DragValue::new(&mut radius_max)
                                        .range(emitter.radius_min..=f32::MAX)
                                        .speed(0.01)
                                    )
                                    .changed()
                                {
                                    emitter.radius_max = radius_max;
                                    *command = InspectionState::UpdateSignal(view.entity, *emitter);
                                }
                                ui.end_row();

                                ui.label("Inner Radius");
                                let mut radius_min = emitter.radius_min;
                                let max = emitter.radius_max;
                                if ui
                                    .add(
                                        egui::Slider::new(&mut radius_min, 0.0..=max)
                                            .drag_value_speed(0.1)
                                            .step_by(0.00001),
                                    )
                                    .changed()
                                {
                                    emitter.radius_min = radius_min;
                                    *command = InspectionState::UpdateSignal(view.entity, *emitter);
                                }
                                ui.end_row();

                                ui.label("Aperture");
                                let mut degrees = emitter.cone_angle.to_degrees();
                                if ui
                                    .add(egui::Slider::new(&mut degrees, 0.0..=360.0).suffix("°"))
                                    .changed()
                                {
                                    emitter.cone_angle = degrees.to_radians();
                                    *command = InspectionState::UpdateSignal(view.entity, *emitter);
                                }
                                ui.end_row();
                            });
                    });

                    ui.separator();

                    let mut mask = LevelMask::ZERO;
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

    fn render_agents(painter: &egui::Painter, frame: &FrameData) {
        let stroke_color = egui::Color32::WHITE;
        let stroke_width = 1.0;

        for data in &frame.agents {
            let color = egui::Color32::from_rgba_unmultiplied(
                data.color[0],
                data.color[1],
                data.color[2],
                data.color[3],
            );
            let signal = &data.signal;
            let origin = egui::pos2(signal.origin.x, signal.origin.y);

            // C. Calculate the Half-Angle
            let half_angle = signal.angle_cos.clamp(-1.0, 1.0).acos();

            // OPTIMIZATION: Full Circle
            // If ~360 degrees and NOT hollow, use hardware circle
            if half_angle > std::f32::consts::PI - 0.01 && signal.inner_radius < 0.1 {
                painter.circle(
                    origin,
                    signal.outer_radius,
                    color,
                    egui::Stroke::new(stroke_width, stroke_color),
                );
                continue;
            }

            // D. Manual Mesh Generation (Works for all angles)
            let base_angle = signal.direction.y.atan2(signal.direction.x);
            let steps = 32;
            let start_angle = base_angle - half_angle;
            let angle_step = (half_angle * 2.0) / (steps as f32);

            // 1. Prepare the Mesh (For Fill)
            let mut mesh = egui::Mesh::default();

            // 2. Prepare the Outline Points (For Stroke)
            // Capacity: (steps+1) for outer + (steps+1) for inner
            let mut outline_points: Vec<egui::Pos2> = Vec::with_capacity((steps + 1) * 2);

            // 3. Generate Vertices
            for i in 0..=steps {
                let theta = start_angle + (i as f32 * angle_step);
                let (sin, cos) = theta.sin_cos();

                // Outer Vertex
                let outer_pos = egui::pos2(
                    origin.x + signal.outer_radius * cos,
                    origin.y + signal.outer_radius * sin,
                );

                // Inner Vertex
                let inner_pos = if signal.inner_radius > 0.0 {
                    egui::pos2(
                        origin.x + signal.inner_radius * cos,
                        origin.y + signal.inner_radius * sin,
                    )
                } else {
                    origin
                };

                // Add to Mesh (Triangle Strip Logic)
                // We add two vertices (Outer, Inner) per step.
                // egui::Mesh uses indexed triangles, but we can just add quads manually.
                if i > 0 {
                    let idx = mesh.vertices.len() as u32;
                    // Previous Outer, Previous Inner
                    // Current Outer, Current Inner
                    // We need the indices of the PREVIOUS two points (idx-2, idx-1)
                    // and the CURRENT two points (which we are about to add).

                    // Actually, simpler approach: Just add colored vertices and let egui triangulate?
                    // No, manual indices are safest.
                }

                // Let's use the simplest robust method:
                // Add vertices, then add indices for a Quad connecting to the previous step.

                mesh.vertices.push(egui::epaint::Vertex {
                    pos: outer_pos,
                    uv: egui::pos2(0.0, 0.0),
                    color: color,
                });
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: inner_pos,
                    uv: egui::pos2(0.0, 0.0),
                    color: color,
                });

                if i > 0 {
                    let base_idx = (mesh.vertices.len() as u32) - 4; // Start of previous pair
                    // 0: Prev Outer, 1: Prev Inner, 2: Curr Outer, 3: Curr Inner
                    // Triangle 1: PrevOuter, PrevInner, CurrInner
                    mesh.add_triangle(base_idx, base_idx + 1, base_idx + 3);
                    // Triangle 2: PrevOuter, CurrInner, CurrOuter
                    mesh.add_triangle(base_idx, base_idx + 3, base_idx + 2);
                }

                // Add to Outline path
                // We push outer points normally
                outline_points.push(outer_pos);
            }

            // 4. Finish Outline Path
            // Now push inner points in REVERSE to close the loop properly for the stroke
            if signal.inner_radius > 0.0 {
                // We need to regenerate or store them. Storing is easier but we didn't store inner list.
                // Re-calculating loop in reverse is cheap.
                for i in (0..=steps).rev() {
                    let theta = start_angle + (i as f32 * angle_step);
                    let (sin, cos) = theta.sin_cos();
                    outline_points.push(egui::pos2(
                        origin.x + signal.inner_radius * cos,
                        origin.y + signal.inner_radius * sin,
                    ));
                }
            } else {
                outline_points.push(origin);
            }

            // 5. Draw
            // A. Draw the solid fill (The Mesh)
            painter.add(egui::Shape::Mesh(mesh.into()));

            // B. Draw the outline (The Path)
            painter.add(egui::Shape::Path(egui::epaint::PathShape {
                points: outline_points,
                closed: true,
                fill: egui::Color32::TRANSPARENT, // Important! Don't fill the path, we did that with the mesh
                stroke: egui::Stroke::new(stroke_width, stroke_color).into(),
            }));
        }
    }
}

impl eframe::App for Presenter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Drain the channel to get the newest frame from the engine
        let mut newest_arrived = None;
        while let Ok(frame) = self.receiver.try_recv() {
            // If we pulled a frame earlier in this loop (newest_arrived is Some),
            // but found an even newer one, the previous one is "skipped".
            // Since skipped frames were never "current" and never modified by GUI,
            // we send them back immediately so the engine doesn't starve.
            if let Some(skipped) = newest_arrived.replace(frame) {
                let _ = self.returner.send(skipped);
            }
        }

        // 2. Prepare a variable to hold the old frame (if we swap)
        let mut buffer_to_return = None;

        // 3. If a new frame arrived, swap it into storage
        if let Some(new_frame) = newest_arrived {
            // We store the old buffer in a local variable instead of sending it immediately.
            // This satisfies the requirement to hold it until the function ends,
            // though usually, the "old" frame is safe to send here.
            buffer_to_return = self.current_frame.replace(new_frame);
        }

        // 4. Update Logic & Rendering
        if let Some(frame) = &mut self.current_frame {
            // UPS Calculation
            let time = ctx.input(|i| i.time);
            if time - self.last_measure_time >= 0.5 {
                let current_total = frame.debug_info.tick_counter;
                let ticks_passed = current_total.wrapping_sub(self.last_tick_count);
                let time_passed = (time - self.last_measure_time) as f64;
                self.display_ups = (ticks_passed as f64 / time_passed) as u64;
                self.last_tick_count = current_total;
                self.last_measure_time = time;
            }

            // Render loop (Modifies frame.inspection_view)
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(egui::Color32::from_rgb(10, 10, 15)))
                .show(ctx, |ui| {
                    let painter = ui.painter();
                    // Self::render_waves(painter, frame);
                    Self::render_agents(painter, frame);
                    Self::render_debug_window(ctx, frame, self.display_ups);

                    // This is where the frame data is modified
                    Self::render_inspection_window(
                        ctx,
                        &mut frame.inspection_view,
                        &mut frame.inspection_command,
                    );
                });

            ctx.request_repaint();
        }

        // 5. Finally, send the old buffer back to the engine
        // We do this at the very end, ensuring all GUI work for this tick is complete.
        if let Some(old_buffer) = buffer_to_return {
            let _ = self.returner.send(old_buffer);
        }
    }
}
