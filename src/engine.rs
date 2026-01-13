// engine.rs

use crate::wave::{Signal, SignalField, SignalKey, SignalMask};

use hecs::World;
use rand::Rng;
use std::ops::{Deref, DerefMut};
// use std::thread;
use glam::Vec2;
use std::time::Instant;

const FIXED_DT: f32 = 1.0 / 100.0; // Run physics exactly 60 times a second
//
pub const BIT_RENDER: usize = 0;
pub const BIT_PASSIVE: usize = 7;
//
pub struct Engine {
    dummy_background_value: f32,
    world: World,         // entity component system
    last_update: Instant, // used for delta_time calculation
    //
    tick_counter: u64,     // overflows in ~584,000 years at 1.000.000hz
    time_accumulator: f32, //
    last_tick_time_ms: f32, //
    //
    signal_layer: SignalField,
    //
    camera_dimension: Vec2,
    camera_position: Vec2,
    //
}

#[derive(Default, Clone, Copy)]
pub struct DebugInfo {
    pub tick_counter: u64,   // overflows in ~584,000 years at 1.000.000hz
    pub agent_count: usize,  // Useful since you are using ECS
    pub render_time_ms: f32, // Render time
    pub tick_time_ms: f32, //
}

#[derive(Clone, Copy, Debug)]
pub struct AgentPoint {
    pub position: Vec2,
    pub radius: f32,
    pub color: [u8; 4],
    // Add extra fields here if your shaders need them (e.g., velocity for motion blur)
}

pub struct FrameData {
    pub agents: Vec<AgentPoint>, // The "Points" for the GPU
    pub signals: Vec<Signal>,    // Keep these for UI/Overlay logic
    pub debug_info: DebugInfo,
}

// --- Components (The Data) ---
#[derive(Debug, Clone, Copy)]
pub struct Position(pub Vec2); // Unique type 1
impl Deref for Position {
    type Target = Vec2;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for Position {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Velocity(pub Vec2); // Unique type 2
impl Deref for Velocity {
    type Target = Vec2;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for Velocity {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
//
pub struct AgentSize {
    pub radius: f32,
}
pub struct AgentColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
pub struct SignalEmitter {
    pub signal_id: SignalKey,
}

impl Engine {
    pub fn new() -> Self {
        let mut world = World::new();
        let mut signal_layer = SignalField::new(); // User variable name
        let width = 1024.0;
        let height = 768.0;
        let mut rng = rand::rng();

        // 1. Spawn 1000 "Dummy" Agents (No Signal)
        for _ in 0..10000 {
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
                Position(Vec2::new(rand_pos_x, rand_pos_y)),
                Velocity(Vec2::new(rand_vel_x, rand_vel_y)),
                AgentSize { radius: 2.0 },
                AgentColor {
                    r: r_val,
                    g: g_val,
                    b: b_val,
                },
            ));

            let mut mask = SignalMask::default();
            mask.set(BIT_RENDER, true);

            let sig_key = signal_layer.emit(Signal {
                entity: id,
                origin: Vec2::new(rand_pos_x, rand_pos_y),
                outer_radius: 10.0, // Use the agent's physical size
                inner_radius: 0.0,
                intensity: 1.0, // Doesn't "glow", just exists
                falloff: 0.0,
                mask,
            });

            // C. Link the SignalKey to the Entity so they stay in sync during movement
            world
                .insert_one(id, SignalEmitter { signal_id: sig_key })
                .unwrap();
        }

        // 2. Spawn THE CHOSEN ONE (With Signal)
        // Let's make it stand out (Red Color, Center Screen)
        let hero_pos = Vec2::new(512.0, 384.0);

        // 1. Spawn the Hero (without the emitter initially)
        let hero_id = world.spawn((
            Position(Vec2::new(hero_pos[0], hero_pos[1])),
            Velocity(Vec2::new(200.0, 200.0)),
            AgentSize { radius: 15.0 },
            AgentColor { r: 255, g: 0, b: 0 },
            // We leave SignalEmitter out for a split second
        ));

        // 2. Create the Signal tied to that ID
        let mut mask = SignalMask::default();
        mask.set(BIT_RENDER, true); // Bit 0 = Red Color in your GUI
        mask.set(BIT_PASSIVE, true); // Bit 0 = Red Color in your GUI
        let sig_key = signal_layer.emit(Signal {
            entity: hero_id, // Link: Signal -> Entity
            origin: hero_pos,
            outer_radius: 550.0,
            inner_radius: 120.0,
            intensity: 1.0,
            falloff: 0.5,
            mask,
        });

        // 3. Link back: Entity -> Signal
        // This adds the component to the existing entity
        world
            .insert_one(hero_id, SignalEmitter { signal_id: sig_key })
            .unwrap();

        Self {
            dummy_background_value: 0.0,
            world,
            last_update: Instant::now(),
            tick_counter: 0,
            time_accumulator: 0.0,
            last_tick_time_ms: 0.0,
            signal_layer, // Store the layer
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
            self.dummy_background_value += 60.0 * FIXED_DT;

            // 1. MOVEMENT LOOP (Updates Position)
            for (_id, (pos, vel)) in self.world.query_mut::<(&mut Position, &mut Velocity)>() {
                pos.x += vel.x * FIXED_DT;
                pos.y += vel.y * FIXED_DT;

                // Collision Logic (Bounce off walls)
                if pos.x >= 1024.0 {
                    vel.x *= -1.0;
                    pos.x = 1024.0;
                }
                if pos.x <= 0.0 {
                    vel.x *= -1.0;
                    pos.x = 0.0;
                }
                if pos.y >= 768.0 {
                    vel.y *= -1.0;
                    pos.y = 768.0;
                }
                if pos.y <= 0.0 {
                    vel.y *= -1.0;
                    pos.y = 0.0;
                }
            }

            // 2. SIGNAL SYNC LOOP (Updates SignalLayer)
            // We iterate over entities that have BOTH Position and Emitter
            for (_id, (pos, size, emitter)) in self.world.query_mut::<(&Position, &AgentSize, &SignalEmitter)>() {
                // Reposition the signal to match the agent
                // We split the borrow here: 'pos' is from 'world', 'reposition' is on 'signal_layer'
                self.signal_layer.reposition(
                    emitter.signal_id,
                    *(*pos),
                    size.radius, // Keep the radius constant (or pulse it here!)
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
        signal_mask.set(BIT_RENDER, true);
        signal_mask.set(BIT_PASSIVE, true);

        let mut layer_mask = SignalMask::default();
        layer_mask.fill(true);

        // let mut filter = SignalMask::<1>::default();
        // filter.set(BIT_RENDER, true);

        // 2. Spatial Query
        // We only care about agents the camera can actually see
        self.signal_layer.scan_volume(
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
                if let Ok(mut query) = self
                    .world
                    .query_one::<(&Position, &AgentSize, &AgentColor)>(signal.entity)
                {
                    if let Some((pos, size, color)) = query.get() {
                        frame.agents.push(AgentPoint {
                            position: **pos,
                            radius: size.radius,
                            color: [color.r, color.g, color.b, 255],
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
    }
}

////////

fn _dummy_image_checkerboard(pixels: &mut Vec<u8>, width: usize, value: f32) {
    if width == 0 {
        return;
    } // Prevent division by zero panic

    // CONFIGURATION
    // ----------------------
    let block_log2 = 5; // 2^6 = 64 pixels per square (Larger = less flickering)
    let speed = 0.25; // Pixels per second (Lower = smoother)

    // Calculate offset. We wrap it (%) by a large power of 2 to prevent
    // the float from losing precision if the app runs for hours.
    // 4096 is just an arbitrary large multiple of our block size.
    let scroll = (value * speed) as usize % 4096;

    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let x = i % width;
        let y = i / width;

        // Apply offset to X and Y.
        // We use wrapping_add to ensure safety, though usize is huge.
        let dx = x.wrapping_add(scroll);
        let dy = y.wrapping_add(scroll);

        // Logic: (x / size) XOR (y / size) checks for the "check pattern"
        // (>> block_log2) is the fastest way to divide by 64
        let is_white = ((dx >> block_log2) ^ (dy >> block_log2)) & 1 == 1;

        if is_white {
            pixel[0] = 160;
            pixel[1] = 160;
            pixel[2] = 160;
            pixel[3] = 255;
        } else {
            pixel[0] = 40;
            pixel[1] = 40;
            pixel[2] = 40;
            pixel[3] = 255;
        }
    }
}

fn _dummy_image_sunrises(pixels: &mut Vec<u8>, _width: usize, value: f32) {
    // Fastest way to iterate: Mutable chunks of 4 (R,G,B,A)
    // This compiles down to raw assembly efficiency
    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let val = ((i as u64 + value as u64) % 255) as u8;

        // Direct assignment
        pixel[0] = val;
        pixel[1] = val / 2;
        pixel[2] = 255 - val;
        pixel[3] = 255;
    }
}
