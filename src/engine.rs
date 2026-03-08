// engine.rs

use crate::components::*;
use crate::field::{Mask, Signal, SignalField};
use bitvec::prelude::*;
use hecs::Entity;

use glam::{Vec2, vec2};
use hecs::World;
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Tween};
use rand::Rng;
use rustc_hash::FxHashMap;
use std::f32::consts::TAU;
use std::time::Duration;
use std::time::Instant;

const FIXED_DT: f32 = 1.0 / 100.0; // run physics exactly 100 times a second

#[repr(usize)]
enum Bit {
    BoundingVolume = 0,
    Occlude,
    Collider,
    Audio,
    Player,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentRenderData {
    pub signal: Signal,
    pub color: [u8; 4],
    pub _label: Option<u8>,
}

#[derive(Default, Clone, Copy)]
pub struct DebugInfo {
    pub tick_counter: u64,
    pub agent_count: usize,
    pub render_time_ms: f32,
    pub tick_time_ms: f32,
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
    SpawnAudio(Vec2),
}

#[derive(Debug, Clone)]
pub struct InspectionData {
    pub entity: hecs::Entity,
    pub xform: Transform,
    pub emitters: Vec<SignalEmitter>,
}

#[derive(Clone)]
pub struct FrameData {
    pub agents: Vec<AgentRenderData>, // the "points" for the GPU
    pub debug_info: DebugInfo,
    //
    pub camera_xform: Transform,
    pub internal_res: Vec2,
    //
    // (read-only for gui, write-only for engine)
    // the engine guarantees this is always populated with the latest reality.
    // the gui just reads this to draw the window.
    pub inspection_view: InspectionData,
    pub inspection_entities: Vec<(Entity, Label)>, // for the hierarchy window
}

impl FrameData {
    pub fn new(width: usize, height: usize) -> Self {
        FrameData {
            inspection_view: InspectionData {
                entity: Entity::DANGLING,
                xform: Transform::default(),
                emitters: Vec::new(),
            },
            inspection_entities: Vec::new(),
            agents: Vec::new(),
            debug_info: DebugInfo::default(),
            camera_xform: Transform::default(),
            internal_res: vec2(width as f32, height as f32),
        }
    }
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
    selected_entity: Entity,
    camera_entity: Entity,
    viewport_size: Vec2,
    //
    _audio_manager: AudioManager<DefaultBackend>,
}

impl Engine {
    pub fn new() -> Self {
        let width = 1024.0;
        let height = 768.0;

        let mut world = World::new();
        let mut signal_field = SignalField::new();

        // kira audio manager
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .expect("Failed to inialize kira audio manager");

        let camera_id = Self::spawn_camera(vec2(0.0, 0.0), &mut world, &mut signal_field);
        Self::spawn_dummy_entities(width, height, &mut world, &mut signal_field);
        Self::spawn_dummy_player(&mut world, &mut signal_field);
        Self::spawn_wolf(vec2(0.0, 0.0), &mut world, &mut signal_field);

        Self {
            world,
            last_update: Instant::now(),
            tick_counter: 0,
            time_accumulator: 0.0,
            last_tick_time_ms: 0.0,
            signal_field,
            camera_entity: camera_id,
            selected_entity: camera_id,
            viewport_size: vec2(width, height),
            _audio_manager: manager,
        }
    }

    pub fn tick(&mut self) {
        // time management
        let start_update = Instant::now();
        let delta_time = start_update.duration_since(self.last_update).as_secs_f32();
        self.last_update = start_update;
        self.time_accumulator += delta_time;

        // spiral of death protection
        if self.time_accumulator > 0.25 {
            println!("too much time between ticks. slowing time down");
            self.time_accumulator = 0.25;
        }

        // fixed timestep loop
        while self.time_accumulator >= FIXED_DT {
            self.tick_once(); // execute one discrete simulation step
            self.time_accumulator -= FIXED_DT;
        }
    }

    // performs exactly one discrete simulation step (1/100th of a second, for example)
    pub fn tick_once(&mut self) {
        let tick_start = Instant::now();

        self.system_chase();
        self.system_fade_audio();
        self.system_physics_collisions();
        self.system_physics_bounce_on_edges();
        self.system_sync_spatial();

        // syncs components
        for (id, (xform, emitter)) in self.world.query_mut::<(&Transform, &SignalEmitter)>() {
            self.signal_field
                .reposition(id, xform.position, emitter.radius_max * xform.scale);
            self.signal_field.reshape(
                id,
                xform.rotation,
                emitter.cone_angle,
                emitter.radius_min * xform.scale,
            );
        }

        // update counters & metrics
        self.tick_counter += 1;
        self.last_tick_time_ms = tick_start.elapsed().as_secs_f32() * 1000.0;
    }

    fn system_physics_bounce_on_edges(&mut self) {
        // determine edges (clipping your movement to the current camera view)
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

        for (_id, (xform, vel, anchor_opt, emitter)) in self.world.query_mut::<(
            &mut Transform,
            &mut Velocity,
            Option<&mut SpatialAnchor>,
            &SignalEmitter,
        )>() {
            match anchor_opt {
                Some(anchor) => {
                    // apply linear velocity
                    anchor.position_offset += vel.linear * FIXED_DT;
                    anchor.rotation_offset += vel.angular * FIXED_DT;
                    anchor.scale_offset += vel.scalar * FIXED_DT;
                }
                _ => {
                    xform.position += vel.linear * FIXED_DT;
                    xform.rotation += vel.angular * FIXED_DT;
                    xform.scale += vel.scalar * FIXED_DT;

                    if !(emitter.emit_mask[Bit::Collider as usize]) {
                        continue;
                    }
                    // bounce off camera edges
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
        for (id, (xform, emitter)) in self
            .world
            .query_mut::<(&mut Transform, &mut SignalEmitter)>()
        {
            if !(emitter.emit_mask[Bit::Collider as usize]) {
                continue;
            }

            let my_radius = emitter.radius_max * xform.scale;
            self.signal_field.scan(id, |signal, key| {
                if !signal.emit_mask[Bit::Collider as usize] {
                    return;
                }

                // self check
                if id == key {
                    return;
                }

                // calculate vector from them -> to me
                let delta = xform.position - signal.origin;
                // We need 'dist' to normalize the vector (direction)
                // and to calculate 'overlap' (strength)
                let dist = delta.length_squared().sqrt();

                let normal = delta / dist.max(0.0001);
                let combined_radius = my_radius + signal.outer_radius;
                let overlap = combined_radius - dist;

                // push
                xform.position += normal * overlap * 0.5;

                // // slide (velocity kill)
                // let impact = vel.linear.dot(normal);
                // if impact < 0.0 {
                //     vel.linear *= normal * impact;
                // }
            });
        }
    }

    fn system_sync_spatial(&self) {
        let mut buffer = Vec::new(); // not bothering with allocations for now

        // pass 1: read & calc
        for (child_id, (anchor, _)) in self.world.query::<(&SpatialAnchor, &Transform)>().iter() {
            if let Ok(parent) = self.world.get::<&Transform>(anchor.parent) {
                buffer.push((
                    child_id,
                    parent.position + anchor.position_offset,
                    parent.rotation + anchor.rotation_offset,
                ));
            }
        }

        // pass 2: write
        for (child_id, pos, rot) in buffer.iter() {
            if let Ok(mut child) = self.world.get::<&mut Transform>(*child_id) {
                child.position = *pos;
                child.rotation = *rot;
            }
        }
    }
    pub fn render(&self, frame: &mut FrameData) {
        let start_render = Instant::now();

        // 1. prepare the query for the specific entity
        let mut query = self
            .world
            .query_one::<&Transform>(self.camera_entity)
            .expect("Entity does not exist");

        // 2. fetch the component references from the query
        let xform = query.get().expect("Entity is missing Transform or Camera");
        frame.internal_res = self.viewport_size;
        frame.camera_xform = *xform;

        // 2. spatial query
        // we only care about agents the camera can actually see
        let mut signal_mask = Mask::default();
        signal_mask.set(Bit::BoundingVolume as usize, true);

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
                        color: [model.r / 4, model.g / 4, model.b / 4, model.a],
                        _label: None,
                    };
                    to_render.insert((*entity, signal.emit_mask), data);
                }
            },
        );

        // self.render_player_vision(&mut to_render);
        self.render_player_vision_occluded(&mut to_render);

        // quick z buffer sorting
        let mut entries: Vec<_> = to_render.into_iter().collect();
        entries.sort_unstable_by_key(|((_, mask), _)| *mask);
        frame
            .agents
            .extend(entries.into_iter().map(|(_, data)| data));

        // metadata
        frame.debug_info.tick_counter = self.tick_counter;
        frame.debug_info.agent_count = self.world.len() as usize;
        frame.debug_info.render_time_ms = start_render.elapsed().as_secs_f32() * 1000.0;
        //
        frame.debug_info.tick_time_ms = self.last_tick_time_ms;
        frame.debug_info.active_levels_mask = self.signal_field.get_level_mask();

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

    #[allow(dead_code)]
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
                                _label: None,
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
        let mut occlusion_mask = BitArray::ZERO;
        occlusion_mask.set(Bit::Occlude as usize, true);
        let mut i = 0;

        // 1. validate that the selected entity exists and has a signalemitter
        if self
            .world
            .get::<&SignalEmitter>(self.selected_entity)
            .is_ok()
        {
            // 2. use the new occluded scan logic.
            // this will sort tiles and signals front-to-back to calculate shadows
            self.signal_field.scan_occluded(
                self.selected_entity,
                // layer_mask,
                occlusion_mask,
                |signal, entity, visible_bits| {
                    // 3. only process if the entity has a model component to render
                    if let Ok(mut query) = self.world.query_one::<&Model>(entity) {
                        // 4. update the render data with detected colors
                        // we pass the 'visible_bits' (shadowmask) into agentrenderdata
                        // so the gui shader/painter knows which arcs to actually draw.
                        if let Some(model) = query.get() {
                            let data = AgentRenderData {
                                signal: *signal,
                                color: [model.r, model.g, model.b, model.a],
                                _label: Some(visible_bits.count_ones() as u8),
                            };

                            i += 1;
                            to_render.insert((entity, signal.emit_mask), data);
                        }
                    }
                },
            );
        }
    }

    fn spawn_camera(position: Vec2, world: &mut World, _signal_field: &mut SignalField) -> Entity {
        let mut level_mask = Mask::default();
        level_mask.fill(true);

        let mut signal_mask = Mask::default();
        signal_mask.fill(true);

        world.spawn((
            Label {
                name: "Main Camera".to_string(),
            },
            Transform {
                position: position,
                rotation: 0.0,
                scale: 1.0,
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

        for i in 0..1000 {
            let rand_pos_x = rng.random_range(-width / 2.0..(width / 2.0));
            let rand_pos_y = rng.random_range(-height / 2.0..(height / 2.0));
            let rand_vel_x = rng.random_range(-100.0..100.0);
            let rand_vel_y = rng.random_range(-100.0..100.0);

            let pos = vec2(rand_pos_x, rand_pos_y);
            let radius = 3.0;
            let direction = Vec2::X;
            let mut scale = 2.0;

            // 1. reserve id
            let id = world.reserve_entity();

            // 2. prepare masks
            let mut signal_mask = Mask::default();
            signal_mask.set(Bit::BoundingVolume as usize, true);
            signal_mask.set(Bit::Collider as usize, true);
            let model;
            let occlude = (i % 300) == 0;
            if occlude {
                signal_mask.set(Bit::Occlude as usize, true);
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
                unit_direction: direction,
                outer_radius: radius * scale,
                inner_radius: 0.0,
                angle_radians: angle,
                emit_mask: signal_mask,
                sense_mask: signal_mask,
            };

            signal_field.emit(signal, id);

            // everything enters the world at the exact same moment
            world.spawn_at(
                id,
                (
                    // Label {
                    //     name: format!("dummy_{}", i),
                    // },
                    Transform {
                        position: pos,
                        scale: scale,
                        rotation: direction.to_angle(),
                    },
                    model,
                    emitter,
                ),
            );

            if !occlude {
                let _ = world.insert_one(
                    id,
                    Velocity {
                        linear: vec2(rand_vel_x, rand_vel_y),
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

    fn spawn_audio(
        pos: Vec2,
        world: &mut World,
        // audio_manager: &mut Option<AudioManager>,
        signal_field: &mut SignalField,
    ) {
        let radius = 1.0;
        let scale = 0.0;
        let direction = Vec2::X;

        let id = world.reserve_entity();

        let model = Model {
            r: 180,
            g: 180,
            b: 0,
            a: 100,
        };

        let mut signal_mask = Mask::default();
        signal_mask.set(Bit::BoundingVolume as usize, true);
        signal_mask.set(Bit::Audio as usize, true);

        let angle = TAU;
        let emitter = SignalEmitter {
            radius_max: radius,
            radius_min: 0.0,
            cone_angle: angle,
            emit_mask: signal_mask,
            sense_mask: signal_mask,
        };

        // let sound_data = StaticSoundData::from_file("assets/raining_loop.flac")
        //     .expect("Failed to load audio file")
        //     .volume(-100.0);
        //
        // // play it
        // let handle = audio_manager
        //     .play(sound_data.clone())
        //     .expect("Failed to play audio");
        //
        // let audio = AudioSourcePersistent {
        //     // sound_data: sound_data,
        //     handle: handle,
        //     base_volume: 10.0,
        // };

        let signal = Signal {
            origin: pos,
            unit_direction: direction,
            outer_radius: radius * scale,
            inner_radius: 0.0,
            angle_radians: angle,
            emit_mask: signal_mask,
            sense_mask: signal_mask,
        };
        signal_field.emit(signal, id);

        world.spawn_at(
            id,
            (
                Label {
                    name: format!("audio_signal"),
                },
                Transform {
                    position: pos,
                    scale: scale,
                    rotation: direction.to_angle(),
                },
                Velocity {
                    linear: vec2(0.0, 0.0),
                    angular: 0.0,
                    scalar: 250.0,
                },
                model,
                emitter,
                Audio,
            ),
        );
    }

    fn spawn_dummy_player(world: &mut World, signal_field: &mut SignalField) {
        let player_pos = vec2(512.0, 384.0);
        let player_scale = 20.0;

        let player_id = world.reserve_entity();

        let mut signal_mask = Mask::default();
        signal_mask.set(Bit::BoundingVolume as usize, true);
        signal_mask.set(Bit::Collider as usize, true);

        let outer_rad = 1.0;
        let inner_rad = 0.0;
        let cone_angle = TAU;
        let direction = Vec2::X;

        signal_mask.set(Bit::Player as usize, true);
        let player_emitter = SignalEmitter {
            radius_max: outer_rad,
            radius_min: inner_rad,
            cone_angle: cone_angle,
            emit_mask: signal_mask,
            sense_mask: signal_mask,
        };

        let player_signal = Signal {
            origin: player_pos,
            unit_direction: direction,
            outer_radius: outer_rad * player_scale,
            inner_radius: inner_rad,
            angle_radians: cone_angle,
            emit_mask: signal_mask,
            sense_mask: signal_mask,
        };

        signal_mask.set(Bit::Player as usize, false);
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
                    rotation: direction.to_angle(),
                },
                Model {
                    r: 000,
                    g: 100,
                    b: 000,
                    a: 255,
                },
                player_emitter,
            ),
        );

        /////////////
        // view cone
        let player_vision_id = world.reserve_entity();
        let scanner_range = 2.0;
        let scale = 30.0;
        let cone_angle = TAU / 8.0;
        signal_mask.set(Bit::Collider as usize, false);

        let child_emitter = SignalEmitter {
            radius_max: scanner_range,
            radius_min: 0.0,
            cone_angle: cone_angle,
            emit_mask: signal_mask,
            sense_mask: signal_mask,
        };

        let child_signal = Signal {
            origin: player_pos,
            unit_direction: direction,
            outer_radius: scanner_range * scale,
            inner_radius: 0.0,
            angle_radians: cone_angle,
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
                    rotation: direction.to_angle(),
                },
                SpatialAnchor {
                    parent: player_id,
                    position_offset: Vec2::ZERO,
                    rotation_offset: 0.0,
                    scale_offset: 0.0,
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
    }

    pub fn handle(&mut self, command: EngineCommand) {
        match command {
            EngineCommand::UpdateViewport(size) => {
                self.viewport_size = size;
            }
            EngineCommand::UpdateTransform(entity, new_transform) => {
                // update the transform (cache)
                if let Ok(mut xform) = self.world.get::<&mut Transform>(entity) {
                    *xform = new_transform;
                }

                // update the anchor
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
                        // update signal
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
            EngineCommand::UpdateSignal(entity, signal) => {
                if let Ok(mut query) = self
                    .world
                    .query_one::<(&Transform, &mut SignalEmitter)>(entity)
                {
                    if let Some((xform, emitter)) = query.get() {
                        // update signal
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
            EngineCommand::SpawnAudio(pos) => {
                Self::spawn_audio(pos, &mut self.world, &mut self.signal_field);
            }
        }
    }

    fn spawn_wolf(
        position: Vec2,
        world: &mut World,
        signal_field: &mut SignalField,
    ) -> (Entity, Entity) {
        let wolf_id = world.reserve_entity();
        let vision_id = world.reserve_entity();

        let scale = 3.0;
        let body_radius = 10.0;
        let mut body_mask = Mask::default();
        body_mask.set(Bit::BoundingVolume as usize, true);
        body_mask.set(Bit::Collider as usize, true);

        let body_emitter = SignalEmitter {
            radius_max: body_radius,
            radius_min: 0.0,
            cone_angle: std::f32::consts::TAU,
            emit_mask: body_mask,
            sense_mask: body_mask,
        };

        let direction = Vec2::X;
        let body_signal = Signal {
            origin: position,
            unit_direction: direction,
            outer_radius: body_radius,
            inner_radius: 0.0,
            angle_radians: std::f32::consts::TAU,
            emit_mask: body_mask,
            sense_mask: body_mask,
        };

        signal_field.emit(body_signal, wolf_id);

        let seeker = Seeker {
            state: SeekerState::Idle,
            target_source: glam::vec2(0.0, 0.0),
            vision_entity: vision_id,
            target: Entity::DANGLING,
        };

        world.spawn_at(
            wolf_id,
            (
                Label {
                    name: "Wolf".to_string(),
                },
                Wolf,
                Transform {
                    position,
                    rotation: direction.to_angle(),
                    scale: scale,
                },
                Velocity::default(),
                Model {
                    r: 10,
                    g: 10,
                    b: 200,
                    a: 255,
                },
                body_emitter,
                seeker,
            ),
        );

        /////////////
        // view cone

        let vision_range = 10.0;
        let vision_scale = 30.0;
        let cone_angle = std::f32::consts::TAU / 8.0;

        let mut vision_mask = Mask::default();
        vision_mask.set(Bit::BoundingVolume as usize, true);

        let vision_emitter = SignalEmitter {
            radius_max: vision_range,
            radius_min: 0.0,
            cone_angle,
            emit_mask: vision_mask,
            sense_mask: vision_mask,
        };

        let vision_signal = Signal {
            origin: position,
            unit_direction: direction,
            outer_radius: vision_range * vision_scale,
            inner_radius: 0.0,
            angle_radians: cone_angle,
            emit_mask: vision_mask,
            sense_mask: vision_mask,
        };

        signal_field.emit(vision_signal, vision_id);

        world.spawn_at(
            vision_id,
            (
                Label {
                    name: "Wolf Vision".to_string(),
                },
                Transform {
                    position,
                    rotation: direction.to_angle(),
                    scale: vision_scale,
                },
                Velocity::default(),
                SpatialAnchor {
                    parent: wolf_id,
                    position_offset: glam::Vec2::ZERO,
                    rotation_offset: 0.0,
                    scale_offset: 0.0,
                },
                Model {
                    r: 200,
                    g: 50,
                    b: 200,
                    a: 80,
                },
                vision_emitter,
            ),
        );

        (wolf_id, vision_id)
    }

    fn system_chase(&mut self) {
        // query all seeker components
        for (wolf, (xform, vel, seeker)) in self
            .world
            .query_mut::<(&mut Transform, &mut Velocity, &mut Seeker)>()
        {
            // everytime the wolf vision cones sees a player it locks it's target to it and starts
            // walking in it's direction.
            let mut player_found = false;

            self.signal_field
                .scan(seeker.vision_entity, |signal, _key| {
                    // early return
                    if player_found {
                        return;
                    };

                    if signal.emit_mask[Bit::Player as usize] {
                        seeker.target_source = signal.origin;
                        seeker.state = SeekerState::Chasing;
                        player_found = true;
                        return;
                    }
                });

            if !player_found {
                self.signal_field.scan(wolf, |signal, key| {
                    // footstep hearing
                    if signal.emit_mask[Bit::Audio as usize] {
                        // there are two cases which may happen
                        // 1. this is the first time audio has been heard, just turn the view cone in
                        //    the source direction, becoming alert
                        // 2. this is the second time audio has been heard, start walking towards the
                        //    source

                        // case 1
                        if seeker.target == Entity::DANGLING && seeker.state != SeekerState::Alert {
                            seeker.state = SeekerState::Alert;
                        }
                        // case 2
                        else if seeker.target != key {
                            seeker.state = SeekerState::Seeking;
                            seeker.target = key;
                        }

                        seeker.target = key;
                        seeker.target_source = signal.origin;
                    }
                });
            }

            let mut rotate_function = || {
                // 1. get the direction vector to the audio source
                let direction = seeker.target_source - xform.position;

                // 2. calculate the desired angle (in radians)
                let target_angle = direction.y.atan2(direction.x);

                // 3. find the shortest angular difference (-pi to pi)
                let mut angle_diff = target_angle - xform.rotation;
                while angle_diff > std::f32::consts::PI {
                    angle_diff -= 2.0 * std::f32::consts::PI;
                }
                while angle_diff < -std::f32::consts::PI {
                    angle_diff += 2.0 * std::f32::consts::PI;
                }

                // 4. apply angular velocity (multiplier controls turn speed)
                let turn_speed = 5.0;
                vel.angular = angle_diff * turn_speed;
            };

            // the wolf behavior is controlled by a state machine
            match seeker.state {
                SeekerState::Idle => {
                    // stay still
                    vel.linear = vec2(0.0, 0.0);
                    vel.angular = 0.0;
                    seeker.target = Entity::DANGLING; // reset target when idle
                    println!("Idle -- {:?}", vel.linear);
                }
                SeekerState::Alert => {
                    vel.linear = vec2(0.0, 0.0);
                    rotate_function();
                    println!("Alert -- {:?}", vel.angular);
                }
                SeekerState::Seeking => {
                    let distance = seeker.target_source - xform.position;
                    vel.linear = (distance).normalize_or_zero() * 100.0;
                    rotate_function();

                    if distance.length() < 10.0 {
                        seeker.state = SeekerState::Idle;
                    }

                    if player_found {
                        seeker.state = SeekerState::Chasing;
                    }

                    println!("Seeeking -- {:?}", vel.angular);
                }
                SeekerState::Chasing => {
                    vel.linear =
                        (seeker.target_source - xform.position).normalize_or_zero() * 100.0;

                    rotate_function();

                    if !player_found {
                        seeker.state = SeekerState::Idle;
                    }

                    println!("Chasing -- {:?}", vel.linear);
                }
            }
        }
    }

    fn system_fade_audio(&mut self) {
        let mut to_despawn = Vec::new();
        let max_scale = 800.0;

        for (key, (_audio, xform, model)) in
            self.world.query_mut::<(&Audio, &Transform, &mut Model)>()
        {
            // calculate progress (0.0 at start, 1.0 at max_scale)
            let progress = xform.scale / max_scale;

            // map progress to alpha (255 down to 0)
            let alpha = (1.0 - progress) * 255.0;

            // apply and clamp
            model.a = alpha.clamp(0.0, 255.0) as u8;

            // despawn logic
            if xform.scale >= max_scale || model.a == 0 {
                self.signal_field.cease(key);
                to_despawn.push(key);
            }
        }

        for key in to_despawn {
            let _ = self.world.despawn(key);
        }
    }

    //////////////////////////

    fn _experimental_system_audio_render(&mut self) {
        let mut query = self.world.query::<(&mut AudioListener, &Transform)>();
        let (listener_xform, last_active_sources, current_active_sources) = {
            match query.iter().next() {
                Some((_entity, (listener, xform))) => {
                    let last_active_sources = std::mem::take(&mut listener.last_active_sources);
                    let current_active_sources = &mut listener.last_active_sources;

                    (xform, last_active_sources, current_active_sources)
                }
                _ => return,
            }
        };

        // scan all audio sources that intersect our listener position
        let mut mask = Mask::ZERO;
        mask.set(Bit::Audio as usize, true);

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
                        // println!(
                        //     "(distance / audio_max_radius) -> {} / {} => {}",
                        //     distance,
                        //     audio_max_radius,
                        //     distance / audio_max_radius
                        // );

                        // Panning (Dot Product)
                        let dir =
                            (source_xform.position - listener_xform.position).normalize_or_zero();
                        // calculates the right vector of the listener
                        let (sin, cos) = listener_xform.rotation.sin_cos();
                        let right = vec2(sin, -cos); // Forward = (cos, sin)
                        // finally, calculate the pan value
                        let pan = right.dot(dir);

                        // (Already playing)
                        let _ = audio_source.handle.set_volume(volume, tween);
                        let _ = audio_source.handle.set_panning(pan, tween);
                        // println!("{:?}: volume {:.2}", audio_entity, volume);
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
}
