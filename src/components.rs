// components.rs

use crate::wave::{SignalKey, SignalMask, LevelMask};
use glam::Vec2;

#[derive(Default, Clone, Copy, Debug)]
pub struct Transform {
    pub position: Vec2,
    // pub rotation: Vec2,
    pub scale: f32, // should be Vec2, but not supported for now
}

impl Transform {
    pub fn default() -> Self {
        Self {
            position: Vec2::default(),
            // rotation: Vec2::new(0.0, 1.0),
            scale: 1.0,
        }
    }
}

#[derive(Default)]
pub struct Velocity {
    pub linear: Vec2, // Meters per second
                      // pub angular: f32, // Radians per second (rotation speed)
}

pub struct Model {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct SignalEmitter {
    // shape
    pub radius: f32,
    pub cone_angle: f32, // 90 degrees = PI/2
    pub rotation: f32, // 0.0 = Right, PI/2 = Up, PI = Left.
    // properties
    pub signal_mask: SignalMask,
    pub layer_mask: LevelMask,
    pub key: SignalKey,
}

impl SignalEmitter {
    pub fn new(key: SignalKey) -> Self {
        Self {
            key: key,
            radius: 10.0,
            cone_angle: std::f32::consts::PI * 2.0, // Default to 360 (Omni)
            rotation: 0.0,
            signal_mask: SignalMask::ZERO,
            layer_mask: LevelMask::ZERO,
        }
    }
}

// pub struct DebugCamera {
//     level_mask: LevelMask,
//     signal_mask: SignalMask,
// }
