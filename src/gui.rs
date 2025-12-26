pub use eframe::egui;

pub use std::sync::mpsc;
use std::thread;

pub struct Engine {
    age: u64,
}

impl Engine {
    pub fn new(frame_sender: mpsc::SyncSender<u8>) -> Self {
        // explicty types because i'm still learning
        // let (tx, rx): (mpsc::SyncSender<u8>, mpsc::Receiver<u8>)  = mpsc::sync_channel(1);

        std::thread::spawn(move || {
            self.tick();
            let pixels = self.generate_frame(320, 240);

            // match transmitter.try_send(pixels) {
            //     Ok(_) => {}
            //     Err(_) => {}
            // }
        });
        Self { age: 0 }
    }

    fn tick(&self) {}

    /// Generates a dummy "World Grid" (Noise) for demo purposes
    fn generate_frame(&self, width: usize, height: usize) -> egui::ColorImage {
        let mut pixels = Vec::with_capacity(width * height * 4);
        for i in 0..(width * height) {
            // Simple animated pattern based on index and age
            let val = ((i as u64 + self.age) % 255) as u8;
            pixels.extend_from_slice(&[val, val / 2, 255 - val, 255]); // R, G, B, A
        }

        egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels)
    }
}

// Small wrapper for the eframe::App trait
//
// Implements the update() method that queries a frame
// and sends it to eframe to be presented on the window
//
pub struct Presenter {
    texture: Option<egui::TextureHandle>,
    receiver: mpsc::Receiver<egui::ColorImage>,
}

impl Presenter {
    pub fn new(_cc: &eframe::CreationContext<'_>, frame_receiver: mpsc::Receiver<u8>) -> Self {
        Self {
            texture: None,
            receiver: frame_receiver,
        }
    }
}

impl eframe::App for Presenter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // retrieve the frame
            if let Ok(frame) = self.receiver.try_recv() {}

            // Load the texture into the GPU.
            // "texture" is a handle. If we reuse the name, it updates the existing one.
            self.texture = Some(ctx.load_texture(
                "world_display",
                //
                // TODO: the question now becomes should I send the
                // egui::ColorImageor just the Vec<u8 something> pixels?
                //
                // The engine class should have nothing to do with egui
                // so the best answer looks like the raw pixels,
                // but what about width/height?
                //
                frame,
                egui::TextureOptions::NEAREST, // NEAREST = Sharp pixels (Retro style)
            ));

            // 2. Draw the texture scaling to fill the available space
            if let Some(texture) = &self.texture {
                ui.image(texture);
            }

            // CRITICAL: Request a repaint immediately.
            // Default behavior is to wait for mouse movement.
            // For a game/simulation, we want 60 FPS.
            ctx.request_repaint();
        });
    }
}
