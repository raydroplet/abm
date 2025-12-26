// main.rs
mod gui;
use gui::{Presenter, egui, crossbeam};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_resizable(false),
        run_and_return: false,
        ..Default::default()
    };

    let (presenter_sender, engine_returner) = crossbeam::bounded(1);
    let (engine_sender, presenter_returner) = crossbeam::bounded(1);

    // Launch the app
    eframe::run_native(
        "Phase 1: Prototype",
        options,
        // TODO: figure out this double box api callback syntax choice fully
        Box::new(|context| {
            Ok(Box::new(Presenter::new(
                context,
                presenter_returner,
                presenter_sender,
            )))
        }),
    )
}
