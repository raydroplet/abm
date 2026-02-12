// engine.rs

use crate::components::*;
use crate::wave::{Mask, Signal, SignalField};
use bitvec::prelude::*;
use hecs::Entity;

use glam::Vec2;
use hecs::World;
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Tween};
use rand::Rng;
use rustc_hash::FxHashMap;
use std::f32::consts::TAU;
use std::time::Duration;
use std::time::Instant; // or f64::consts::TAU

const FIXED_DT: f32 = 1.0 / 100.0; // Run physics exactly 100 times a second
const BIT_AUDIO: usize = 3;

const BIT_BOUNDING_VOLUME: usize = 0;
const BIT_OCCLUDE: usize = 1;
const BIT_COLLIDER: usize = 2;

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
    pub active_levels_mask: Mask,
}

#[derive(Copy, Clone)]
pub enum EngineCommand {
    UpdateViewport(Vec2),
    //
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
    pub camera_xform: Transform,
    pub internal_res: Vec2,
    //
    // (Read-Only for GUI, Write-Only for Engine)
    // The Engine guarantees this is always populated with the latest reality.
    // The GUI just reads this to draw the window.
    pub inspection_view: InspectionData,
    pub inspection_entities: Vec<(Entity, Label)>, // for the hierarchy window
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
    //
    selected_entity: Entity,
    camera_entity: Entity,
    viewport_size: Vec2,
    //
    audio_manager: AudioManager<DefaultBackend>,
}

impl Engine {
    pub fn new() -> Self {
        let mut world = World::new();
        let mut signal_field = SignalField::new(); // User variable name
        let width = 1024.0;
        let height = 768.0;

        // kira audio manager
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .expect("Failed to inialize kira audio manager");

        let camera_id = Self::spawn_camera(width, height, &mut world, &mut signal_field);
        Self::spawn_dummy_entities(width, height, &mut world, &mut signal_field);
        Self::spawn_audio_sources(width, height, &mut world, &mut manager, &mut signal_field);
        let (player_id, player_vision_id) = Self::spawn_dummy_player(&mut world, &mut signal_field);

        Self {
            world,
            last_update: Instant::now(),
            tick_counter: 0,
            time_accumulator: 0.0,
            last_tick_time_ms: 0.0,
            signal_field, // Store the layer
            camera_entity: camera_id,
            selected_entity: player_id,
            viewport_size: Vec2::new(width, height),
            audio_manager: manager,
        }
    }

    pub fn tick(&mut self) {
        // 1. Time Management
        let start_update = Instant::now();
        let delta_time = start_update.duration_since(self.last_update).as_secs_f32();
        self.last_update = start_update;
        self.time_accumulator += delta_time;

        // 2. Spiral of Death Protection
        if self.time_accumulator > 0.25 {
            println!("too much time between ticks. slowing time down");
            self.time_accumulator = 0.25;
        }

        // 3. Fixed Timestep Loop
        while self.time_accumulator >= FIXED_DT {
            self.tick_once(); // Execute one discrete simulation step
            self.time_accumulator -= FIXED_DT;
        }
    }

    /// Performs exactly one discrete simulation step (1/100th of a second).
    pub fn tick_once(&mut self) {
        let tick_start = Instant::now();

        self.system_physics_collisions();
        self.system_physics_bounce_on_edges();
        self.system_sync_spatial();
        self.system_audio_render();

        // Updates the Universal Field Engine with new physical positions
        for (id, (xform, emitter)) in self.world.query_mut::<(&Transform, &SignalEmitter)>() {
            self.signal_field
                .reposition(id, xform.position, emitter.radius_max * xform.scale);
        }

        // 5. Update Counters & Metrics
        self.tick_counter += 1;
        self.last_tick_time_ms = tick_start.elapsed().as_secs_f32() * 1000.0;
    }

    /// Extracted movement logic to be called within tick_once
    fn system_physics_bounce_on_edges(&mut self) {
        // Determine edges (clipping your movement to the current camera view)
        let (left, right, top, bottom) = {
            let query = self.world.query_one::<&Transform>(self.camera_entity).ok();
            if let Some(mut q) = query {
                if let Some(xform) = q.get() {
                    let aspect = self.viewport_size.x / self.viewport_size.y;
                    let h = self.viewport_size.y * xform.scale;
                    let w = h * aspect;
                    (
                        xform.position.x - w / 2.0,
                        xform.position.x + w / 2.0,
                        xform.position.y - h / 2.0,
                        xform.position.y + h / 2.0,
                    )
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                }
            } else {
                (0.0, 0.0, 0.0, 0.0)
            }
        };

        for (_id, (xform, vel, anchor_opt)) in
            self.world
                .query_mut::<(&mut Transform, &mut Velocity, Option<&mut SpatialAnchor>)>()
        {
            match anchor_opt {
                Some(anchor) => {
                    anchor.position_offset += vel.linear * FIXED_DT;
                }
                None => {
                    xform.position += vel.linear * FIXED_DT;

                    // Bounce off camera edges
                    if xform.position.x >= right {
                        vel.linear.x = -vel.linear.x.abs();
                        xform.position.x = right;
                    }
                    if xform.position.x <= left {
                        vel.linear.x = vel.linear.x.abs();
                        xform.position.x = left;
                    }
                    if xform.position.y >= bottom {
                        vel.linear.y = -vel.linear.y.abs();
                        xform.position.y = bottom;
                    }
                    if xform.position.y <= top {
                        vel.linear.y = vel.linear.y.abs();
                        xform.position.y = top;
                    }
                }
            }
        }
    }

    fn system_physics_collisions(&mut self) {
        for (id, (xform, vel, emitter)) in
            self.world
                .query_mut::<(&mut Transform, &mut Velocity, &mut SignalEmitter)>()
        {
            let my_radius = emitter.radius_max * xform.scale;
            self.signal_field.scan(id, |signal, key| {
                if signal.emit_mask[BIT_COLLIDER] {
                    // self check
                    if id == key {
                        return;
                    }

                    // A. Calculate Vector from Them -> To Me
                    let delta = xform.position - signal.origin;
                    // We need 'dist' to normalize the vector (direction)
                    // We need 'dist' to calculate 'overlap' (strength)
                    let dist = delta.length_squared().sqrt();

                    //
                    let normal = delta / dist.max(0.0001);
                    let combined_radius = my_radius + signal.outer_radius;
                    let overlap = combined_radius - dist;

                    // Push
                    xform.position += normal * overlap * 0.5;

                    // // Slide (Velocity Kill)
                    // let impact = vel.linear.dot(normal);
                    // if impact < 0.0 {
                    //     vel.linear *= normal * impact;
                    // }
                }
            });
        }
    }

    fn system_sync_spatial(&self) {
        let mut buffer = Vec::new(); // not bothering with allocations for now

        // Pass 1: Read & Calc
        for (child_id, (anchor, _)) in self.world.query::<(&SpatialAnchor, &Transform)>().iter() {
            if let Ok(parent) = self.world.get::<&Transform>(anchor.parent) {
                buffer.push((child_id, parent.position + anchor.position_offset));
            }
        }

        // Pass 2: Write
        for (child_id, pos) in buffer.iter() {
            if let Ok(mut child) = self.world.get::<&mut Transform>(*child_id) {
                child.position = *pos;
            }
        }
    }

    fn system_audio_render(&mut self) {
        //
        let mut query = self.world.query::<(&mut AudioListener, &Transform)>();
        let (listener_xform, last_active_sources, current_active_sources) = {
            match query.iter().next() {
                Some((_entity, (listener, xform))) => {
                    let last_active_sources = std::mem::take(&mut listener.last_active_sources);
                    let current_active_sources = &mut listener.last_active_sources;

                    (xform, last_active_sources, current_active_sources)
                }
                None => return,
            }
        };

        // Scan all audio sources that intersect our listener position
        let mut mask = Mask::ZERO;
        mask.set(BIT_AUDIO, true);

        let tween = Tween {
            duration: Duration::from_millis(30), // ~2 frames at 60fps
            ..Default::default()
        };

        self.signal_field.scan_point(
            listener_xform.position,
            mask,
            |audio_signal, audio_entity| {
                // query the audio source component
                if let Ok(mut query) =
                    self.world
                        .query_one::<(&Transform, &SignalEmitter, &mut AudioSourcePersistent)>(
                            *audio_entity,
                        )
                {
                    if let Some((source_xform, _, audio_source)) = query.get() {
                        // A. Mark this entity as "Found"
                        current_active_sources.push(*audio_entity);

                        // B. Math: Calculate Volume & Pan
                        let audio_max_radius = audio_signal.outer_radius;
                        let distance = listener_xform.position.distance(source_xform.position);
                        let attenuation = distance / audio_max_radius;
                        let volume = audio_source.base_volume + (attenuation * -30.0);
                        println!(
                            "(distance / audio_max_radius) -> {} / {} => {}",
                            distance,
                            audio_max_radius,
                            distance / audio_max_radius
                        );

                        // Panning (Dot Product)
                        let dir =
                            (source_xform.position - listener_xform.position).normalize_or_zero();
                        // calculates the right vector of the listener
                        let (sin, cos) = listener_xform.rotation.sin_cos();
                        let right = Vec2::new(sin, -cos); // Forward = (cos, sin)
                        // finally, calculate the pan value
                        let pan = right.dot(dir);

                        // (Already playing)
                        let _ = audio_source.handle.set_volume(volume, tween);
                        let _ = audio_source.handle.set_panning(pan, tween);
                        println!("{:?}: volume {:.2}", audio_entity, volume);
                        //
                    }
                }
            },
        );

        // disable sounds that are not in range anymore
        // this is O(N * M), but supposedly faster than a hashmap for few entities (~ <30)
        for entity in last_active_sources
            .iter()
            .filter(|e| !current_active_sources.contains(e))
        {
            // query audio source component
            if let Ok(mut query) = self.world.query_one::<&mut AudioSourcePersistent>(*entity) {
                if let Some(audio_source) = query.get() {
                    // stop the audio
                    let _ = audio_source.handle.set_volume(-100.0, tween);
                    println!("{:?}: volume {:.2}", *entity, -100.0);
                    // let _ = handle.stop(Tween::default());
                    // Handle is dropped here, freeing the voice resource.
                }
            }
        }
    }

    pub fn render(&self, frame: &mut FrameData) {
        let start_render = Instant::now();

        // 1. Prepare the query for the specific entity
        let mut query = self
            .world
            .query_one::<&Transform>(self.camera_entity)
            .expect("Entity does not exist");

        // 2. Fetch the component references from the query
        let xform = query.get().expect("Entity is missing Transform or Camera");
        frame.internal_res = self.viewport_size;
        frame.camera_xform = *xform;

        // 2. Spatial Query
        // We only care about agents the camera can actually see
        let mut signal_mask = Mask::default();
        // let mut layer_mask = Mask::default();
        signal_mask.set(BIT_BOUNDING_VOLUME, true);
        // layer_mask.fill(true);

        frame.agents.clear();
        let mut to_render: FxHashMap<(hecs::Entity, Mask), AgentRenderData> = FxHashMap::default();

        // ghost entities
        self.signal_field.scan_range(
            xform.position - ((self.viewport_size / 2.0) * xform.scale),
            xform.position + ((self.viewport_size / 2.0) * xform.scale),
            signal_mask,
            // layer_mask,
            |signal, entity| {
                if let Ok(model) = self.world.get::<&Model>(*entity) {
                    let data = AgentRenderData {
                        signal: *signal,
                        // color: [50, 50, 70, 40],
                        color: [model.r / 4, model.g / 4, model.b / 4, 255],
                        label: None,
                    };
                    to_render.insert((*entity, signal.emit_mask), data);
                }
            },
        );
        // self.render_player_vision(/* layer_mask,  */&mut to_render);
        self.render_player_vision_occluded(/* layer_mask, */ &mut to_render);

        // quick z buffer sorting hack
        // 1. Collect everything into a temporary Vec so we can access the Mask (in the key)
        // entries type is Vec<((Entity, Mask), AgentRenderData)>
        let mut entries: Vec<_> = to_render.into_iter().collect();

        // 2. Sort: Audio masks (true) come before non-audio (false)
        entries.sort_unstable_by(|((_, mask_a), _), ((_, mask_b), _)| {
            // Check if the 3rd bit is set (assuming 1 << 3)
            let a_has_audio = mask_a[BIT_AUDIO];
            let b_has_audio = mask_b[BIT_AUDIO];

            // Use b.cmp(a) so 'true' sorts before 'false'
            b_has_audio.cmp(&a_has_audio)
        });

        // 3. Drop the keys (Entity, Mask) and extend your frame agents
        frame
            .agents
            .extend(entries.into_iter().map(|(_, data)| data));

        // frame.agents.extend(to_render.into_values());

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
                .query_one::<(&Transform, Option<&SignalEmitter>)>(self.selected_entity)
            {
                if let Some((transform, emitter)) = query.get() {
                    view.xform = *transform;
                    if let Some(emitter) = emitter {
                        view.emitters.push(*emitter);
                    }
                }
            }
        }
        frame.inspection_entities.clear();
        for (entity, label) in self.world.query::<&Label>().iter() {
            frame.inspection_entities.push((entity, label.clone()));
        }
    }

    fn render_player_vision(
        &self,
        // layer_mask: Mask,
        to_render: &mut FxHashMap<hecs::Entity, AgentRenderData>,
    ) {
        // for the selected signal, display signal interactions
        if self
            .world
            .get::<&SignalEmitter>(self.selected_entity)
            .is_ok()
        {
            self.signal_field.scan(
                self.selected_entity,
                /* layer_mask, */
                |signal, entity| {
                    if let Ok(mut query) = self.world.query_one::<&Model>(entity) {
                        if let Some(model) = query.get() {
                            let data = AgentRenderData {
                                signal: *signal,
                                color: [model.r, model.g, model.b, model.a],
                                label: None,
                            };
                            to_render.insert(entity, data);
                        }
                    }
                },
            );
        }
    }

    fn render_player_vision_occluded(
        &self,
        // layer_mask: Mask,
        to_render: &mut FxHashMap<(hecs::Entity, Mask), AgentRenderData>,
    ) {
        // 1. Validate that the selected entity exists and has a SignalEmitter

        let mut occlusion_mask = BitArray::ZERO;
        occlusion_mask.set(BIT_OCCLUDE, true);
        let mut i = 0;
        if self
            .world
            .get::<&SignalEmitter>(self.selected_entity)
            .is_ok()
        {
            // 2. Use the new occluded scan logic
            // This will sort tiles and signals front-to-back to calculate shadows
            self.signal_field.scan_occluded(
                self.selected_entity,
                // layer_mask,
                occlusion_mask,
                |signal, entity, visible_bits| {
                    // 3. Only process if the entity has a Model component to render
                    if let Ok(mut query) = self.world.query_one::<&Model>(entity) {
                        // 4. Update the render data with detected colors
                        // We pass the 'visible_bits' (ShadowMask) into AgentRenderData
                        // so the GUI shader/painter knows which arcs to actually draw.
                        if let Some(model) = query.get() {
                            let data = AgentRenderData {
                                signal: *signal,
                                color: [model.r, model.g, model.b, model.a],
                                label: Some(visible_bits.count_ones() as u8),
                            };

                            i += 1;
                            to_render.insert((entity, signal.emit_mask), data);
                        }
                    }
                },
            );
            // println!("count {}", i);
        }
    }

    ///////////////////
    /// Spawns the main viewport camera into the ECS world.
    fn spawn_camera(
        width: f32,
        height: f32,
        world: &mut World,
        _signal_field: &mut SignalField,
    ) -> Entity {
        // 1. Determine starting position (centered in the world)
        let center = Vec2::new(width * 0.5, height * 0.5);

        // 2. Prepare Viewport Masks
        // By default, the camera should be able to "see" everything.
        let mut level_mask = Mask::default();
        level_mask.fill(true);

        let mut signal_mask = Mask::default();
        signal_mask.fill(true);

        // 3. Spawn the Camera Entity
        world.spawn((
            Label {
                name: "Main Camera".to_string(),
            },
            Transform {
                position: center,
                rotation: 0.0,
                scale: 0.5, // Camera scale usually represents a zoom multiplier
            },
            Camera {
                // level_mask,
                // signal_mask,
                // zoom: 1.0,
            },
        ))
    }

    fn spawn_dummy_entities(
        width: f32,
        height: f32,
        world: &mut World,
        signal_field: &mut SignalField,
    ) {
        let mut rng = rand::rng();

        for i in 0..1 {
            // Random Data
            let rand_pos_x = rng.random_range(width / 4.0..(width / 4.0) + (width / 2.0));
            let rand_pos_y = rng.random_range(height / 4.0..(height / 4.0) + (height / 2.0));
            let rand_vel_x = rng.random_range(-100.0..100.0);
            let rand_vel_y = rng.random_range(-100.0..100.0);
            // let r_val = rng.random_range(0..=255);
            // let g_val = rng.random_range(0..=255);
            // let b_val = rng.random_range(0..=255);
            // let mut color = 200;

            let pos = Vec2::new(rand_pos_x, rand_pos_y);
            let radius = 3.0;
            let mut scale = 2.0;

            // 1. RESERVE ID (Critical for correct linking)
            let id = world.reserve_entity();

            // 2. Prepare Masks
            let mut signal_mask = Mask::default();
            signal_mask.set(BIT_BOUNDING_VOLUME, true);
            signal_mask.set(BIT_COLLIDER, true);
            let model;
            let occlude = (i % 300) == 0;
            if occlude {
                signal_mask.set(BIT_OCCLUDE, true);
                model = Model {
                    r: 150,
                    g: 20,
                    b: 20,
                    a: 255,
                };
                scale = scale * 3.0;
            } else {
                model = Model {
                    r: 20,
                    g: 150,
                    b: 20,
                    a: 255,
                };
            }
            let layer_mask = Mask::default();

            // 3. Create Emitter (Sphere Factory)
            let angle = TAU;
            let emitter = SignalEmitter {
                radius_max: radius,
                radius_min: 0.0,
                cone_angle: angle,
                emit_mask: signal_mask,
                sense_mask: signal_mask,
            };

            let signal = Signal {
                origin: pos,
                unit_direction: Vec2::new(0.0, 0.0), // Rotation (irrelevant for sphere)
                outer_radius: radius,                // Outer Radius
                inner_radius: 0.0,
                angle_radians: angle, // Cone Angle: 2*PI (Full Sphere)
                emit_mask: signal_mask,
                sense_mask: signal_mask,
            };

            signal_field.emit(signal, id);

            // 4. ATOMIC SPAWN
            // Everything enters the world at the exact same moment
            world.spawn_at(
                id,
                (
                    // Label {
                    //     name: format!("dummy_{}", i),
                    // },
                    Transform {
                        position: pos,
                        scale: scale,
                        ..Transform::default()
                    },
                    model,
                    emitter, // <--- Attached immediately
                ),
            );

            if !occlude {
                let _ = world.insert_one(
                    id,
                    Velocity {
                        linear: Vec2::new(rand_vel_x, rand_vel_y),
                        ..Velocity::default()
                    },
                );
            } else {
                let _ = world.insert_one(
                    id,
                    Label {
                        name: format!("occluder_{}", i),
                    },
                );
            }
        }
    }

    fn spawn_audio_sources(
        width: f32,
        height: f32,
        world: &mut World,
        audio_manager: &mut AudioManager,
        signal_field: &mut SignalField,
    ) {
        let mut rng = rand::rng();

        for i in 0..1 {
            let rand_pos_x = rng.random_range(width / 4.0..(width / 4.0) + (width / 2.0));
            let rand_pos_y = rng.random_range(height / 4.0..(height / 4.0) + (height / 2.0));

            let pos = Vec2::new(rand_pos_x, rand_pos_y);
            let radius = 2.0;
            let scale = 60.0;

            let id = world.reserve_entity();

            let model = Model {
                r: 180,
                g: 180,
                b: 0,
                a: 100,
            };

            let mut signal_mask = Mask::default();
            signal_mask.set(BIT_BOUNDING_VOLUME, true);
            signal_mask.set(BIT_AUDIO, true);

            let angle = TAU;
            let emitter = SignalEmitter {
                radius_max: radius,
                radius_min: 0.0,
                cone_angle: angle,
                emit_mask: signal_mask,
                sense_mask: signal_mask,
            };

            let sound_data = StaticSoundData::from_file("assets/raining_loop.flac")
                .expect("Failed to load audio file")
                .volume(-100.0);

            // 3. Play it
            let handle = audio_manager
                .play(sound_data.clone())
                .expect("Failed to play audio");

            let audio = AudioSourcePersistent {
                // sound_data: sound_data,
                handle: handle,
                base_volume: 10.0,
            };

            let signal = Signal {
                origin: pos,
                unit_direction: Vec2::new(0.0, 0.0), // Rotation (irrelevant for sphere)
                outer_radius: radius,                // Outer Radius
                inner_radius: 0.0,
                angle_radians: angle, // Cone Angle: 2*PI (Full Sphere)
                emit_mask: signal_mask,
                sense_mask: signal_mask,
            };
            signal_field.emit(signal, id);

            world.spawn_at(
                id,
                (
                    Label {
                        name: format!("audio_{}", i),
                    },
                    Transform {
                        position: pos,
                        scale: scale,
                        ..Transform::default()
                    },
                    model,
                    emitter,
                    audio,
                ),
            );
        }
    }

    fn spawn_dummy_player(world: &mut World, signal_field: &mut SignalField) -> (Entity, Entity) {
        let player_pos = Vec2::new(512.0, 384.0);
        let player_scale = 20.0;

        // =========================================================
        // 1. THE PLAYER (Ring Shape)
        // =========================================================
        let player_id = world.reserve_entity();

        let mut signal_mask = Mask::default();
        let layer_mask = Mask::default();
        signal_mask.set(BIT_BOUNDING_VOLUME, true);
        signal_mask.set(BIT_COLLIDER, true);

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
        };

        let player_signal = Signal {
            origin: player_pos,
            unit_direction: Vec2::new(1.0, 0.0), // Default 0.0 rotation (Right)
            outer_radius: outer_rad,
            inner_radius: inner_rad,
            angle_radians: cone_angle,
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
                    rotation: TAU / 4.0,
                },
                // Velocity {
                //     linear: Vec2::new(100.0, 100.0),
                //     ..Velocity::default()
                // },
                Model {
                    r: 000,
                    g: 100,
                    b: 000,
                    a: 255,
                },
                player_emitter,
                AudioListener {
                    last_active_sources: Vec::new(),
                },
            ),
        );

        // =========================================================
        // 2. THE CHILD SCANNER (Omni Sensor)
        // =========================================================
        let player_vision_id = world.reserve_entity();
        let scanner_range = 2.0;
        let scale = 30.0;
        let cone_angle = TAU / 8.0;
        signal_mask.set(BIT_COLLIDER, false);

        // Component
        let child_emitter = SignalEmitter {
            radius_max: scanner_range, // Large range
            radius_min: 0.0,
            cone_angle: cone_angle,
            emit_mask: signal_mask,
            sense_mask: signal_mask,
        };

        let child_signal = Signal {
            origin: player_pos, // Starts at parent position
            unit_direction: Vec2::X,
            outer_radius: scanner_range,
            inner_radius: 0.0,
            angle_radians: cone_angle, // cos(PI) for full sphere
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
                    r: 150,
                    g: 150,
                    b: 150,
                    a: 100,
                },
                child_emitter,
            ),
        );

        (player_id, player_vision_id)
    }

    pub fn handle(&mut self, command: EngineCommand) {
        match command {
            EngineCommand::UpdateViewport(size) => {
                self.viewport_size = size;
            }
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
                            emitter.radius_max * xform.scale,
                        );
                        self.signal_field.reshape(
                            entity,
                            xform.rotation,
                            emitter.cone_angle,
                            emitter.radius_min * xform.scale,
                        );
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
                        self.signal_field.reposition(
                            entity,
                            xform.position,
                            emitter.radius_max * xform.scale,
                        );
                        self.signal_field.reshape(
                            entity,
                            xform.rotation,
                            emitter.cone_angle,
                            emitter.radius_min * xform.scale,
                        );
                    }
                }
            }
            EngineCommand::SelectEntity(entity) => {
                self.selected_entity = entity;
            }
        }
    }
}
