// engine.rs

use std::time::Instant;
use std::thread;

pub struct Engine {
    value: f32,
    last_update: Instant,
}

pub struct FrameData {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            value: 0.0,
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

        thread::sleep(std::time::Duration::from_millis(10)); // avoid the current 100% cpu
    }

    // Generates a dummy "World Grid" (Noise) for demo purposes
    pub fn render(&self, frame: &mut FrameData) {
        frame.pixels.clear();

        // Sanity check to ensure buffer is big enough (resizes only if screen size changed)
        let required_size = frame.width * frame.height * 4;
        if frame.pixels.len() != required_size {
            frame.pixels.resize(required_size, 0);
        }

        // Fastest way to iterate: Mutable chunks of 4 (R,G,B,A)
        // This compiles down to raw assembly efficiency
        for (i, pixel) in frame.pixels.chunks_exact_mut(4).enumerate() {
            let val = ((i as u64 + self.value as u64) % 255) as u8;

            // Direct assignment
            pixel[0] = val;
            pixel[1] = val / 2;
            pixel[2] = 255 - val;
            pixel[3] = 255;
        }
    }
}
