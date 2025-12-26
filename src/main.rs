// main.rs
mod gui;
mod engine;

use gui::{Presenter, Producer, crossbeam};
use crate::engine::{Engine};

fn main() -> eframe::Result<()> {
    // 1. Channels (Capacity 2 is a sweet spot for double buffering)
    // - producer_tx: Engine fills this
    // - presenter_rx: Presenter reads this
    let (producer_tx, presenter_rx) = crossbeam::bounded(2);

    // - presenter_tx: Presenter fills this (with empty frames)
    // - producer_rx: Engine reads this
    let (presenter_tx, producer_rx) = crossbeam::bounded(2);

    // 2. Setup Classes
    let engine = Engine::new();
    let producer = Producer::new(producer_rx, producer_tx);
    let presenter = Presenter::new(presenter_rx, presenter_tx);

    // 3. Execution
    producer.run_thread(engine); // Spawns background thread
    presenter.run() // Blocks main thread until window closes
}
