// engine.rs

use crate::components::*;
use crate::wave::{LevelMask, Signal, SignalField, SignalMask};
use hecs::Entity;

use glam::Vec2;
use hecs::World;
use rand::Rng;
use std::f32::consts::TAU;
use std::time::Instant; // or f64::consts::TAU

const FIXED_DT: f32 = 1.0 / 100.0; // Run physics exactly 100 times a second
const BIT_BOUNDING_VOLUME: usize = 0;

#[derive(Clone, Copy, Debug)]
pub struct AgentRenderData {
    pub signal: Signal,
    pub color: [u8; 4],
    pub label: Option<u8>,
    // Add extra fields here if your shaders need them (e.g., velocity for motion blur)
}

#[derive(Default, Clone, Copy)]
pub struct DebugInfo {
    pub tick_counter: u64,   // overflows in ~584,000 years at 1.000.000hz
    pub agent_count: usize,  // Useful since you are using ECS
    pub render_time_ms: f32, // Render time
    pub tick_time_ms: f32,   //
    //
    pub active_levels_mask: LevelMask,
}

#[derive(Copy, Clone)]
pub enum EngineCommand {
    UpdateTransform(Entity, Transform),
    UpdateSignal(Entity, SignalEmitter),
    SelectEntity(Entity),
}

#[derive(Debug)]
pub struct InspectionData {
    pub entity: hecs::Entity,
    pub xform: Transform,
    pub emitters: Vec<SignalEmitter>,
}

pub struct FrameData {
    pub agents: Vec<AgentRenderData>, // The "Points" for the GPU
    pub debug_info: DebugInfo,
    //
    //////
    // 1. THE VIEW (Read-Only for GUI, Write-Only for Engine)
    // The Engine guarantees this is always populated with the latest reality.
    // The GUI just reads this to draw the window.
    pub inspection_view: InspectionData,
    // 2. THE COMMAND (Write-Only for GUI, Read-Only for Engine)
    // The GUI only touches this if the user CHANGED something.
    // The Engine checks this to see if it needs to update the ECS.
    pub inspection_entities: Vec<(Entity, Label)>,
}

//////////////////

pub struct Engine {
    world: World,         // entity component system
    last_update: Instant, // used for delta_time calculation
    //
    tick_counter: u64,      // overflows in ~584,000 years at 1.000.000hz
    time_accumulator: f32,  //
    last_tick_time_ms: f32, //
    //
    signal_field: SignalField,
    //
    camera_dimension: Vec2,
    camera_position: Vec2,
    //
    selected_entity: Entity,
}

impl Engine {
    pub fn new() -> Self {
        let mut world = World::new();
        let mut signal_field = SignalField::new(); // User variable name
        let width = 1024.0;
        let height = 768.0;

        Self::spawn_dummy_entities(width, height, &mut world, &mut signal_field);
        let (player_id, player_vision_id) = Self::spawn_dummy_player(&mut world, &mut signal_field);

        Self {
            world,
            last_update: Instant::now(),
            tick_counter: 0,
            time_accumulator: 0.0,
            last_tick_time_ms: 0.0,
            signal_field, // Store the layer
            camera_dimension: Vec2::new(width, height),
            camera_position: Vec2::new(0.0, 0.0),
            selected_entity: player_vision_id,
        }
    }

    pub fn tick(&mut self) {
        // time management
        let start_update = Instant::now();
        let delta_time = start_update.duration_since(self.last_update).as_secs_f32();
        self.last_update = start_update;
        self.time_accumulator += delta_time;

        self.system_simple_physics();
        //
    }

    pub fn render(&self, frame: &mut FrameData) {
        let start_render = Instant::now();

        // 1. Prepare for new frame
        frame.agents.clear();
        // frame.signals.clear();

        let mut signal_mask = SignalMask::default();
        signal_mask.set(BIT_BOUNDING_VOLUME, true);
        // signal_mask.set(BIT_PASSIVE, true);

        let mut layer_mask = SignalMask::default();
        layer_mask.fill(true);

        // let mut filter = SignalMask::<1>::default();
        // filter.set(BIT_RENDER, true);

        // // 2. Spatial Query
        // // We only care about agents the camera can actually see
        // self.signal_field.scan_volume_rectangle(
        //     self.camera_position,
        //     self.camera_dimension + self.camera_position,
        //     signal_mask,
        //     layer_mask,
        //     |signal, entity| {
        //         // A. Store signal for the GUI/Debug overlays
        //         // if signal.mask != filter {
        //         // frame.signals.push(signal.clone());
        //         // }
        //
        //         // B. Fetch visual data from ECS for the GPU
        //         if let Ok(mut query) = self.world.query_one::<&Model>(*entity) {
        //             if let Some(model) = query.get() {
        //                 frame.agents.push(AgentRenderData {
        //                     signal: signal.clone(),
        //                     color: [model.r, model.g, model.b, model.a],
        //                     label: None,
        //                 });
        //             }
        //         }
        //     },
        // );

        self.signal_field
            .scan(self.selected_entity, layer_mask, |signal, entity| {
                // B. Fetch visual data from ECS for the GPU
                if let Ok(mut query) = self.world.query_one::<&Model>(*entity) {
                    if let Some(model) = query.get() {
                        frame.agents.push(AgentRenderData {
                            signal: signal.clone(),
                            color: [model.r, model.g, model.b, model.a],
                            label: None,
                        });
                    }
                }
            });

        // 3. Metadata
        frame.debug_info.tick_counter = self.tick_counter;
        frame.debug_info.agent_count = self.world.len() as usize;
        frame.debug_info.render_time_ms = start_render.elapsed().as_secs_f32() * 1000.0;
        //
        frame.debug_info.tick_time_ms = self.last_tick_time_ms;
        frame.debug_info.active_levels_mask = self.signal_field.get_level_mask();
        //
        // for the inspection window
        if self.selected_entity != Entity::DANGLING {
            let view = &mut frame.inspection_view;
            view.entity = self.selected_entity;
            view.emitters.clear();

            if let Ok(mut query) = self
                .world
                .query_one::<(&Transform, &SignalEmitter)>(self.selected_entity)
            {
                if let Some((transform, emitter)) = query.get() {
                    view.xform = *transform;
                    view.emitters.push(*emitter);
                }
            }
        }
        frame.inspection_entities.clear();
        for (entity, label) in self.world.query::<&Label>().iter() {
            frame.inspection_entities.push((entity, label.clone()));
        }
    }

    ///////////////////

    fn spawn_dummy_entities(
        width: f32,
        height: f32,
        world: &mut World,
        signal_field: &mut SignalField,
    ) {
        let mut rng = rand::rng();

        for _ in 0..10000 {
            // Random Data
            let rand_pos_x = rng.random_range(0.0..width);
            let rand_pos_y = rng.random_range(0.0..height);
            let rand_vel_x = rng.random_range(-100.0..100.0);
            let rand_vel_y = rng.random_range(-100.0..100.0);
            // let r_val = rng.random_range(0..=255);
            // let g_val = rng.random_range(0..=255);
            // let b_val = rng.random_range(0..=255);
            let color = rng.random_range(200..=255);

            let pos = Vec2::new(rand_pos_x, rand_pos_y);
            let radius = 3.0;
            let scale = 2.0;

            // 1. RESERVE ID (Critical for correct linking)
            let id = world.reserve_entity();

            // 2. Prepare Masks
            let mut signal_mask = SignalMask::default();
            signal_mask.set(BIT_BOUNDING_VOLUME, true);
            let layer_mask = SignalMask::default();

            // 3. Create Emitter (Sphere Factory)
            let emitter = SignalEmitter {
                radius_max: radius,
                radius_min: 0.0,
                cone_angle: std::f32::consts::TAU,
                emit_mask: signal_mask,
                sense_mask: signal_mask,
                layer_mask: layer_mask,
            };

            let signal = Signal {
                origin: pos,
                direction: Vec2::new(0.0, 0.0), // Rotation (irrelevant for sphere)
                outer_radius: radius,           // Outer Radius
                inner_radius: 0.0,
                angle_cos: (std::f32::consts::TAU * 0.5).cos(), // Cone Angle: 2*PI (Full Sphere)
                emit_mask: signal_mask,
                sense_mask: signal_mask,
            };

            signal_field.emit(signal, id);

            // 4. ATOMIC SPAWN
            // Everything enters the world at the exact same moment
            world.spawn_at(
                id,
                (
                    Transform {
                        position: pos,
                        scale: scale,
                        ..Transform::default()
                    },
                    Velocity {
                        linear: Vec2::new(rand_vel_x, rand_vel_y),
                        ..Velocity::default()
                    },
                    Model {
                        r: color,
                        g: color,
                        b: color,
                        a: 0,
                    },
                    emitter, // <--- Attached immediately
                ),
            );
        }
    }

    fn system_sync_spatial(world: &mut World) {
        let mut buffer = Vec::new(); // not bothering with allocations for now

        // Pass 1: Read & Calc
        for (child_id, (anchor, _)) in world.query::<(&SpatialAnchor, &Transform)>().iter() {
            if let Ok(parent) = world.get::<&Transform>(anchor.parent) {
                buffer.push((child_id, parent.position + anchor.position_offset));
            }
        }

        // Pass 2: Write
        for (child_id, pos) in buffer.iter() {
            if let Ok(mut child) = world.get::<&mut Transform>(*child_id) {
                child.position = *pos;
            }
        }
    }

    fn system_simple_physics(&mut self) {
        // SPIRAL OF DEATH PROTECTION:
        // If the game lags hard (0.25s freeze), don't try to catch up
        // by running 15 physics steps instantly. Just cap it.
        //
        // the logic inside the while loop is currently too fast for this to happen,
        // but it will stay here in case of future needs
        if self.time_accumulator > 0.25 {
            println!("heavy processing. slowing time down!");
            self.time_accumulator = 0.25;
        }

        // fixed update loop
        // We only update physics in chunks of FIXED_DT (e.g., 0.0166s)
        while self.time_accumulator >= FIXED_DT {
            let tick_time = Instant::now();

            // 1. MOVEMENT LOOP (Updates Position)
            // We now query for Anchor optionally.
            // This lets us differentiate between Roots and Children.
            for (_id, (xform, vel, anchor_opt)) in
                self.world
                    .query_mut::<(&mut Transform, &mut Velocity, Option<&mut SpatialAnchor>)>()
            {
                // handles local vs global positions
                match anchor_opt {
                    Some(anchor) => {
                        // the sync system will update the transform world position latter
                        anchor.position_offset += vel.linear * FIXED_DT;

                        // Note: We skip world-bounds collision for children.
                        // They stick to their parents even if the parent drags them through a wall.
                    }
                    None => {
                        xform.position += vel.linear * FIXED_DT;

                        // Collision Logic (Bounce off walls)
                        if xform.position.x >= self.camera_dimension.x {
                            vel.linear.x *= -1.0;
                            xform.position.x = self.camera_dimension.x;
                        }
                        if xform.position.x <= 0.0 {
                            vel.linear.x *= -1.0;
                            xform.position.x = 0.0;
                        }
                        if xform.position.y >= self.camera_dimension.y {
                            vel.linear.y *= -1.0;
                            xform.position.y = self.camera_dimension.y;
                        }
                        if xform.position.y <= 0.0 {
                            vel.linear.y *= -1.0;
                            xform.position.y = 0.0;
                        }
                    }
                }
            }
            // 2. SPATIAL SYNC (The Cache Refresh)
            // This propagates ParentPos + AnchorOffset -> ChildPos
            // It ensures the Transform is correct before we use it for Signals/Rendering.
            Self::system_sync_spatial(&mut self.world);

            // 2. SIGNAL SYNC LOOP (Updates SignalLayer)
            // We iterate over entities that have BOTH Position and Emitter
            for (id, (xform, emitter)) in self.world.query_mut::<(&Transform, &SignalEmitter)>() {
                // Reposition the signal to match the agent
                // We split the borrow here: 'pos' is from 'world', 'reposition' is on 'signal_layer'
                self.signal_field.reposition(
                    id,
                    xform.position,
                    emitter.radius_min * xform.scale, // Keep the radius constant (or pulse it here!)
                    emitter.radius_max * xform.scale, // Keep the radius constant (or pulse it here!)
                );
            }

            self.time_accumulator -= FIXED_DT;
            self.tick_counter += 1;

            self.last_tick_time_ms = tick_time.elapsed().as_secs_f32() * 1000.0;
        }
    }

    fn spawn_dummy_player(world: &mut World, signal_field: &mut SignalField) -> (Entity, Entity) {
        let player_pos = Vec2::new(512.0, 384.0);
        let player_scale = 20.0;

        // =========================================================
        // 1. THE PLAYER (Ring Shape)
        // =========================================================
        let player_id = world.reserve_entity();

        let mut signal_mask = SignalMask::default();
        let layer_mask = SignalMask::default();
        signal_mask.set(BIT_BOUNDING_VOLUME, true);

        let outer_rad = 1.0;
        let inner_rad = 0.0;
        let cone_angle = TAU;
        // let cone_angle = 2.094; // 120 degrees

        let player_emitter = SignalEmitter {
            radius_max: outer_rad, // Base size
            radius_min: inner_rad, // Normalized inner (14/15)
            cone_angle: cone_angle,
            emit_mask: signal_mask,
            sense_mask: signal_mask,
            layer_mask: layer_mask,
        };

        let player_signal = Signal {
            origin: player_pos,
            direction: Vec2::new(1.0, 0.0), // Default 0.0 rotation (Right)
            outer_radius: outer_rad,
            inner_radius: inner_rad,
            angle_cos: (cone_angle * 0.5).cos(),
            emit_mask: signal_mask,
            sense_mask: signal_mask,
        };

        signal_field.emit(player_signal, player_id);

        world.spawn_at(
            player_id,
            (
                Label {
                    name: String::from("Player"),
                },
                Transform {
                    position: player_pos,
                    scale: player_scale,
                    rotation: 0.0,
                },
                // Velocity {
                //     linear: Vec2::new(200.0, 200.0),
                //     ..Velocity::default()
                // },
                Model {
                    r: 000,
                    g: 200,
                    b: 000,
                    a: 100,
                },
                player_emitter,
            ),
        );

        // =========================================================
        // 2. THE CHILD SCANNER (Omni Sensor)
        // =========================================================
        let player_vision_id = world.reserve_entity();
        let scanner_range = 1.0;
        let scale = 200.0;
        let cone_angle = TAU / 4.0;

        // Component
        let child_emitter = SignalEmitter {
            radius_max: scanner_range, // Large range
            radius_min: 0.0,
            cone_angle: cone_angle,
            emit_mask: signal_mask,
            sense_mask: signal_mask,
            layer_mask: layer_mask,
        };

        let child_signal = Signal {
            origin: player_pos, // Starts at parent position
            direction: Vec2::X,
            outer_radius: scanner_range,
            inner_radius: 0.0,
            angle_cos: (cone_angle * 0.5).cos(), // cos(PI) for full sphere
            emit_mask: signal_mask,
            sense_mask: signal_mask,
        };

        signal_field.emit(child_signal, player_vision_id);

        world.spawn_at(
            player_vision_id,
            (
                Label {
                    name: String::from("Vision"),
                },
                Transform {
                    position: player_pos,
                    scale: scale,
                    rotation: 0.0,
                },
                SpatialAnchor {
                    parent: player_id,
                    position_offset: Vec2::ZERO,
                },
                Model {
                    r: 200,
                    g: 200,
                    b: 200,
                    a: 100,
                },
                child_emitter,
            ),
        );

        (player_id, player_vision_id)
    }

    pub fn handle(&mut self, command: EngineCommand) {
        match command {
            EngineCommand::UpdateTransform(entity, new_transform) => {
                // 1. Update the Transform (Cache)
                if let Ok(mut xform) = self.world.get::<&mut Transform>(entity) {
                    *xform = new_transform;
                }

                // 2. Update the Anchor
                if let Ok(mut anchor) = self.world.get::<&mut SpatialAnchor>(entity) {
                    if let Ok(parent_xform) = self.world.get::<&Transform>(anchor.parent) {
                        anchor.position_offset = new_transform.position - parent_xform.position;
                    }
                }

                if let Ok(mut query) = self
                    .world
                    .query_one::<(&mut Transform, &SignalEmitter)>(entity)
                {
                    if let Some((xform, emitter)) = query.get() {
                        // Update Signal
                        self.signal_field.reposition(
                            entity,
                            xform.position,
                            // Keep the radius constant (or pulse it here!)
                            emitter.radius_min * xform.scale,
                            emitter.radius_max * xform.scale,
                        );
                        self.signal_field
                            .reshape(entity, xform.rotation, emitter.cone_angle);
                    }
                }
            }
            EngineCommand::UpdateSignal(entity, signal) => {
                if let Ok(mut query) = self
                    .world
                    .query_one::<(&Transform, &mut SignalEmitter)>(entity)
                {
                    if let Some((xform, emitter)) = query.get() {
                        // Update Signal
                        *emitter = signal;
                        self.signal_field
                            .reshape(entity, xform.rotation, emitter.cone_angle);
                    }
                }
            }
            EngineCommand::SelectEntity(entity) => {
                self.selected_entity = entity;
            }
        }
    }
}
