// gui.rs

use crate::components::Transform;
use crate::engine::{DebugInfo, Engine, EngineCommand, FrameData, InspectionData};
use crate::wave::{LevelMask, SignalField};

pub use crossbeam_channel as crossbeam;
pub use eframe::egui;
use glam::Vec2;
use hecs::Entity;
use std::thread;

pub enum ProducerCommand {
    PLAY,
    PAUSE,
    STEP,
}

pub enum Command {
    Producer(ProducerCommand),
    Engine(EngineCommand),
}

impl From<EngineCommand> for Command {
    fn from(cmd: EngineCommand) -> Self {
        Command::Engine(cmd)
    }
}

pub struct Producer {
    returner: crossbeam::Sender<FrameData>,
    receiver: crossbeam::Receiver<FrameData>,
    command_receiver: crossbeam::Receiver<Command>,
    //
    to_tick: bool,
    single_step: bool,
}

impl Producer {
    pub fn new(
        engine_receiver: crossbeam::Receiver<FrameData>,
        engine_returner: crossbeam::Sender<FrameData>,
        command_receiver: crossbeam::Receiver<Command>,
    ) -> Self {
        Self {
            returner: engine_returner,
            receiver: engine_receiver,
            command_receiver: command_receiver,
            to_tick: true,
            single_step: false,
        }
    }

    // Takes ownership of Engine and runs it in a background thread
    pub fn run_thread(mut self, mut engine: Engine) {
        thread::spawn(move || {
            loop {
                match self.command_receiver.try_recv() {
                    Ok(command) => match command {
                        Command::Producer(producer_command) => {
                            self.handle(producer_command);
                        }
                        Command::Engine(engine_command) => {
                            engine.handle(engine_command);
                        }
                    },
                    Err(_) => {}
                }

                // Inside Producer::run_thread's loop
                if self.to_tick {
                    // Normal play mode: uses the internal while-loop with FIXED_DT
                    engine.tick();
                } else if self.single_step {
                    // Debug step mode: forces exactly one simulation frame
                    engine.tick_once();
                    self.single_step = false;
                }

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

    fn handle(&mut self, command: ProducerCommand) {
        match command {
            ProducerCommand::PLAY => {
                self.to_tick = true;
            }
            ProducerCommand::PAUSE => {
                self.to_tick = false;
            }
            ProducerCommand::STEP => {
                self.single_step = true;
            }
        }
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
    command_sender: crossbeam::Sender<Command>,
    //
    // UPS (Physics) Smoothing
    last_tick_count: u64,   // Snapshot of total ticks 0.5s ago
    last_measure_time: f64, // Timestamp of the last check
    display_ups: u64,
    selected_grid_level: usize,
    //
    // local "Double Buffer"
    // It stays here so we can draw it even if the engine is busy ticking.
    current_frame: Option<FrameData>,
    last_viewport_size: egui::Vec2, // Store the size from the previous frame
}

impl Presenter {
    pub fn new(
        frame_receiver: crossbeam::Receiver<FrameData>,
        frame_returner: crossbeam::Sender<FrameData>,
        command_sender: crossbeam::Sender<Command>,
    ) -> Self {
        Self {
            receiver: frame_receiver,
            returner: frame_returner,
            command_sender: command_sender,
            //
            last_tick_count: 0,
            last_measure_time: 0.0,
            display_ups: 0,
            selected_grid_level: 0,
            //
            current_frame: Option::default(),
            last_viewport_size: egui::Vec2::new(0.0, 0.0),
        }
    }

    pub fn run(self) -> eframe::Result<()> {
        // frames
        let width: usize = 1024;
        let height: usize = 768;

        for _ in 0..2 {
            let _ = self.returner.send(FrameData {
                inspection_view: InspectionData {
                    entity: Entity::DANGLING,
                    xform: Transform::default(),
                    emitters: Vec::new(),
                },
                inspection_entities: Vec::new(),
                agents: Vec::new(),
                // signals: Vec::new(),
                debug_info: DebugInfo::default(),
                camera_xform: Transform::default(),
                internal_res: Vec2::new(width as f32, height as f32),
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

    fn render_hierarchy_window(
        ctx: &egui::Context,
        frame: &mut FrameData,
        command_channel: &mut crossbeam::Sender<Command>,
    ) {
        let padding = 5.0;
        egui::Window::new("Scene Hierarchy")
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(padding, -padding))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .default_width(200.0)
            .default_height(400.0)
            .vscroll(false) // We handle scrolling manually with show_rows
            .show(ctx, |ui| {
                // 1. Search / Filter (Optional, adds polish)
                ui.horizontal(|ui| {
                    // If you want a search bar, you'd store the string in 'Presenter' struct
                    // and pass it here. For now, just a placeholder header.
                    ui.label(format!("Entities: {}", frame.inspection_entities.len()));
                });
                ui.separator();

                // 2. Virtualized List
                // We use show_rows to only render what is visible on screen.
                let row_height = 18.0; // Height of one label
                let total_rows = frame.inspection_entities.len();

                egui::ScrollArea::vertical().show_rows(
                    ui,
                    row_height,
                    total_rows,
                    |ui, row_range| {
                        // 'row_range' tells us which indices are currently visible (e.g., 10..40)
                        for i in row_range {
                            if let Some((entity, label)) = frame.inspection_entities.get(i) {
                                // 3. Selection Logic
                                // Check if this row is the currently selected one
                                let is_selected = frame.inspection_view.entity == *entity;

                                // We assume 'Label' is your component wrapper Label(String).
                                // If it is just a String, use 'label' directly.
                                let text = &label.name;

                                // 4. Draw the Button
                                if ui.selectable_label(is_selected, text).clicked() {
                                    // 5. Send Command
                                    // We write to the command buffer. The engine will pick this up
                                    // next tick and populate 'inspection_view' with the new data.
                                    let _ = command_channel
                                        .send(EngineCommand::SelectEntity(*entity).into());
                                }
                            }
                        }
                    },
                );
            });
    }

    fn render_debug_window(
        ctx: &egui::Context,
        frame: &FrameData,
        command_channel: &mut crossbeam::Sender<Command>,
        display_ups: u64,
        selected_level: &mut usize,
    ) {
        let padding = 5.0;
        egui::Window::new("Monitor")
            .resizable(false)
            .collapsible(true)
            .default_pos([padding, padding])
            .default_width(0.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("▶ Play").clicked() {
                        let _ = command_channel.send(Command::Producer(ProducerCommand::PLAY));
                    }
                    if ui.button("⏸ Pause").clicked() {
                        let _ = command_channel.send(Command::Producer(ProducerCommand::PAUSE));
                    }
                    if ui.button("⏭  Step").clicked() {
                        let _ = command_channel.send(Command::Producer(ProducerCommand::STEP));
                    }
                });
                ui.separator();

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
                    ui.label("Render Time:");
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
                    ui.label("Tick Time:");
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

                ///////////////////////
                ui.separator();
                // 1. Collect only the indices that are actually active in the grid
                let active_indices: Vec<usize> =
                    frame.debug_info.active_levels_mask.iter_ones().collect();

                if !active_indices.is_empty() {
                    // 2. We need to find the "index of the current level" in our active list
                    // to keep the slider position consistent.
                    let mut current_selection_idx = active_indices
                        .iter()
                        .position(|&idx| idx == *selected_level)
                        .unwrap_or(0);

                    // 3. Slider over the AVAILABLE indices only
                    let max_idx = active_indices.len() - 1;
                    let res = ui.add(
                        egui::Slider::new(&mut current_selection_idx, 0..=max_idx)
                            .custom_formatter(|idx, _| {
                                // Show the actual Level value (e.g. "Level 5") rather than the list index
                                format!("Level {}", active_indices[idx as usize])
                            }),
                    );

                    // 4. Update the actual selected_level if the slider moved
                    if res.changed() {
                        *selected_level = active_indices[current_selection_idx];
                    }

                    // Display the pixel size for the currently active choice
                    ui.label(format!("Cell Size: {}", 1 << *selected_level));
                } else {
                    ui.colored_label(egui::Color32::GRAY, "No active spatial levels");
                }
                ///////////////////////
            });
    }

    fn render_inspection_window(
        ctx: &egui::Context,
        view: &mut InspectionData,
        command_channel: &mut crossbeam::Sender<Command>,
    ) {
        let padding = 5.0;
        egui::Window::new("Inspector")
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(padding, -padding))
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

                Self::render_component_transform(ui, view, command_channel);
                Self::render_component_emitter(ui, view, command_channel);
            });
    }

    fn render_component_transform(
        ui: &mut egui::Ui,
        view: &mut InspectionData,
        command_channel: &mut crossbeam::Sender<Command>,
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
                                let _ = command_channel.send(
                                    EngineCommand::UpdateTransform(view.entity, *transform).into(),
                                );
                            }
                            ui.label("Y");
                            let response =
                                ui.add(egui::DragValue::new(&mut transform.position.y).speed(1.0));
                            if response.changed() {
                                let _ = command_channel.send(
                                    EngineCommand::UpdateTransform(view.entity, *transform).into(),
                                );
                            }
                        });
                        ui.end_row();

                        // Scale
                        ui.label("Scale");
                        ui.horizontal(|ui| {
                            ui.label("X");
                            let response = ui.add(
                                egui::DragValue::new(&mut transform.scale)
                                    .speed(1.0)
                                    .range(0.1..=f32::MAX),
                            );
                            if response.changed() {
                                let _ = command_channel.send(
                                    EngineCommand::UpdateTransform(view.entity, *transform).into(),
                                );
                            }
                            ui.label("Y");
                            let response = ui.add(
                                egui::DragValue::new(&mut transform.scale)
                                    .speed(1.0)
                                    .range(0.1..=f32::MAX),
                            );
                            if response.changed() {
                                let _ = command_channel.send(
                                    EngineCommand::UpdateTransform(view.entity, *transform).into(),
                                );
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
                                let _ = command_channel.send(
                                    EngineCommand::UpdateTransform(view.entity, *transform).into(),
                                );
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
        command_channel: &mut crossbeam::Sender<Command>,
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
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut emitter.radius_max)
                                            .range(emitter.radius_min..=f32::MAX)
                                            .speed(0.01),
                                    )
                                    .changed()
                                {
                                    let _ = command_channel.send(
                                        EngineCommand::UpdateSignal(view.entity, *emitter).into(),
                                    );
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
                                    let _ = command_channel.send(
                                        EngineCommand::UpdateSignal(view.entity, *emitter).into(),
                                    );
                                }
                                ui.end_row();

                                ui.label("Aperture");
                                let mut degrees = emitter.cone_angle.to_degrees();
                                if ui
                                    .add(egui::Slider::new(&mut degrees, 0.0..=360.0).suffix("°"))
                                    .changed()
                                {
                                    emitter.cone_angle = degrees.to_radians();
                                    let _ = command_channel.send(
                                        EngineCommand::UpdateSignal(view.entity, *emitter).into(),
                                    );
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
        let stroke_color = egui::Color32::from_rgba_unmultiplied(120, 120, 120, 135);
        let stroke_width = 0.3;
        //
        let viewport = painter.clip_rect();
        let camera = &frame.camera_xform;
        let zoom = 1.0 / camera.scale; // Calculate zoom once
        //
        for data in &frame.agents {
            let color = egui::Color32::from_rgba_unmultiplied(
                data.color[0],
                data.color[1],
                data.color[2],
                data.color[3],
            );

            let signal = &data.signal;

            // 2. Project the origin
            let origin = Self::world_to_screen(signal.origin, frame, viewport);

            // 3. Scale the radii by zoom (Division by scale)
            let radius_max = signal.outer_radius * zoom;
            let radius_min = signal.inner_radius * zoom;

            // C. Calculate the Half-Angle
            let half_angle = signal.angle_radians * 0.5;

            // OPTIMIZATION: Full Circle
            // If ~360 degrees and NOT hollow, use hardware circle
            if half_angle > std::f32::consts::PI - 0.01 && radius_min < 0.1 {
                // ---------------------------------------------------------
                // Draw Occlusion Label
                // ---------------------------------------------------------
                // if let Some(occlusion_count) = data.label {
                //     // 1. Scale Font Size:
                //     // Multiply a base size (e.g., 14.0 world units) by the zoom factor.
                //     // Clamp it to prevent it from becoming microscopic or massive if needed.
                //     let font_size = (11.0 * zoom).max(1.0);
                //
                //     // 2. Position at Top:
                //     // Subtract the visual radius from the Y coordinate to move it to the top edge.
                //     // We add a small padding (e.g. -2.0 * zoom) so it sits slightly outside the circle.
                //     let top_pos = egui::pos2(origin.x, origin.y - radius_max - (0.5 * zoom));
                //
                //     painter.text(
                //         top_pos,
                //         egui::Align2::CENTER_BOTTOM, // Anchor BOTTOM so the text sits *on top* of the point
                //         occlusion_count.to_string(),
                //         egui::FontId::proportional(font_size),
                //         egui::Color32::GRAY,
                //     );
                // }

                painter.circle(
                    origin,
                    radius_max,
                    color,
                    egui::Stroke::new(stroke_width, stroke_color),
                );
                continue;
            }

            // D. Manual Mesh Generation (Works for all angles)
            let base_angle = signal.unit_direction.y.atan2(signal.unit_direction.x);
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
                let outer_pos =
                    egui::pos2(origin.x + radius_max * cos, origin.y + radius_max * sin);

                // Inner Vertex
                let inner_pos = if radius_min > 0.0 {
                    egui::pos2(origin.x + radius_min * cos, origin.y + radius_min * sin)
                } else {
                    origin
                };

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
            if radius_min > 0.0 {
                // We need to regenerate or store them. Storing is easier but we didn't store inner list.
                // Re-calculating loop in reverse is cheap.
                for i in (0..=steps).rev() {
                    let theta = start_angle + (i as f32 * angle_step);
                    let (sin, cos) = theta.sin_cos();
                    outline_points.push(egui::pos2(
                        origin.x + radius_min * cos,
                        origin.y + radius_min * sin,
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

    fn render_grid(painter: &egui::Painter, frame: &FrameData, selected_level: usize) {
        // 1. Check if the level is actually active
        let mask = frame.debug_info.active_levels_mask;
        if selected_level >= mask.len() || !mask[selected_level] {
            return;
        }

        // 2. Setup Camera and Geometry
        let camera = &frame.camera_xform;
        let zoom = 1.0 / camera.scale;
        let screen_rect = painter.clip_rect();

        // World-space size of one cell (e.g., 64.0)
        let world_cell_size = SignalField::get_level_size(selected_level) as f32;
        // Screen-space size (e.g., 32.0 pixels if zoomed out)
        let screen_cell_size = world_cell_size * zoom;

        // --- AABB TILE PAINTING ---
        if !frame.inspection_view.emitters.is_empty() {
            let view = &frame.inspection_view;
            let emitter = &view.emitters[0];

            // World-space radius and position of the selection
            let world_radius = emitter.radius_max * view.xform.scale;
            let world_pos = view.xform.position;

            let min_aabb = world_pos - glam::Vec2::splat(world_radius);
            let max_aabb = world_pos + glam::Vec2::splat(world_radius);

            // Get the grid tile indices from the engine
            let (min_g, max_g) = SignalField::get_tile_range(min_aabb, max_aabb, selected_level);

            let highlight_color = egui::Color32::from_rgba_unmultiplied(80, 80, 80, 25);

            for gx in min_g.x..max_g.x {
                for gy in min_g.y..max_g.y {
                    // Determine the World-Space corners of this specific tile
                    let world_tile_min =
                        glam::Vec2::new(gx as f32 * world_cell_size, gy as f32 * world_cell_size);
                    let world_tile_max = glam::Vec2::new(
                        (gx + 1) as f32 * world_cell_size,
                        (gy + 1) as f32 * world_cell_size,
                    );

                    // Project those World coordinates to Screen pixels
                    let tile_min = Self::world_to_screen(world_tile_min, frame, screen_rect);
                    let tile_max = Self::world_to_screen(world_tile_max, frame, screen_rect);

                    painter.rect_filled(
                        egui::Rect::from_min_max(tile_min, tile_max),
                        0.0,
                        highlight_color,
                    );
                }
            }
        }

        // --- WIREFRAME RENDERING ---
        let grid_lines_color = egui::Color32::from_rgba_unmultiplied(31, 31, 31, 80);
        let stroke = egui::Stroke::new(1.0, grid_lines_color);

        // Find where the World (0,0) point is currently located on your monitor
        let world_zero = Self::world_to_screen(glam::Vec2::ZERO, frame, screen_rect);

        // 1. Vertical Lines
        // Use modulo to find the first line position relative to the viewport edge
        let mut x = world_zero.x % screen_cell_size;
        while x < screen_rect.min.x {
            x += screen_cell_size;
        }
        while x > screen_rect.min.x + screen_cell_size {
            x -= screen_cell_size;
        }

        while x <= screen_rect.max.x {
            painter.line_segment(
                [
                    egui::pos2(x, screen_rect.min.y),
                    egui::pos2(x, screen_rect.max.y),
                ],
                stroke,
            );
            x += screen_cell_size;
        }

        // 2. Horizontal Lines
        let mut y = world_zero.y % screen_cell_size;
        while y < screen_rect.min.y {
            y += screen_cell_size;
        }
        while y > screen_rect.min.y + screen_cell_size {
            y -= screen_cell_size;
        }

        while y <= screen_rect.max.y {
            painter.line_segment(
                [
                    egui::pos2(screen_rect.min.x, y),
                    egui::pos2(screen_rect.max.x, y),
                ],
                stroke,
            );
            y += screen_cell_size;
        }
    }

    // takes an world position and calculates exactly which monitor pixel should represent it
    fn world_to_screen(world_pos: Vec2, frame: &FrameData, viewport: egui::Rect) -> egui::Pos2 {
        let camera = &frame.camera_xform;

        // 1. Calculate the "Base Scale" (How many pixels = 1 world unit)
        // We anchor to the Height so that vertical view remains consistent.
        let base_scale = viewport.height() / frame.internal_res.y;

        // 2. Combine with Camera Zoom (1.0 / scale)
        let total_scale = base_scale * (1.0 / camera.scale);

        // 3. Translation: World-to-Camera relative distance
        let relative = world_pos - camera.position;

        // 4. Final Mapping to Screen
        // We multiply the relative distance by the total scale and add to center
        let screen_offset = egui::vec2(relative.x * total_scale, relative.y * total_scale);
        viewport.center() + screen_offset
    }

    // Untested
    pub fn screen_to_world(
        screen_pos: egui::Pos2,
        frame: &FrameData,
        viewport: egui::Rect,
    ) -> Vec2 {
        let camera = &frame.camera_xform;

        // 1. Calculate the same base scale used for rendering
        let base_scale = viewport.height() / frame.internal_res.y;

        // 2. The Total Scale (Pixels per World Unit)
        let total_scale = base_scale * (1.0 / camera.scale);

        // 3. Get the vector from the center of the screen to the mouse click
        let relative_screen = screen_pos - viewport.center();

        // 4. Divide by the scale to get World Units
        let relative_world = Vec2::new(
            relative_screen.x / total_scale,
            relative_screen.y / total_scale,
        );

        // 5. Add back the camera position to get absolute world coords
        camera.position + relative_world
    }
}

impl eframe::App for Presenter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let current_size = ctx.viewport_rect().size();

        if current_size != self.last_viewport_size {
            // 1. Update the local tracker
            self.last_viewport_size = current_size;

            // 2. Dispatch ONLY once per resize
            let _ = self.command_sender.send(
                EngineCommand::UpdateViewport(glam::Vec2::new(current_size.x, current_size.y))
                    .into(),
            );

            println!("Viewport resized to: {:?}", current_size); // Debug confirmation
        }

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

            // Grid default
            let mask = frame.debug_info.active_levels_mask;

            // Only snap if the current level is no longer valid
            if !mask[self.selected_grid_level] {
                // Find the closest active bit index
                if let Some(closest) = mask
                    .iter_ones()
                    .min_by_key(|&bit| (bit as i32 - self.selected_grid_level as i32).abs())
                {
                    self.selected_grid_level = closest;
                }
            }

            // Render loop (Modifies frame.inspection_view)
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(egui::Color32::from_rgb(12, 12, 12)))
                .show(ctx, |ui| {
                    let painter = ui.painter();
                    // Self::render_waves(painter, frame);
                    Self::render_agents(painter, frame);
                    Self::render_grid(painter, frame, self.selected_grid_level);
                    Self::render_hierarchy_window(ctx, frame, &mut self.command_sender);
                    Self::render_debug_window(
                        ctx,
                        frame,
                        &mut self.command_sender,
                        self.display_ups,
                        &mut self.selected_grid_level,
                    );

                    // This is where the frame data is modified
                    Self::render_inspection_window(
                        ctx,
                        &mut frame.inspection_view,
                        &mut self.command_sender,
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
