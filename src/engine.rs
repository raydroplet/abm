// engine.rs

use crate::components::*;
use crate::wave::{LevelMask, Signal, SignalField, SignalMask};

use glam::Vec2;
use hecs::World;
use rand::Rng;
use std::time::Instant;

const FIXED_DT: f32 = 1.0 / 100.0; // Run physics exactly 100 times a second
const BIT_BOUNDING_VOLUME: usize = 0;

#[derive(Clone, Copy, Debug)]
pub struct AgentPoint {
    pub position: Vec2,
    pub radius: f32,
    pub color: [u8; 4],
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

pub struct FrameData {
    pub agents: Vec<AgentPoint>, // The "Points" for the GPU
    pub signals: Vec<Signal>,    // Keep these for UI/Overlay logic
    pub debug_info: DebugInfo,
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
}

impl Engine {
    pub fn new() -> Self {
        let mut world = World::new();
        let mut signal_field = SignalField::new(); // User variable name
        let width = 1024.0;
        let height = 768.0;

        Self::spawn_dummy_entities(width, height, &mut world, &mut signal_field);
        Self::spawn_dummy_player(&mut world, &mut signal_field);

        Self {
            world,
            last_update: Instant::now(),
            tick_counter: 0,
            time_accumulator: 0.0,
            last_tick_time_ms: 0.0,
            signal_field, // Store the layer
            camera_dimension: Vec2::new(width, height),
            camera_position: Vec2::new(0.0, 0.0),
        }
    }

    pub fn tick(&mut self) {
        // time management
        let start_update = Instant::now();
        let delta_time = start_update.duration_since(self.last_update).as_secs_f32();
        self.last_update = start_update;
        //
        self.time_accumulator += delta_time;

        // SPIRAL OF DEATH PROTECTION:
        // If the game lags hard (0.25s freeze), don't try to catch up
        // by running 15 physics steps instantly. Just cap it.
        //
        // the logic inside the while loop is currently too fast for this to happen,
        // but it will stay here in case of future needs
        if self.time_accumulator > 0.25 {
            self.time_accumulator = 0.25;
        }

        // fixed update loop
        // We only update physics in chunks of FIXED_DT (e.g., 0.0166s)
        while self.time_accumulator >= FIXED_DT {
            let tick_time = Instant::now();

            // 1. MOVEMENT LOOP (Updates Position)
            for (_id, (xform, vel)) in self.world.query_mut::<(&mut Transform, &mut Velocity)>() {
                xform.position.x += vel.linear.x * FIXED_DT;
                xform.position.y += vel.linear.y * FIXED_DT;

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

            // 2. SIGNAL SYNC LOOP (Updates SignalLayer)
            // We iterate over entities that have BOTH Position and Emitter
            for (_id, (xform, emitter)) in self.world.query_mut::<(&Transform, &SignalEmitter)>() {
                // Reposition the signal to match the agent
                // We split the borrow here: 'pos' is from 'world', 'reposition' is on 'signal_layer'
                self.signal_field.reposition(
                    emitter.key,
                    xform.position,
                    xform.scale, // Keep the radius constant (or pulse it here!)
                );
            }

            self.time_accumulator -= FIXED_DT;
            self.tick_counter += 1;

            self.last_tick_time_ms = tick_time.elapsed().as_secs_f32() * 1000.0;
        }
    }

    pub fn render(&self, frame: &mut FrameData) {
        let start_render = Instant::now();

        // 1. Prepare for new frame
        frame.agents.clear();
        frame.signals.clear();

        let mut signal_mask = SignalMask::default();
        signal_mask.set(BIT_BOUNDING_VOLUME, true);
        // signal_mask.set(BIT_PASSIVE, true);

        let mut layer_mask = SignalMask::default();
        layer_mask.fill(true);

        // let mut filter = SignalMask::<1>::default();
        // filter.set(BIT_RENDER, true);

        // 2. Spatial Query
        // We only care about agents the camera can actually see
        self.signal_field.scan_volume_rectangle(
            self.camera_position,
            self.camera_dimension + self.camera_position,
            signal_mask,
            layer_mask,
            |signal| {
                // A. Store signal for the GUI/Debug overlays
                // if signal.mask != filter {
                frame.signals.push(signal.clone());
                // }

                // B. Fetch visual data from ECS for the GPU
                if let Ok(mut query) = self.world.query_one::<(&Transform, &Model)>(signal.entity) {
                    if let Some((xform, model)) = query.get() {
                        frame.agents.push(AgentPoint {
                            position: xform.position,
                            radius: xform.scale,
                            color: [model.r, model.g, model.b, 255],
                        });
                    }
                }
            },
        );

        // 3. Metadata
        frame.debug_info.tick_counter = self.tick_counter;
        frame.debug_info.agent_count = self.world.len() as usize;
        frame.debug_info.render_time_ms = start_render.elapsed().as_secs_f32() * 1000.0;
        //
        frame.debug_info.tick_time_ms = self.last_tick_time_ms;
        frame.debug_info.active_levels_mask = self.signal_field.get_level_mask();
    }

    ///////////////////

    fn spawn_dummy_entities(
        width: f32,
        height: f32,
        world: &mut World,
        signal_field: &mut SignalField,
    ) {
        let mut rng = rand::rng();
        for _ in 0..3000 {
            // ... (Your existing random generation code) ...
            let rand_pos_x = rng.random_range(0.0..width);
            let rand_pos_y = rng.random_range(0.0..height);
            let rand_vel_x = rng.random_range(-100.0..100.0);
            let rand_vel_y = rng.random_range(-100.0..100.0);
            let r_val = rng.random_range(0..=255);
            let g_val = rng.random_range(0..=255);
            let b_val = rng.random_range(0..=255);

            // A. Spawn the Entity first
            let id = world.spawn((
                Transform {
                    position: Vec2::new(rand_pos_x, rand_pos_y),
                    // rotation: Vec2::default(),
                    scale: 3.0,
                },
                Velocity {
                    linear: Vec2::new(rand_vel_x, rand_vel_y),
                    // angular: 0.0,
                },
                Model {
                    r: r_val,
                    g: g_val,
                    b: b_val,
                },
            ));

            let mut mask = SignalMask::default();
            mask.set(BIT_BOUNDING_VOLUME, true);

            let origin = Vec2::new(rand_pos_x, rand_pos_y);
            let radius = 10.0;
            let sig_key = signal_field.emit(Signal::new_sphere(origin, radius, mask, id));

            // C. Link the SignalKey to the Entity so they stay in sync during movement
            world.insert_one(id, SignalEmitter::new(sig_key)).unwrap();
        }
    }

    fn spawn_dummy_player(world: &mut World, signal_field: &mut SignalField) {
        let hero_pos = Vec2::new(512.0, 384.0);
        let scale = 15.0;
        let hero_id = world.spawn((
            Transform {
                position: Vec2::new(hero_pos[0], hero_pos[1]),
                scale: scale,
                ..Transform::default()
            },
            Velocity {
                linear: Vec2::new(200.0, 200.0),
                ..Velocity::default()
            },
            Model { r: 255, g: 0, b: 0 },
            // We leave SignalEmitter out for a split second
        ));

        let mut mask = SignalMask::default();
        mask.set(BIT_BOUNDING_VOLUME, true); //
        // mask.set(BIT_PASSIVE, true); //
        //
        let sig_key = signal_field.emit(Signal::new_sphere(hero_pos, scale, mask, hero_id));

        world
            .insert_one(hero_id, SignalEmitter::new(sig_key))
            .unwrap();
    }
}
