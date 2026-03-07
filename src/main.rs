// main.rs
mod app;
mod view_egui;
mod view_macroquad;
mod engine;
mod components;
mod field;

use crate::engine::Engine;
use app::{Presenter, Producer, crossbeam};

fn main() {
    // frame channel
    let (producer_tx, presenter_rx) = crossbeam::bounded(2);
    let (presenter_tx, producer_rx) = crossbeam::bounded(2);

    // command channel
    let (command_tx, command_rx) = crossbeam::bounded(1);

    let engine = Engine::new();
    let producer = Producer::new(producer_rx, producer_tx, command_rx);
    let presenter = Presenter::new(presenter_rx, presenter_tx, command_tx);

    producer.run_thread(engine); // spawns background thread
    presenter.run() // blocks main thread until window closes
}
