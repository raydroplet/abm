// main.rs
mod components;
mod view_egui;
mod engine;
mod gui;
mod wave;

use crate::engine::Engine;
use gui::{Presenter, Producer, crossbeam};

fn main() {
    // 1. Channels (Capacity 2 is a sweet spot for double buffering)
    // - producer_tx: Engine fills this
    // - presenter_rx: Presenter reads this
    let (producer_tx, presenter_rx) = crossbeam::bounded(2);

    // - presenter_tx: Presenter fills this (with empty frames)
    // - producer_rx: Engine reads this
    let (presenter_tx, producer_rx) = crossbeam::bounded(2);

    // // Command channel
    let (command_tx, command_rx) = crossbeam::bounded(1);

    // 2. Setup Classes
    let engine = Engine::new();
    let producer = Producer::new(producer_rx, producer_tx, command_rx);
    let presenter = Presenter::new(presenter_rx, presenter_tx, command_tx);

    // 3. Execution
    producer.run_thread(engine); // Spawns background thread
    presenter.run() // Blocks main thread until window closes
}
