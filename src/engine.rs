// engine.rs

use hecs::World;
use std::thread;
use std::time::Instant;

const FIXED_DT: f32 = 1.0 / 60.0; // Run physics exactly 60 times a second
//
pub struct Engine {
    dummy_background_value: f32,
    world: World,         // entity component system
    last_update: Instant, // used for delta_time calculation
    //
    tick_counter: u64, // overflows in ~584,000 years at 1.000.000hz
    time_accumulator: f32, //
}

#[derive(Default, Clone, Copy)]
pub struct DebugInfo {
    pub tick_counter: u64,  // overflows in ~584,000 years at 1.000.000hz
    pub agent_count: usize, // Useful since you are using ECS
}

pub struct FrameData {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub debug_info: DebugInfo,
}

// --- Components (The Data) ---
pub struct Position {
    pub x: f32,
    pub y: f32,
}
pub struct Velocity {
    pub x: f32,
    pub y: f32,
}
pub struct AgentSize {
    pub radius: f32,
}

impl Engine {
    pub fn new() -> Self {
        let mut world = World::new();

        // Spawn a test agent
        world.spawn((
            Position { x: 100.0, y: 100.0 },
            Velocity { x: 300.0, y: 150.0 }, // Moving diagonally
            AgentSize { radius: 15.0 },
        ));

        Self {
            dummy_background_value: 0.0,
            world: world,
            last_update: Instant::now(),
            tick_counter: 0,
            time_accumulator: 0.0,
        }
    }

    pub fn tick(&mut self) {
        // 1. Get current time
        let start_update = Instant::now();

        // 2. Calculate difference (Delta Time) in seconds
        let delta_time = start_update.duration_since(self.last_update).as_secs_f32();

        // 3. Reset the clock for the next tick
        self.last_update = start_update;

        // 4. Use dt
        // Example: Move at 60.0 units per second
        self.dummy_background_value += 60.0 * delta_time;

        // Agents Movement
        // We query for everything that has a Position AND Velocity
        for (_id, (pos, vel)) in self.world.query_mut::<(&mut Position, &mut Velocity)>() {
            pos.x += vel.x * delta_time;
            pos.y += vel.y * delta_time;

            // (Optional) Wrap around screen for the "Ant Farm" feel
            // In Phase 0.3, this would be collision with the Field
            if pos.x > 1024.0 || pos.x < 0.0 {
                vel.x *= -1.0;
            }
            if pos.y > 768.0 || pos.y < 0.0 {
                vel.y *= -1.0;
            }
        };

        // Add to the "bank" of time we need to simulate
        self.time_accumulator += delta_time;

        // --- SPEED LIMIT ---
        // This loop ONLY runs if 16ms have passed.
        // If accumulator is 0.0001 (fast CPU), this loop is skipped entirely.
        while self.time_accumulator >= FIXED_DT {
            // ... Physics Logic ...
            self.tick_counter += 1; // Count UPS only here
            self.time_accumulator -= FIXED_DT;
        }
    }

    pub fn render(&self, frame: &mut FrameData) {
        // let start_render = Instant::now();

        // Sanity check to ensure buffer is big enough (resizes only if screen size changed)
        let required_size = frame.width * frame.height * 4;
        if frame.pixels.len() != required_size {
            println!(
                "resize happened: {} -> {}",
                frame.pixels.len(),
                required_size
            );
            frame.pixels.resize(required_size, 0);
        }

        // 2. Render the Eulerian Grid (Your background/fields)
        // dummy_image_checkerboard(&mut frame.pixels, frame.width, self.value);
        frame.pixels.fill(0);

        // 3. Render the Lagrangian Agents (The ECS Entities)
        // We query purely for read access here
        for (_id, (pos, size)) in &mut self.world.query::<(&Position, &AgentSize)>() {
            // Simple rasterization of a circle/square at pos.x/y
            render_agent(frame, pos, size);
        }

        // 4. Debug info
        // Store in FrameData to send to UI
        frame.debug_info.tick_counter = self.tick_counter;
        frame.debug_info.agent_count = self.world.len() as usize;
    }
}

// Helper to draw agents onto the pixel buffer
fn render_agent(frame: &mut FrameData, pos: &Position, size: &AgentSize) {
    let center_x = pos.x as isize;
    let center_y = pos.y as isize;
    let r = size.radius as isize;
    let width = frame.width as isize;

    // Naive box drawing for prototype
    for y in (center_y - r)..=(center_y + r) {
        for x in (center_x - r)..=(center_x + r) {
            if x >= 0 && x < width && y >= 0 && y < frame.height as isize {
                let idx = ((y * width + x) * 4) as usize;
                // Draw Green Agent
                frame.pixels[idx] = 0;
                frame.pixels[idx + 1] = 255;
                frame.pixels[idx + 2] = 0;
                frame.pixels[idx + 3] = 255;
            }
        }
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
