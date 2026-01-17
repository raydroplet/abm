// wave.rs

use bitvec::prelude::*;
use glam::{/* DVec2, */ Vec2};
use hecs::Entity;
use rustc_hash::FxHashMap;
use slotmap::{SlotMap, new_key_type};
use smallvec::SmallVec;

// ==================================================================================
// 1. DATA STRUCTURES
// ==================================================================================

const LEVEL_COUNT: usize = 64;
pub type LevelCount<const N: usize = 1> = [u32; LEVEL_COUNT];
pub type ShadowMask<const N: usize = 1> = BitArray<[u64; N]>;
pub type SignalMask<const N: usize = 1> = BitArray<[u64; N]>;
pub type OcclusionMask<const N: usize = 1> = BitArray<[u64; N]>;
pub type LevelMask<const N: usize = 1> = BitArray<[u64; N]>;
pub type Bucket<const N: usize = 1> = SmallVec<[(SignalKey, SignalMask); 4]>; // The Bucket: A list of (ID, Mask) tuples
pub type SpatialGrid<const N: usize = 1> = FxHashMap<TileKey, Bucket>; // The Grid: Map of Coordinates -> Bucket

new_key_type! { pub struct SignalKey; }

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
#[derive(Clone, Debug)]
pub struct Signal<const N: usize = 1> {
    // source
    pub origin: Vec2,
    pub direction: Vec2,
    // shape
    pub outer_radius: f32,
    pub inner_radius: f32,
    pub angle_cos: f32,
    // force
    pub intensity: f32, // How strong?
    pub falloff: f32,   // How fast it fades?
    // data
    pub entity: Entity,
    pub mask: SignalMask, // SignalType bit mask
}

impl Signal {
    pub fn new_sphere(origin: Vec2, radius: f32, mask: SignalMask, entity: Entity) -> Self {
        Self {
            origin,
            direction: Vec2::X, // Arbitrary valid vector
            outer_radius: radius,
            inner_radius: 0.0,
            angle_cos: -1.0, // Optimization: Skips angle math
            intensity: 1.0,
            falloff: 0.1,
            mask: mask,
            entity: entity,
        }
    }
}

// ==================================================================================
// 2. THE SYSTEM
// ==================================================================================

pub struct SignalField {
    pub store: SlotMap<SignalKey, Signal>,

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
            store: SlotMap::with_key(),
            grid: FxHashMap::default(),
            active_levels: LevelMask::default(),
            level_counts: [0; LEVEL_COUNT],
        }
    }

    // =========================================================================
    // PUBLIC API
    // =========================================================================

    /// CREATE: Generates a NEW Key
    pub fn emit(&mut self, signal: Signal) -> SignalKey {
        // 1. Destructure Self (Split Borrows)
        let Self {
            store,
            grid,
            active_levels,
            level_counts,
            ..
        } = self;

        // 2. Insert into storage (Moves 'signal', so we can't use it afterwards)
        let key = store.insert(signal);

        // 3. Read the signal BACK from the store to index it
        // We can do this because 'store' and 'grid' are now separate references.
        // We use the reference inside 'store' so we don't need to clone anything.
        if let Some(stored_signal) = store.get(key) {
            Self::internal_add(grid, active_levels, level_counts, key, stored_signal);
        }

        key
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
        if let Some(sig) = store.get(key) {
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
        store.remove(key);
    }

    /// MOVE: Keeps Key, Updates Position
    pub fn reposition(&mut self, key: SignalKey, new_pos: Vec2, new_radius: f32) {
        // 1. SPLIT SELF
        let Self {
            store,
            grid,
            active_levels,
            level_counts,
            ..
        } = self;

        // 2. Get the signal (Mutable Borrow of STORE)
        let signal = if let Some(sig) = store.get_mut(key) {
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
        let new_tile = Self::get_coordinates(new_radius, new_pos);

        // If the agent moved, but didn't cross a grid boundary, we skip the HashMap thrashing.
        // The Grid points to the SignalKey, and the SignalKey is still valid.
        // We only update the raw data in 'store'.
        if old_tile == new_tile {
            signal.origin = new_pos;
            signal.outer_radius = new_radius;
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
        signal.outer_radius = new_radius;

        // 5. Add to NEW grid
        // internal_add calculates the key based on the *current* signal state, which we just updated.
        Self::internal_add(grid, active_levels, level_counts, key, signal);
    }

    //////////
    /// Scan

    /// READ: The Scan Loop
    pub fn scan_point(
        &self,
        pos: Vec2,
        signal_mask: SignalMask,
        layer_mask: LevelMask,
        mut callback: impl FnMut(&Signal),
    ) {
        let scanning = self.active_levels & layer_mask;

        for level in scanning.iter_ones() {
            // let level = scanning.trailing_zeros() as u8;
            // scanning &= !(1 << level);

            let cell_size = (1 << level) as f32;
            let grid_x = (pos[0] / cell_size).floor() as i32;
            let grid_y = (pos[1] / cell_size).floor() as i32;

            if let Some(bucket) = self.grid.get(&TileKey {
                level: level as u8,
                x: grid_x,
                y: grid_y,
            }) {
                for (key, sig_mask) in bucket {
                    if (*sig_mask & signal_mask).any() {
                        if let Some(sig) = self.store.get(*key) {
                            if self.check_intersection_point(pos, sig) {
                                callback(sig);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn scan_volume_rectangle(
        &self,
        min: Vec2,
        max: Vec2,
        query_mask: SignalMask,
        layer_mask: LevelMask,
        mut callback: impl FnMut(&Signal),
    ) {
        let scanning = self.active_levels & layer_mask;

        for level in scanning.iter_ones() {
            // let level = scanning.trailing_zeros() as u8;
            // scanning &= !(1 << level);

            let cell_size = (1 << level) as f32;

            // 1. Calculate the range of tiles this Volume touches
            // We PAD the search by -1/+1 because a signal in a neighbor cell
            // might have a radius that reaches into this volume.
            let min_grid_x = ((min[0] / cell_size).floor() as i32) - 1;
            let max_grid_x = ((max[0] / cell_size).floor() as i32) + 1;
            let min_grid_y = ((min[1] / cell_size).floor() as i32) - 1;
            let max_grid_y = ((max[1] / cell_size).floor() as i32) + 1;

            // 2. Iterate the range (The Volume Loop)
            for gx in min_grid_x..=max_grid_x {
                for gy in min_grid_y..=max_grid_y {
                    if let Some(bucket) = self.grid.get(&TileKey {
                        level: level as u8,
                        x: gx,
                        y: gy,
                    }) {
                        for (key, sig_mask) in bucket {
                            // Quick Mask Filter
                            if (*sig_mask & query_mask).any() {
                                if let Some(sig) = self.store.get(*key) {
                                    // 3. Precise Collision Check
                                    // We use AABB vs Circle here to be safe
                                    // if self.check_aabb_circle_collision(min, max, sig) {
                                    callback(sig);
                                    // }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn scan_volume_cone(
        &self,
        origin: Vec2,
        direction: Vec2,
        angle_cos: f32,
        outer_radius: f32,
        query_mask: SignalMask,
        layer_mask: LevelMask,
        mut callback: impl FnMut(&Signal),
    ) {
        let scanning = self.active_levels & layer_mask;

        // 1. BROAD PHASE: Calculate a loose Bounding Box
        // Optimizing the AABB of a rotated cone is hard.
        // It is much faster to just query the square AABB of the full radius.
        // It over-selects tiles, but the Narrow Phase filters them out cheaply.
        let min_aabb = origin - Vec2::splat(outer_radius);
        let max_aabb = origin + Vec2::splat(outer_radius);

        for level in scanning.iter_ones() {
            let cell_size = (1 << level) as f32;

            // Standard Grid Iteration (Same as scan_aabb)
            let min_gx = (min_aabb.x / cell_size).floor() as i32 - 1;
            let max_gx = (max_aabb.x / cell_size).floor() as i32 + 1;
            let min_gy = (min_aabb.y / cell_size).floor() as i32 - 1;
            let max_gy = (max_aabb.y / cell_size).floor() as i32 + 1;

            for gx in min_gx..=max_gx {
                for gy in min_gy..=max_gy {
                    if let Some(bucket) = self.grid.get(&TileKey {
                        level: level as u8,
                        x: gx,
                        y: gy,
                    }) {
                        for (key, sig_mask) in bucket {
                            if (*sig_mask & query_mask).any() {
                                if let Some(sig) = self.store.get(*key) {
                                    // 2. NARROW PHASE: Cone vs Circle Intersection
                                    if self.check_intersection_cone(
                                        origin,
                                        direction,
                                        angle_cos,
                                        outer_radius,
                                        sig,
                                    ) {
                                        callback(sig);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn scan_cone_occluded(
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
        let mut tile_buffer: SmallVec<[(f32, TileKey); 64]> = SmallVec::new();

        // 1. TILE COLLECTION PHASE
        for level in scanning.iter_ones() {
            let cell_size = (1 << level) as f32;
            let level_bias = cell_size; // Prefer higher levels (larger objects) if distances are equal

            // Get grid range padded by 1 (to catch bleeding signals)
            let min_gx = ((min_scan.x / cell_size).floor() as i32) - 1;
            let max_gx = ((max_scan.x / cell_size).floor() as i32) + 1;
            let min_gy = ((min_scan.y / cell_size).floor() as i32) - 1;
            let max_gy = ((max_scan.y / cell_size).floor() as i32) + 1;

            for gx in min_gx..=max_gx {
                for gy in min_gy..=max_gy {
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
                        if let Some(sig) = self.store.get(*key) {
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
                    if visible_bits.any() && (sig.mask & target_mask).any() {
                        callback(sig, visible_bits);
                    }

                    // 3. Update Occlusion
                    if (sig.mask & occluder_mask).any() {
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
        grid.entry(tile_key).or_default().push((key, signal.mask));
    }

    // fn check_collision_hollow_sphere(&self, pos: Vec2, sig: &Signal) -> bool {
    //     let dx = pos[0] - sig.origin.x;
    //     let dy = pos[1] - sig.origin.y;
    //     let dist_sq = dx * dx + dy * dy;
    //     dist_sq <= (sig.outer_radius * sig.outer_radius)
    //         && dist_sq >= (sig.inner_radius * sig.inner_radius)
    // }

    fn check_intersection_point(&self, target_pos: Vec2, sig: &Signal) -> bool {
        // 1. Vector Calculation
        let to_target = target_pos - sig.origin;
        let dist_sq = to_target.length_squared();

        // 2. Radius Check (Outer & Inner)
        if dist_sq > sig.outer_radius * sig.outer_radius
            || dist_sq < sig.inner_radius * sig.inner_radius
        {
            return false;
        }

        // 3. Angle Check (The optimization)
        // If angle_cos is close to -1.0 (Omni), we skip the dot product entirely.
        if sig.angle_cos > -1.0 {
            // Am I a cone?
            // Safe normalization handles the case where target == origin
            let dir_to_target = to_target.normalize_or_zero();

            // Dot Product: 1.0 = Facing, 0.0 = Side, -1.0 = Behind
            if sig.direction.dot(dir_to_target) < sig.angle_cos {
                // (How aligned are these two arrows) < (Is the alignment worse than my limit?)
                return false;
            }
        }

        true
    }

    fn check_intersection_cone(
        &self,
        origin: Vec2,      // Cone Origin (The Camera/Eye)
        direction: Vec2,   // Cone Direction (Where it looks)
        angle_cos: f32,    // Cone Angle (Field of View)
        outer_radius: f32, // Cone Length (Max View Distance)
        sig: &Signal,      // The Object (The Wall/Target)
    ) -> bool {
        // 1. Vector Calculation: FROM Origin TO Signal
        let to_target = sig.origin - origin;
        let dist_sq = to_target.length_squared();
        let target_radius = sig.outer_radius;

        // 2. Distance Check (Cone Length + Target Radius)
        let max_dist = outer_radius + target_radius;
        if dist_sq > max_dist * max_dist {
            return false;
        }

        // 3. Early Out: Inside the target?
        // If the cone origin (camera) is physically inside the wall, it hits.
        if dist_sq < target_radius * target_radius {
            return true;
        }

        // 4. Angle Check
        if angle_cos > -1.0 {
            let dist = dist_sq.sqrt();
            let dir_to_target = to_target / dist; // Normalize

            // 5. "Fat Cone" Expansion
            let angle_expansion = target_radius / dist;
            let expanded_threshold = angle_cos - angle_expansion;

            if direction.dot(dir_to_target) < expanded_threshold {
                return false;
            }
        }

        true
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

    // fn check_aabb_circle_collision(&self, min: Vec2, max: Vec2, sig: &Signal) -> bool {
    //     // Find the point on the AABB closest to the sphere center
    //     let closest_x = sig.origin[0].clamp(min[0], max[0]);
    //     let closest_y = sig.origin[1].clamp(min[1], max[1]);
    //
    //     // Calculate distance from that point to the circle's center
    //     let dx = sig.origin[0] - closest_x;
    //     let dy = sig.origin[1] - closest_y;
    //
    //     let distance_squared = (dx * dx) + (dy * dy);
    //
    //     // Check if less than radius squared
    //     distance_squared < (sig.outer_radius * sig.outer_radius)
    // }

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

    // this can be severely optimized, but it will stay like that until needed
    fn get_level(radius: f32) -> usize {
        if radius < 1.0 {
            return 0;
        }
        // Use ceil to ensure cell_size > radius
        let shift = radius.log2().ceil() as usize;
        if shift > 63 { 63 } else { shift }
    }

    ////////
    /// ---

    pub fn get_level_mask(&self) -> LevelMask {
        self.active_levels
    }
}
