// view_macroquad.rs

use macroquad::prelude::*;

#[allow(dead_code)]
pub struct ViewMacroquad {
    camera_position: glam::Vec3,
    //
    screen_dimensions: glam::UVec2,
}

#[allow(dead_code, unused_variables)]
impl ViewMacroquad {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            camera_position: glam::vec3(0.0, 2.0, 10.0),
            //
            screen_dimensions: glam::uvec2(width, height),
        }
    }

    pub fn run(&self) {
        let (width, height) = (self.screen_dimensions.x, self.screen_dimensions.y);

        let conf = Conf {
            window_title: "Macroquad Renderer".to_owned(),
            window_width: width as i32,
            window_height: height as i32,
            fullscreen: false,
            ..Default::default()
        };

        macroquad::Window::from_config(conf, ViewMacroquad::game_loop(width, height));
    }

    async fn game_loop(width: u32, height: u32) {
        let signals_render_target = render_target_ex(width, height, Default::default());
        let world_render_target = render_target_ex(width, height, Default::default());

        loop {
            clear_background(DARKBLUE);
            draw_text(
                "Macroquad running without the macro!",
                20.0,
                40.0,
                30.0,
                WHITE,
            );
            next_frame().await;
        }
    }
}
