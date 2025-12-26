// gui.rs
pub use crossbeam_channel as crossbeam;
pub use eframe::egui;

struct Engine {
    value: u64,
}

impl Engine {
    pub fn new() -> Self {
        Self { value: 0 }
    }

    pub fn tick(&mut self) {
        self.value += 1;
    }

    /// Generates a dummy "World Grid" (Noise) for demo purposes
    fn generate_frame(&self, mut frame: FrameData) {
        for i in 0..(frame.width * frame.height) {
            // Simple animated pattern based on index and age
            let val = ((i as u64 + self.value) % 255) as u8;
            frame
                .pixels
                .extend_from_slice(&[val, val / 2, 255 - val, 255]); // R, G, B, A
        }
    }
}

pub struct Producer {
    returner: crossbeam::Sender<FrameData>,
    receiver: crossbeam::Receiver<FrameData>,
}

impl Producer {
    pub fn new(
        engine: Engine,
        engine_receiver: crossbeam::Receiver<FrameData>,
        engine_returner: crossbeam::Sender<FrameData>,
    ) -> Self {
        // spawns the engine thread loop
        std::thread::spawn(|| {
            let mut engine = engine;

            loop {
                engine.tick();
            }
        });

        Self {
            returner: engine_returner,
            receiver: engine_receiver,
        }
    }

    fn tick(&self) {}
}

pub struct FrameData {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

// Small wrapper for the eframe::App trait
//
// Implements the update() method that queries a frame
// and sends it to eframe to be presented on the window
//
pub struct Presenter {
    texture: Option<egui::TextureHandle>,
    frame_receiver: crossbeam::Receiver<FrameData>,
    frame_returner: crossbeam::Sender<FrameData>,
}

impl Presenter {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        frame_receiver: crossbeam::Receiver<FrameData>,
        frame_returner: crossbeam::Sender<FrameData>,
    ) -> Self {
        Self {
            texture: None,
            frame_receiver: frame_receiver,
            frame_returner: frame_returner,
        }
    }
}

impl eframe::App for Presenter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // 1. Drain the channel to get the LATEST frame
            // We loop so we skip any old frames that piled up (lag reduction)
            let mut latest_frame = None;
            while let Ok(frame) = self.frame_receiver.try_recv() {
                // If we skipped a frame, return it immediately to be reused!
                if let Some(skipped) = latest_frame {
                    let _ = self.frame_returner.send(skipped);
                }
                latest_frame = Some(frame);
            }

            // 2. If we got a new frame, update texture AND recycle
            if let Some(frame) = latest_frame {
                // Create/Update Texture
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width, frame.height],
                    &frame.pixels,
                );

                self.texture =
                    Some(ctx.load_texture("display", image, egui::TextureOptions::NEAREST));

                // RETURN THE BOTTLE!
                let _ = self.frame_returner.send(frame);
            }

            // 3. Draw existing texture (even if we didn't get a new frame this specific tick)
            if let Some(texture) = &self.texture {
                ui.image(texture);
            }

            ctx.request_repaint();
        });
    }
}
