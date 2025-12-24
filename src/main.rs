use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]), // Set initial window size
        ..Default::default()
    };

    // Launch the app
    eframe::run_native(
        "Phase 1: Logic Prototype",
        options,
        Box::new(|cc| Ok(Box::new(EngineApp::new(cc)))),
    )
}

struct EngineApp {
    // 1. Simulation State
    age: u64,
    speed: f32,

    // 2. The Visualization Buffer (Your "Pixels")
    // We keep a texture handle to update the GPU image efficiently
    texture: Option<egui::TextureHandle>,
}

impl EngineApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            age: 0,
            speed: 1.0,
            texture: None,
        }
    }

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

impl eframe::App for EngineApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- LOGIC STEP ---
        // Increment simulation tick
        self.age += self.speed as u64;

        // --- UI STEP: Sidebar ---
        egui::SidePanel::left("controls").show(ctx, |ui| {
            ui.heading("The Director");
            ui.separator();
            ui.add(egui::Slider::new(&mut self.speed, 0.0..=10.0).text("Time Speed"));
            ui.label(format!("Tick: {}", self.age));
        });

        // --- UI STEP: Main Viewport ---
        egui::CentralPanel::default().show(ctx, |ui| {
            // 1. Create/Update the texture from raw pixel data
            // In a real engine, you'd only update this when the simulation ticks
            let image_data = self.generate_frame(320, 240);

            // Load the texture into the GPU.
            // "texture" is a handle. If we reuse the name, it updates the existing one.
            self.texture = Some(ctx.load_texture(
                "world_display",
                image_data,
                egui::TextureOptions::NEAREST, // NEAREST = Sharp pixels (Retro style)
            ));

            // 2. Draw the texture scaling to fill the available space
            if let Some(texture) = &self.texture {
                ui.image(texture);
            }
        });

        // CRITICAL: Request a repaint immediately.
        // Default behavior is to wait for mouse movement.
        // For a game/simulation, we want 60 FPS.
        ctx.request_repaint();
    }
}
