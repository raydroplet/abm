// engine.rs

use std::thread;
use std::time::Instant;
use hecs::World;

pub struct Engine {
    value: f32,
    world: World,
    last_update: Instant,
}

pub struct FrameData {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

// --- Components (The Data) ---
pub struct Position { pub x: f32, pub y: f32 }
pub struct Velocity { pub x: f32, pub y: f32 }
pub struct AgentSize { pub radius: f32 }

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
            value: 0.0,
            world: world,
            last_update: Instant::now(),
        }
    }

    pub fn tick(&mut self) {
        // 1. Get current time
        let now = Instant::now();

        // 2. Calculate difference (Delta Time) in seconds
        let dt = now.duration_since(self.last_update).as_secs_f32();

        // 3. Reset the clock for the next tick
        self.last_update = now;

        // 4. Use dt
        // Example: Move at 60.0 units per second
        self.value += 60.0 * dt;

        // SYSTEM: Movement
        // This replaces your hardcoded "self.value += ..."
        // We query for everything that has a Position AND Velocity
        for (_id, (pos, vel)) in self.world.query_mut::<(&mut Position, &mut Velocity)>() {
            pos.x += vel.x * dt;
            pos.y += vel.y * dt;
            
            // (Optional) Wrap around screen for the "Ant Farm" feel
            // In Phase 0.3, this would be collision with the Field
            if pos.x > 800.0 || pos.x < 0.0 { vel.x *= -1.0; } 
            if pos.y > 600.0 || pos.y < 0.0 { vel.y *= -1.0; }
        }

        thread::sleep(std::time::Duration::from_millis(10)); // avoid the current 100% cpu
    }

    pub fn render(&self, frame: &mut FrameData) {
        // Sanity check to ensure buffer is big enough (resizes only if screen size changed)
        let required_size = frame.width * frame.height * 4;
        if frame.pixels.len() != required_size {
            println!("resize happened: {} -> {}", frame.pixels.len(), required_size);
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
                frame.pixels[idx+1] = 255;
                frame.pixels[idx+2] = 0;
                frame.pixels[idx+3] = 255;
            }
        }
    }
}

////////

fn dummy_image_checkerboard(pixels: &mut Vec<u8>, width: usize, value: f32) {
    if width == 0 { return; } // Prevent division by zero panic

    // CONFIGURATION
    // ----------------------
    let block_log2 = 5;      // 2^6 = 64 pixels per square (Larger = less flickering)
    let speed = 0.25;        // Pixels per second (Lower = smoother)
    
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
            pixel[0] = 160; pixel[1] = 160; pixel[2] = 160; pixel[3] = 255;
        } else {
            pixel[0] = 40;  pixel[1] = 40;  pixel[2] = 40;  pixel[3] = 255;
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
