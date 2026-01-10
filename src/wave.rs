// wave.rs

use glam::{IVec2, Vec2};
use std::collections::HashMap;

type Position = IVec2;

#[derive(Default, Clone)]
pub struct WaveField {
    pub width: usize,
    pub height: usize,
    pub resolution: usize,
    pub cells: Vec<f32>,
    //
    signals: Vec<Signal>,
    chunk_map: HashMap<Position, Chunk>,
}

#[derive(Clone)]
enum SignalType {
    Sound,
}

#[derive(Clone)]
struct Signal {
    tag: SignalType, // Sound, Smell, Light
    origin: Vec2,    // Where did it start?
    intensity: f32,  // How strong?
    falloff: f32,    // How fast it fades?

                     // emitter_id: EntityId, // Who sent it?
                     // start_time: f64, // When?
}

impl WaveField {
    pub fn new(width: usize, height: usize, resolution: usize) -> Self {
        let w = width / resolution;
        let h = height / resolution;
        Self {
            width: w,
            height: h,
            resolution,
            cells: vec![0.0; w * h],
            signals: Vec::default(),
            chunk_map: HashMap::default(),
        }
    }

    pub fn spawn(&mut self, signal: Signal) {
        let pos: IVec2 = signal.origin.as_ivec2();
        let chunk: &mut Chunk = self.chunk_map.entry(pos).or_insert(Chunk::new());
        chunk.signals.push(signal.tag);
        //
    }

    pub fn get_value(&self, x: f32, y: f32) -> f32 {
        let index = self.get_index(x, y);
        self.cells[index]
    }

    // Convert World Position (Agent) -> Grid Index
    fn get_index(&self, world_x: f32, world_y: f32) -> usize {
        let grid_x = (world_x as usize / self.resolution).min(self.width - 1);
        let grid_y = (world_y as usize / self.resolution).min(self.height - 1);
        grid_y * self.width + grid_x
    }
}

#[derive(Clone)]
struct Chunk {
    signals: Vec<SignalType>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            signals: Vec::default(),
        }
    }
}

/////////
