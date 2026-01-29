// wave.rs

use bitvec::prelude::*;
use glam::{IVec2, Mat2, Vec2};
use rustc_hash::FxHashMap;
// use slotmap::{SlotMap, new_key_type};
use smallvec::SmallVec;
use std::f32::consts::TAU;

// ==================================================================================
// 1. DATA STRUCTURES
// ==================================================================================

const LEVEL_COUNT: usize = 64;
const N: usize = 1;
pub type LevelCount = [u32; LEVEL_COUNT];
//
type CommonBitArray = BitArray<[u64; N]>;
pub type ShadowMask = CommonBitArray;
pub type SignalMask = CommonBitArray;
pub type LevelMask = CommonBitArray;
//
pub type Bucket = SmallVec<[(SignalKey, SignalMask); 4]>; // The Bucket: A list of (ID, Mask) tuples
pub type SpatialGrid = FxHashMap<TileKey, Bucket>; // The Grid: Map of Coordinates -> Bucket

// new_key_type! { pub struct SignalKey; }
// can be swapped for u64 latter and use entity.to_bits() to avoid coupling with hecs
// it is used here for convenience and to remember what they key should actually be
pub type SignalKey = hecs::Entity; // we will just use the generational hecs::Entity as the key

// level: how big is the tile? are we looking at a 1km square or a 1m square?
// x,y: grid positions for this tile
//
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct TileKey {
    level: u8,
    x: i32,
    y: i32,
}

// CONST GENERIC SIGNAL
// N = Number of BYTES (u8).
// Signal<10> = 10 bytes = 80 bits.
#[derive(Clone, Copy, Debug, Default)]
pub struct Signal<const N: usize = 1> {
    // source
    pub origin: Vec2, // !!!
    pub unit_direction: Vec2,
    // shape
    pub outer_radius: f32,
    pub inner_radius: f32,
    pub angle_radians: f32,
    // force
    // pub intensity: f32, // How strong?
    // pub falloff: f32,   // How fast it fades?
    // data
    pub emit_mask: SignalMask,
    pub sense_mask: SignalMask,
}
//
// impl Signal {
//     pub fn new_sphere(origin: Vec2, radius: f32, mask: SignalMask, entity: Entity) -> Self {
//         Self {
//             origin,
//             direction: Vec2::X,
//             outer_radius: radius,
//             inner_radius: 0.0,
//             angle_cos: -1.0,
//             // intensity: 1.0,
//             // falloff: 0.1,
//             mask: mask,
//             // entity: entity,
//         }
//     }
// }

// ==================================================================================
// 2. THE SYSTEM
// ==================================================================================

pub struct SignalField {
    // pub store: SlotMap<SignalKey, Signal>,
    pub store: FxHashMap<SignalKey, Signal>,

    // Mask stored as bytes
    // stores signal identifiers (SignalKey) for a specific tile (TileKey)
    //
    // the value includes the SignalType for filtering the
    // ones the agent querying cares about, without having
    // to lookup (memory acess) the actual Signal struct
    //
    grid: SpatialGrid,

    active_levels: LevelMask,
    level_counts: LevelCount,
}

impl SignalField {
    pub fn new() -> Self {
        Self {
            // store: SlotMap::with_key(),
            store: FxHashMap::default(),
            grid: FxHashMap::default(),
            active_levels: LevelMask::default(),
            level_counts: [0; LEVEL_COUNT],
        }
    }

    // =========================================================================
    // PUBLIC API
    // =========================================================================

    /// CREATE: Generates a NEW Key
    pub fn emit(&mut self, signal: Signal, key: SignalKey) {
        // 1. Destructure Self (Split Borrows)
        let Self {
            store,
            grid,
            active_levels,
            level_counts,
            ..
        } = self;

        Self::internal_add(grid, active_levels, level_counts, key, &signal);
        store.insert(key, signal);
    }

    /// DELETE: Removes existing Key
    pub fn cease(&mut self, key: SignalKey) {
        // 1. Destructure Self
        // We don't need active_levels for removal
        let Self {
            store,
            grid,
            active_levels,
            level_counts,
            ..
        } = self;

        // 2. Look up the signal to see where it was
        if let Some(sig) = store.get(&key) {
            // 3. Remove from Grid (Using specific fields, not the whole struct)
            // matching the signature of internal_remove(grid, key, radius, origin)
            Self::internal_remove(
                grid,
                active_levels,
                level_counts,
                key,
                sig.outer_radius,
                sig.origin,
            );
        }

        // 4. Finally remove from storage
        store.remove(&key);
    }

    /// updates Position and radius
    pub fn reposition(&mut self, key: SignalKey, new_pos: Vec2, outer_radius: f32) {
        // 1. SPLIT SELF
        let Self {
            store,
            grid,
            active_levels,
            level_counts,
            ..
        } = self;

        // 2. Get the signal (Mutable Borrow of STORE)
        let signal = if let Some(sig) = store.get_mut(&key) {
            sig
        } else {
            return;
        };

        // insanely efficient for high agent count
        // ---------------------------------------------------------------------
        // OPTIMIZATION START
        // ---------------------------------------------------------------------

        // Calculate the Spatial Hash for where it IS vs where it WANTS TO BE
        let old_tile = Self::get_coordinates(signal.outer_radius, signal.origin);
        let new_tile = Self::get_coordinates(outer_radius, new_pos);

        // If the agent moved, but didn't cross a grid boundary, we skip the HashMap thrashing.
        // The Grid points to the SignalKey, and the SignalKey is still valid.
        // We only update the raw data in 'store'.
        if old_tile == new_tile {
            signal.origin = new_pos;
            signal.outer_radius = outer_radius;
            return;
        }

        // ---------------------------------------------------------------------
        // OPTIMIZATION END (Fallback to Slow Path)
        // ---------------------------------------------------------------------

        // 3. Remove from OLD grid using OLD coordinates (derived from signal data before update)
        // We manually reproduce the logic of internal_remove to avoid borrow checker wars,
        // or just call it if arguments align (they do).
        Self::internal_remove(
            grid,
            active_levels,
            level_counts,
            key,
            signal.outer_radius, // Pass OLD radius
            signal.origin,       // Pass OLD origin
        );

        // 4. Update the Signal (In Place)
        signal.origin = new_pos;
        signal.outer_radius = outer_radius;

        // 5. Add to NEW grid
        // internal_add calculates the key based on the *current* signal state, which we just updated.
        Self::internal_add(grid, active_levels, level_counts, key, signal);
    }

    /// Updates the facing direction and field-of-view of a signal.
    /// This is O(1) as it does not require updating the spatial grid.
    pub fn reshape(
        &mut self,
        key: SignalKey,
        new_direction_radians: f32,
        new_angle_radians: f32,
        inner_radius: f32,
    ) {
        if let Some(signal) = self.store.get_mut(&key) {
            // 1. Update Direction
            // We convert the angle back into a normalized Vec2
            let (sin, cos) = new_direction_radians.sin_cos();
            signal.unit_direction = Vec2::new(cos, sin);

            // 2. Update Aperture (Cone Angle)
            // We assume 'aperture' is the Full Angle (e.g., 90 degrees).
            // The dot product check requires the cosine of the Half Angle.
            //
            // Example:
            signal.angle_radians = new_angle_radians;
            signal.inner_radius = inner_radius;
        }
    }

    //////////
    /// Scan

    // /// READ: The Scan Loop
    // pub fn scan_point(
    //     &self,
    //     pos: Vec2,
    //     signal_mask: SignalMask,
    //     layer_mask: LevelMask,
    //     mut callback: impl FnMut(&Signal),
    // ) {
    //     let scanning = self.active_levels & layer_mask;
    //
    //     for level in scanning.iter_ones() {
    //         // let level = scanning.trailing_zeros() as u8;
    //         // scanning &= !(1 << level);
    //
    //         let cell_size = (1 << level) as f32;
    //         let grid_x = (pos[0] / cell_size).floor() as i32;
    //         let grid_y = (pos[1] / cell_size).floor() as i32;
    //
    //         if let Some(bucket) = self.grid.get(&TileKey {
    //             level: level as u8,
    //             x: grid_x,
    //             y: grid_y,
    //         }) {
    //             for (key, sig_mask) in bucket {
    //                 if (*sig_mask & signal_mask).any() {
    //                     if let Some(sig) = self.store.get(key) {
    //                         if self.check_intersection_point(pos, sig) {
    //                             callback(sig);
    //                         }
    //                     }
    //                 }
    //             }
    //         }
    //     }
    // }

    pub fn scan_volume_rectangle(
        &self,
        min: Vec2,
        max: Vec2,
        query_mask: SignalMask,
        layer_mask: LevelMask,
        mut callback: impl FnMut(&Signal, &hecs::Entity),
    ) {
        let scanning = self.active_levels & layer_mask;

        for level in scanning.iter_ones() {
            let (min_g, max_g) = Self::get_tile_range(min, max, level);

            // 2. Iterate the range (The Volume Loop)
            for gx in min_g.x..max_g.x {
                for gy in min_g.y..max_g.y {
                    if let Some(bucket) = self.grid.get(&TileKey {
                        level: level as u8,
                        x: gx,
                        y: gy,
                    }) {
                        for (key, sig_mask) in bucket {
                            // Quick Mask Filter
                            if (*sig_mask & query_mask).any() {
                                if let Some(sig) = self.store.get(key) {
                                    callback(sig, key);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn scan(
        &self,
        key: SignalKey,
        layer_mask: LevelMask,
        mut callback: impl FnMut(&Signal, &SignalKey),
    ) {
        let scanning = self.active_levels & layer_mask;
        let signal = self.store.get(&key).expect("Invalid key");

        // 1. BROAD PHASE: Calculate a loose Bounding Box
        // Optimizing the AABB of a rotated cone is hard.
        // query the square AABB of the full (circle) radius.
        // It over-selects tiles, the Narrow Phase filters them out.
        // A better bounding box algorithm may benefit in high range scenarios.
        let min_aabb = signal.origin - Vec2::splat(signal.outer_radius);
        let max_aabb = signal.origin + Vec2::splat(signal.outer_radius);

        for level in scanning.iter_ones() {
            let (min_range, max_range) = Self::get_tile_range(min_aabb, max_aabb, level);

            for gx in min_range.x..max_range.x {
                for gy in min_range.y..max_range.y {
                    if let Some(bucket) = self.grid.get(&TileKey {
                        level: level as u8,
                        x: gx,
                        y: gy,
                    }) {
                        for (key, sig_mask) in bucket {
                            if (*sig_mask & signal.sense_mask).any() {
                                if let Some(target) = self.store.get(key) {
                                    // 2. NARROW PHASE: Cone vs Circle Intersection
                                    if self.check_intersection_arc_circle(signal, target) {
                                        callback(target, key);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn scan_occluded(
        &self,
        key: SignalKey,
        layer_mask: LevelMask,
        occlusion_mask: LevelMask,
        mut callback: impl FnMut(&Signal, &SignalKey, ShadowMask),
    ) {
        let scanning = self.active_levels & layer_mask;
        let viewer = self.store.get(&key).expect("Invalid key");

        // 1. PRE-CALCULATION
        let min_aabb = viewer.origin - Vec2::splat(viewer.outer_radius);
        let max_aabb = viewer.origin + Vec2::splat(viewer.outer_radius);

        // 2. COLLECTION PHASE (Flattened)
        let mut signal_buffer: SmallVec<[(&Signal, &SignalKey, f32); 64]> = SmallVec::new();

        for level in scanning.iter_ones() {
            let (min_range, max_range) = Self::get_tile_range(min_aabb, max_aabb, level);

            for gx in min_range.x..max_range.x {
                for gy in min_range.y..max_range.y {
                    let t_key = TileKey {
                        level: level as u8,
                        x: gx,
                        y: gy,
                    };

                    // Direct Grid Access
                    if let Some(bucket) = self.grid.get(&t_key) {
                        for (sig_key, emit_mask) in bucket {
                            // Filter 1: Sense Mask
                            if (*emit_mask & viewer.sense_mask).any() {
                                if let Some(target) = self.store.get(sig_key) {
                                    let dist = (target.origin - viewer.origin).length();
                                    let edge_dist = dist - target.outer_radius;

                                    // Filter 2: Strict Radius Check
                                    // (Discard tile corners outside the view circle)
                                    if edge_dist > viewer.outer_radius {
                                        continue;
                                    }

                                    signal_buffer.push((target, sig_key, edge_dist));
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. SORTING PHASE
        // Sort everything by distance (closest first).
        signal_buffer
            .sort_unstable_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

        // 4. PROJECTION & OCCLUSION PHASE
        let mut shadow_mask: ShadowMask = BitArray::ZERO;

        // Convert the stored cosine back to radians for projection
        let half_fov_radians = viewer.angle_radians; // BUG: untested

        for (sig, sig_key, edge_dist) in signal_buffer {
            let center_dist = edge_dist + sig.outer_radius;

            let projection_mask = Self::project_to_view_angular(
                sig,
                viewer.origin,
                viewer.unit_direction,
                half_fov_radians,
                center_dist,
            );

            // Visibility Check: Is any part of the projection NOT in the shadow?
            let is_visible = (projection_mask & !shadow_mask).any();

            if is_visible {
                callback(sig, sig_key, shadow_mask);
            }

            // Update Shadow: If this object blocks light, add it to the mask
            if (sig.emit_mask & occlusion_mask).any() {
                shadow_mask |= projection_mask;
            }
        }
    }

    pub fn scan_cone_occluded_old(
        &self,
        origin: Vec2,
        direction: Vec2,
        angle_cos: f32, // Aperture (e.g. 0.707 for ~90deg FOV)
        max_dist: f32,
        occluder_mask: SignalMask, // Signals that CREATE shadows (Walls)
        target_mask: SignalMask,   // Signals we want to SEE (Entities)
        layer_mask: LevelMask,
        mut callback: impl FnMut(&Signal, ShadowMask), // Callback receives Signal + Visible Bits
    ) {
        // 0. PRE-CALCULATION

        // Pre-calc geometric constants for our cone
        // angle_cos is cos(theta). We need sin(theta).
        // Identity: sin^2 + cos^2 = 1  =>  sin = sqrt(1 - cos^2)
        let half_cone_sin = (1.0 - angle_cos * angle_cos).max(0.0).sqrt();
        let cone_right = Vec2::new(-direction.y, direction.x); // Rotate 90 deg

        // levels to iterate, only the active ones
        let scanning = self.active_levels & layer_mask;

        // AABB for the whole cone
        // Vec2::splat(x) creates [x, x]
        let min_scan = origin - Vec2::splat(max_dist);
        let max_scan = origin + Vec2::splat(max_dist);

        // Buffer for tiles to process: (SortKey, TileKey)
        // SortKey = Distance - LevelBias. We want closest tiles first.
        let mut tile_buffer: SmallVec<[(f32, TileKey); 16]> = SmallVec::new();

        // 1. TILE COLLECTION PHASE
        for level in scanning.iter_ones() {
            let cell_size = (1 << level) as f32;
            let level_bias = cell_size; // Prefer higher levels (larger objects) if distances are equal

            // Get grid range padded by 1 (to catch bleeding signals)
            let min_gx = ((min_scan.x / cell_size).floor() as i32) - 1;
            let max_gx = ((max_scan.x / cell_size).floor() as i32) + 1;
            let min_gy = ((min_scan.y / cell_size).floor() as i32) - 1;
            let max_gy = ((max_scan.y / cell_size).floor() as i32) + 1;

            for gx in min_gx..max_gx {
                for gy in min_gy..max_gy {
                    let key = TileKey {
                        level: level as u8,
                        x: gx,
                        y: gy,
                    };

                    // 1. Reconstruct Tile AABB
                    let tile_min = Vec2::new(gx as f32 * cell_size, gy as f32 * cell_size);
                    let tile_max = tile_min + Vec2::splat(cell_size);

                    // 2. Tile Cull (Cone Intersection)
                    // We already know it's inside the max_dist (because of the AABB loop limits),
                    // but is it inside the ANGLE?
                    if !Self::aabb_intersects_cone(
                        tile_min,
                        tile_max,
                        origin,
                        direction,
                        half_cone_sin,
                        max_dist,
                    ) {
                        continue;
                    }

                    if self.grid.contains_key(&key) {
                        // Calculate distance to Tile AABB
                        let dist = Self::dist_sq_point_to_tile(origin, gx, gy, cell_size).sqrt();
                        // level_bias: prioritizes Big Occluders over small ones when distances are comparable
                        tile_buffer.push((dist - level_bias, key));
                    }
                }
            }
        }

        // 2. SORT PHASE (Front-to-Back)
        // We use partial_cmp because f32.
        tile_buffer
            .sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // 3. EXECUTION PHASE
        // The View Line: 0 = Clear, 1 = Blocked
        let mut occlusion_buffer: ShadowMask = BitArray::ZERO;

        // Helper buffer for sorting signals inside a bucket
        let mut signal_buffer: SmallVec<[(&Signal, f32); 16]> = SmallVec::new();

        for (_, tile_key) in tile_buffer {
            // Optimization: If view is completely black (111111...), stop.
            if occlusion_buffer.all() {
                break;
            }

            if let Some(bucket) = self.grid.get(&tile_key) {
                signal_buffer.clear();

                // 3a. Bucket Collection
                for (key, mask) in bucket {
                    if (*mask & (occluder_mask | target_mask)).any() {
                        if let Some(sig) = self.store.get(key) {
                            // 1. Calculate Real Distance
                            let dist = (sig.origin - origin).length();

                            // 2. Calculate Edge Distance (Sort Key)
                            // If we are inside the radius, this becomes negative, which is good!
                            // It ensures "surrounding" signals are processed first.
                            let edge_dist = dist - sig.outer_radius;

                            // 3. Range Check
                            if edge_dist < max_dist {
                                signal_buffer.push((sig, edge_dist));
                            }
                        }
                    }
                }

                // 3b. Bucket Sort
                // Now we are sorting by Edge Distance, which is safe for large objects
                signal_buffer.sort_unstable_by(|a, b| {
                    a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
                });

                // 3c. Processing
                // 3c. Processing
                for (sig, edge_dist) in &signal_buffer {
                    // Reconstruct Center Distance (Free!)
                    let center_dist = edge_dist + sig.outer_radius;

                    let proj_mask = Self::project_to_view_line(
                        sig,
                        origin,
                        direction,
                        cone_right,
                        half_cone_sin,
                        center_dist, // Pass the center distance
                    );

                    if !proj_mask.not_any() {
                        continue;
                    }

                    // 1. Calculate Visible Bits
                    // Logic: "What parts of the Signal overlap with the Empty Space?"
                    // Note: We use parentheses around (!occlusion_buffer) to ensure order of operations
                    let visible_bits = proj_mask & (!occlusion_buffer);

                    // 2. Check if visible (REPLACES: != 0)
                    // .any() returns true if the mask contains at least one '1'
                    if visible_bits.any() && (sig.emit_mask & target_mask).any() {
                        callback(sig, visible_bits);
                    }

                    // 3. Update Occlusion
                    if (sig.emit_mask & occluder_mask).any() {
                        // If |= gives you trouble, use the explicit form:
                        occlusion_buffer = occlusion_buffer | proj_mask;
                    }
                }
            }
        }
    }

    // =========================================================================
    // PRIVATE HELPERS (The deduplication magic)
    // =========================================================================

    fn internal_remove(
        grid: &mut SpatialGrid,
        active_levels: &mut LevelMask,
        level_counts: &mut LevelCount,
        key: SignalKey,
        outer_radius: f32,
        origin: Vec2,
    ) {
        let tile_key = Self::get_coordinates(outer_radius, origin);
        let level = tile_key.level as usize;

        if let Some(bucket) = grid.get_mut(&tile_key) {
            if let Some(idx) = bucket.iter().position(|(k, _)| *k == key) {
                bucket.swap_remove(idx);

                // 1. Decrement Counter
                level_counts[level] = level_counts[level].saturating_sub(1);

                // 2. If level is now empty, flip the bit to 0
                if level_counts[level] == 0 {
                    active_levels.set(level, false);
                }
            }

            // Cleanup empty bucket to prevent Hashmap bloat
            if bucket.is_empty() {
                grid.remove(&tile_key);
            }
        }
    }

    fn internal_add(
        grid: &mut SpatialGrid,
        active_levels: &mut LevelMask,
        level_counts: &mut LevelCount,
        key: SignalKey,
        signal: &Signal,
    ) {
        let tile_key = Self::get_coordinates(signal.outer_radius, signal.origin);
        let level = tile_key.level as usize;

        // 1. Update Bitmask & Counter
        active_levels.set(level, true);
        level_counts[level] += 1;

        // 2. Insert into Grid
        grid.entry(tile_key)
            .or_default()
            .push((key, signal.emit_mask));
    }

    // fn check_collision_hollow_sphere(&self, pos: Vec2, sig: &Signal) -> bool {
    //     let dx = pos[0] - sig.origin.x;
    //     let dy = pos[1] - sig.origin.y;
    //     let dist_sq = dx * dx + dy * dy;
    //     dist_sq <= (sig.outer_radius * sig.outer_radius)
    //         && dist_sq >= (sig.inner_radius * sig.inner_radius)
    // }

    // fn check_intersection_point(&self, target_pos: Vec2, sig: &Signal) -> bool {
    //     let to_target = target_pos - sig.origin;
    //
    //     // 1. Calculate Squared Distance (Cheap: x*x + y*y)
    //     let dist_sq = to_target.length_squared();
    //
    //     // 2. Early Radius Check (Cheap)
    //     if dist_sq > sig.outer_radius * sig.outer_radius {
    //         return false;
    //     }
    //
    //     // 3. Dot Product (Un-normalized)
    //     let dot = sig.direction.dot(to_target);
    //
    //     // 4. "Behind" Check
    //     // If dot is negative, the target is behind us.
    //     // Unless you have >180 FOV, this is an instant fail.
    //     if dot < 0.0 {
    //         return false;
    //     }
    //
    //     // 5. Check
    //     // Instead of: dot / sqrt(dist_sq) > angle_cos
    //     // We use:     dot * dot > angle_cos * angle_cos * dist_sq
    //     let threshold_sq = sig.angle_cos * sig.angle_cos * dist_sq;
    //
    //     if dot * dot < threshold_sq {
    //         return false;
    //     }
    //
    //     true
    // }

    // NOTE: fully implemented by hand, you may as well not go into the trouble of touching it again
    pub fn check_intersection_arc_circle(
        &self,
        viewer_cone: &Signal,
        target_circle: &Signal,
    ) -> bool {
        // 1. Circle Intersection
        //
        // Interpreting both as circles, check if the edges are too far considering the center/radius
        // Fórmula para distância entre dois pontos: D^2 = (x_2 - x_1)^2 + (y_2 - y_1)^2
        // (aka Teorema de Pitágoras, já que a distância é a hipotenusa)
        // It's better to ^2 the radius sum than to calculate a sqrt on the Distance squared.

        let v = viewer_cone;
        let t = target_circle;
        let v_o = v.origin;
        let t_o = t.origin;
        let distance_squared =
            (v_o.x - t_o.x) * (v_o.x - t_o.x) + (v_o.y - t_o.y) * (v_o.y - t_o.y);
        let max_radius_sum_squared =
            (v.outer_radius + t.outer_radius) * (v.outer_radius + t.outer_radius);
        if distance_squared > max_radius_sum_squared {
            return false; // they can't possibly intersect
        }

        // also handle the inner radius
        // the max(0.0) is to avoid negative squares, which causes unwanted intersetion behavior
        let hide_limit = (v.inner_radius - t.outer_radius).max(0.0);
        let hide_limit_sq = hide_limit * hide_limit;

        // 3. The Distance Check
        // We use '<' because we want to hide it if it is closer than the limit.
        if distance_squared < hide_limit_sq {
            return false; // fully hidden inside the blind spot
        }

        // 2. Angle alignment
        //
        //    if the previous test passed, we just need to check for the angle
        //    half angle is needed considering the cone direction is at the center of the cone_angle
        //    angle('cone direction', 'target direction') < (cone_angle / 2)
        //

        let unit_to_target = (target_circle.origin - viewer_cone.origin).normalize();
        let cosine_limit = (viewer_cone.angle_radians * 0.5).cos();
        let signals_cosine = viewer_cone.unit_direction.dot(unit_to_target);
        if cosine_limit < signals_cosine {
            return true;
        }

        // 3. Flank check
        //
        //    finally, we check if the target intersects the edges of the view cone.
        //    If we imagine the edges of the view cone as a line (say on x axis) and calculate
        //    the distance of our target sphere center to it: if radius >= distance then we know
        //    an intersection happened
        //
        //    0. calculate the 'center vector' from the 'cone origin' to the 'target center'
        //
        //    1. calculate both edge vectors, and for each one of them:
        //
        //    2. project the 'center vector' onto the 'edge vector', this returns a distance
        //       we will use it to calculate one of the sides of a triangle.
        //
        //    3. clamp the line. since the projection assumes a infinite line and our edge vector
        //       is finite we clamp said casted_length to one of the edges borders if necessary
        //       => clamped_casted_length = clamp(0, casted_length, radius)
        //
        //    4. calculate the full lenght edge vector by multiplying said unit with our clamped_casted_length
        //       full_edge_vector = (unit_edge_direction * clamped_casted_length)
        //
        //    5. By subtracting two vectors that form a triangle, we get a third one that completes it.
        //       distance_vector = center_vector - full_edge_vector
        //
        //    6.
        //       We can then use this new vector to get the distance to the edge
        //       distance_squared = distance_vector.length_squared()
        //
        //    7. check for an intersection: (target_radius_squared > distance_squared)

        // 0.
        let center_vector = target_circle.origin - viewer_cone.origin;

        // 1.
        let center_dir = viewer_cone.unit_direction;
        let half_angle = viewer_cone.angle_radians * 0.5;

        let rot_left = Mat2::from_angle(half_angle);
        let rot_right = Mat2::from_angle(-half_angle);
        let edge_vectors = [rot_left * center_dir, rot_right * center_dir];

        for unit_edge in edge_vectors {
            // 2.
            let casted_length: f32 = center_vector.dot(unit_edge);

            // 3.
            let casted_length = casted_length.clamp(0.0, viewer_cone.outer_radius);

            // 4.
            let full_edge = unit_edge * casted_length;

            // 5.
            let distance_vector = center_vector - full_edge;

            //6.
            let distance_squared = distance_vector.length_squared();

            // 7.
            let radius_squared = target_circle.outer_radius * target_circle.outer_radius;
            if radius_squared > distance_squared {
                return true;
            }
        }

        false
    }

    /// Projects a signal sphere onto the view line.
    /// Returns ShadowMask::ZERO if the signal is not visible or outside the cone.
    fn project_to_view_line(
        sig: &Signal,
        origin: Vec2,
        cone_dir: Vec2,
        cone_right: Vec2,
        half_cone_sin: f32,
        dist: f32,
    ) -> ShadowMask {
        let total_bits = std::mem::size_of::<ShadowMask>() * 8;
        let max_idx = total_bits as f32;

        // 0. Safety / Inside-Body Check
        if dist < 0.00001 {
            return !ShadowMask::ZERO; // Full Mask
        }

        // 1. Front Check (Culling objects behind)
        let to_target = sig.origin - origin;
        // Note: We use the already-calculated to_target, avoiding re-calculation
        let forward_dist = to_target.dot(cone_dir);

        // If strictly behind the camera plane (plus radius), returns Empty.
        if forward_dist < -sig.outer_radius {
            return ShadowMask::ZERO;
        }

        // 2. Lateral Projection (Sine Space)
        let dist_inv = 1.0 / dist;
        let lateral_offset = to_target.dot(cone_right);

        // Normalized Coordinates (-1.0 to 1.0 relative to cone width)
        let screen_x = (lateral_offset * dist_inv) / half_cone_sin;
        let screen_width = (sig.outer_radius * dist_inv) / half_cone_sin;

        // 3. Bit Indices Calculation
        let center_ratio = 0.5 + (0.5 * screen_x);
        let width_ratio = 0.5 * screen_width;

        let start_idx = ((center_ratio - width_ratio) * max_idx)
            .floor()
            .clamp(0.0, max_idx) as usize;
        let end_idx = ((center_ratio + width_ratio) * max_idx)
            .ceil()
            .clamp(0.0, max_idx) as usize;

        if start_idx >= end_idx {
            return ShadowMask::ZERO;
        }

        // 4. Construct Mask
        let mut mask = ShadowMask::ZERO;

        // Safety: bitvec handles slicing bounds automatically
        if end_idx > start_idx {
            mask[start_idx..end_idx].fill(true);
        }

        mask
    }

    /// Projects a signal onto the view line using exact angular math.
    /// "half_fov" should be the cone half-angle in radians (e.g., PI/4 for 90 deg FOV).
    fn project_to_view_angular(
        sig: &Signal,
        origin: Vec2,
        cone_dir: Vec2,
        half_fov: f32,
        dist: f32,
    ) -> ShadowMask {
        let total_bits = std::mem::size_of::<ShadowMask>() * 8;
        let max_idx = total_bits as f32;

        if dist < 0.0001 {
            return !ShadowMask::ZERO;
        }

        // 1. Calculate the Angle of the object relative to the view direction
        let to_target = sig.origin - origin;

        // "perp_dot" gives us the signed lateral distance (2D cross product equivalent)
        // assuming cone_dir is normalized.
        let det = cone_dir.x * to_target.y - cone_dir.y * to_target.x;
        let dot = cone_dir.dot(to_target);

        // atan2 gives us the exact angle in radians (-PI to PI) relative to forward
        let angle = det.atan2(dot);

        // 2. Calculate the Angular Width of the object
        // A sphere of radius R at distance D spans an angle of 2 * asin(R/D).
        // (We use asin to handle the curvature of the sphere correctly close up)
        let angular_half_width = (sig.outer_radius / dist).min(1.0).asin();

        // 3. Map Angles to Screen Coordinates (-1.0 to 1.0)
        // We normalize by the FOV.
        let screen_center = angle / half_fov;
        let screen_width = angular_half_width / half_fov;

        // 4. Bit Indices Calculation (Linear Space)
        let start_ratio = 0.5 + (0.5 * (screen_center - screen_width));
        let end_ratio = 0.5 + (0.5 * (screen_center + screen_width));

        let start_idx = (start_ratio * max_idx).floor().clamp(0.0, max_idx) as usize;
        let end_idx = (end_ratio * max_idx).ceil().clamp(0.0, max_idx) as usize;

        if start_idx >= end_idx {
            return ShadowMask::ZERO;
        }

        let mut mask = ShadowMask::ZERO;
        if end_idx > start_idx {
            mask[start_idx..end_idx].fill(true);
        }
        mask
    }

    /// Calculates squared distance from a point to a Grid Cell (AABB).
    fn dist_sq_point_to_tile(p: Vec2, gx: i32, gy: i32, size: f32) -> f32 {
        let min_x = gx as f32 * size;
        let min_y = gy as f32 * size;
        let max_x = min_x + size;
        let max_y = min_y + size;

        // AABB Clamping
        let cx = p.x.clamp(min_x, max_x);
        let cy = p.y.clamp(min_y, max_y);

        let dx = p.x - cx;
        let dy = p.y - cy;

        dx * dx + dy * dy
    }

    fn aabb_intersects_cone(
        min: Vec2,
        max: Vec2,
        origin: Vec2,
        dir: Vec2,
        half_sin: f32,
        radius: f32,
    ) -> bool {
        // 1. Closest Point Test (Distance)
        let closest = origin.clamp(min, max);
        let dist_sq = (closest - origin).length_squared();
        if dist_sq > radius * radius {
            return false;
        }

        // 2. Vertex Test (Angle)
        // If the closest point is inside the tile (dist ~= 0), we are INSIDE the tile. Visible.
        if dist_sq < 0.0001 {
            return true;
        }

        // Otherwise, check if any corner of the box is within the cone angles.
        // (This is a simplified check; technically a box can intersect a cone without a corner being inside,
        // but for culling grid tiles, checking the 4 corners is usually sufficient and fast).
        let corners = [min, Vec2::new(max.x, min.y), Vec2::new(min.x, max.y), max];

        for c in corners {
            let to_corner = (c - origin).normalize_or_zero();
            // Dot product > cos(theta) means inside angle
            // (You can pass angle_cos into this function to make this cheap)
            if to_corner.dot(dir) >= (1.0 - half_sin * half_sin).sqrt() {
                return true;
            }
        }

        false
    }

    fn get_coordinates(outer_radius: f32, origin: Vec2) -> TileKey {
        let level = Self::get_level(outer_radius);

        // 1. Calculate the actual size of a cell at this level
        // level 0 = 1.0, level 1 = 2.0, level 2 = 4.0, etc.
        let cell_size = (1u64 << level) as f32;

        // 2. Divide by cell_size and floor to find the grid index
        // .floor() is critical to handle negative coordinates correctly!
        let gx = (origin.x / cell_size).floor() as i32;
        let gy = (origin.y / cell_size).floor() as i32;

        TileKey {
            level: level as u8,
            x: gx,
            y: gy,
        }
    }

    ////////////////////////////////////////////////////////////////////////////////////////

    /// Returns the (min, max) grid coordinates as IVec2s for the given AABB and level.
    /// Includes the 1-tile padding for broad-phase safety.
    pub fn get_tile_range(min_aabb: Vec2, max_aabb: Vec2, level: usize) -> (IVec2, IVec2) {
        let cell_size = Self::get_level_size(level);

        // Use floor to ensure consistent behavior in negative coordinates
        let min_g = (min_aabb / cell_size).floor().as_ivec2() - IVec2::ONE;
        let max_g = (max_aabb / cell_size).ceil().as_ivec2() + IVec2::ONE;

        // WARN: when looping do not use min..=max, use min..max instead
        (min_g, max_g)
    }

    /// Returns the level of the smallest tile that can fit
    /// 8 circles of this radius arranged in a 2x2 grid.
    pub fn get_level(radius: f32) -> usize {
        // The required tile diameter is 4 times the radius.
        let required_width = radius * 4.0;

        if required_width <= 1.0 {
            return 0;
        }

        // Smallest power of two that fits the required width
        required_width.log2().ceil() as usize
    }

    // Returns the tile square side dimension
    pub fn get_level_size(level: usize) -> f32 {
        (2.0_f32).powi(level as i32)
    }

    ////////////////////////////////////////////////////////////////////////////////////////

    pub fn get_level_mask(&self) -> LevelMask {
        self.active_levels
    }
}
