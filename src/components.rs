// components.rs

use crate::wave::{LevelMask, Signal, SignalField, SignalKey, SignalMask};
use glam::Vec2;

#[derive(Default, Clone, Copy, Debug)]
pub struct Transform {
    pub position: Vec2,
    pub rotation: Vec2,
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
    pub a: u8,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct SignalEmitter {
    // shape
    pub radius: f32,
    pub cone_angle: f32, // 90 degrees = PI/2
    // pub rotation: f32, // 0.0 = Right, PI/2 = Up, PI = Left.
    // properties
    pub signal_mask: SignalMask,
    pub layer_mask: LevelMask,
}

// impl SignalEmitter {
// pub fn emit(field: &mut SignalField, origin: Vec2, /* direction_radians: f32, */ outer_radius: f32, inner_radius: f32, cone_angle_radians: f32, signal_mask: SignalMask, layer_mask: SignalMask, entity: hecs::Entity) -> Self {
//     let direction_unit = Vec2::new(direction_radians.cos(), direction_radians.sin());
//     let signal = Signal {
//         origin: origin,
//         direction: direction_unit,
//         outer_radius: outer_radius,
//         inner_radius: inner_radius,
//         angle_cos: (cone_angle_radians * 0.5).cos(),
//         // intensity: ,
//         // falloff: ,
//         mask: signal_mask,
//         entity: entity,
//     };
//
//     let key = field.emit(signal);
//
//     Self {
//         key: key,
//         radius: outer_radius,
//         cone_angle: cone_angle_radians, // Default to 360 (Omni)
//         // rotation: direction_radians,
//         signal_mask: signal_mask,
//         layer_mask: layer_mask,
//
//     }
// }

// Points to parent
#[derive(Debug, Clone, Copy)]
pub struct SpatialAnchor {
    pub parent: hecs::Entity,
    pub position_offset: Vec2,
}

// Points to parent
#[derive(Debug, Clone, Copy)]
pub struct LifecycleAnchor {
    pub parent: hecs::Entity,
}

// pub struct DebugCamera {
//     level_mask: LevelMask,
//     signal_mask: SignalMask,
// }
