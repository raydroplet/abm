// components.rs
#![allow(dead_code)] // WARN: remove this when convenient

use crate::field::{Mask};
use glam::Vec2;
use kira::sound::static_sound::{StaticSoundHandle};
use hecs::{Entity};

#[derive(Default, Clone, Copy, Debug)]
pub struct Transform {
    pub position: Vec2,
    pub rotation: f32,
    pub scale: f32, // could be a Vec2. could also be bothersome to implement.
}

impl Transform {
    pub fn default() -> Self {
        Self {
            position: Vec2::default(),
            rotation: 0.0,
            scale: 1.0,
        }
    }
}

#[derive(Default)]
pub struct Velocity {
    pub linear: Vec2,
    pub angular: f32, // Radians per second (rotation speed)
    pub scalar: f32,
}

pub struct Model {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct SignalEmitter {
    pub radius_min: f32,
    pub radius_max: f32,
    pub cone_angle: f32, // 90 degrees = PI/2
    //
    // pub layer_mask: Mask,
    pub emit_mask: Mask,
    pub sense_mask: Mask,
}

// Points to parent
#[derive(Debug, Clone, Copy)]
pub struct SpatialAnchor {
    pub parent: hecs::Entity,
    pub position_offset: Vec2,
    pub rotation_offset: f32,
    pub scale_offset: f32,
}

#[derive(Debug, Clone)]
pub struct Label {
    pub name: String,
}

// won't bother for now
pub struct Camera {
    // pub level_mask: LevelMask,
    // pub signal_mask: SignalMask,
}

// markers
pub struct Wolf;
pub struct Audio;


//-//-//-//-//-//-//-//
// WARN: experimental

//// audio with kira

pub struct AudioListener {
    pub last_active_sources: Vec<Entity>,
}

// kira will keep the audio playing in the background even if there are no entities listenting.
// this engine aspect, and many others, can be a source of optimization, but for now it's fine.
pub struct AudioSourcePersistent {
    // pub sound_data: StaticSoundData,
    pub handle: StaticSoundHandle,
    pub base_volume: f32,
}

// Points to parent
#[derive(Debug, Clone, Copy)]
pub struct LifecycleAnchor {
    pub parent: hecs::Entity,
}

//// wolf logic

#[derive(PartialEq, Eq)]
pub enum SeekerState {
    Idle,
    Alert,
    Seeking,
    Chasing,
}

pub struct Seeker {
    pub state: SeekerState,
    pub target_source: Vec2,
    pub target: Entity,
    pub vision_entity: Entity,
}

//
//-//-//-//-//-//-//-//
