// wave.rs

use bitvec::prelude::*;
use glam::{IVec2, Mat2, Vec2};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

const LEVEL_COUNT: usize = 64;
pub type LevelCount = [u32; LEVEL_COUNT];
pub type Mask = BitArray<[u64; 1]>;
pub type Bucket = SmallVec<[(SignalKey, Mask); 4]>; // The Bucket: A list of (ID, Mask) tuples
pub type SpatialGrid = FxHashMap<TileKey, Bucket>; // The Grid: Map of Coordinates -> Bucket

// new_key_type! { pub struct SignalKey; }
// can be swapped for u64 + entity.to_bits() latter to avoid coupling with hecs
// kept like this for convenience and to remember what they key should actually be
pub type SignalKey = hecs::Entity; // we will just use the generational hecs::Entity as the key

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct TileKey {
    // how big is the tile? are we looking at a 1km square or a 1m square?
    level: u8,
    // x, y: grid positions for this tile
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Signal {
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
    pub emit_mask: Mask,
    pub sense_mask: Mask,
}

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

    active_levels: Mask,
    level_counts: LevelCount,
}

impl SignalField {
    pub fn new() -> Self {
        Self {
            // store: SlotMap::with_key(),
            store: FxHashMap::default(),
            grid: FxHashMap::default(),
            active_levels: Mask::default(),
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

    pub fn scan_point(
        &self,
        pos: Vec2,
        signal_mask: Mask,
        mut callback: impl FnMut(&Signal, &hecs::Entity),
    ) {
        let scanning = self.active_levels /* & layer_mask */;

        for level in scanning.iter_ones() {
            let (min_g, max_g) = Self::get_tile_range(pos, pos, level);

            for gx in min_g.x..max_g.x {
                for gy in min_g.y..max_g.y {
                    if let Some(bucket) = self.grid.get(&TileKey {
                        level: level as u8,
                        x: gx,
                        y: gy,
                    }) {
                        for (key, sig_mask) in bucket {
                            if (*sig_mask & signal_mask).any() {
                                if let Some(sig) = self.store.get(key) {
                                    if self.check_intersection_point_circle(pos, sig) {
                                        callback(sig, key);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn scan_range(
        &self,
        min: Vec2,
        max: Vec2,
        query_mask: Mask,
        // layer_mask: Mask,
        mut callback: impl FnMut(&Signal, &hecs::Entity),
    ) {
        let scanning = self.active_levels/*  & layer_mask */;

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

    pub fn scan<'a>(
        &'a self,
        key: SignalKey,
        // layer_mask: Mask,
        mut callback: impl FnMut(&'a Signal, SignalKey),
    ) {
        let scanning = self.active_levels/*  & layer_mask */;
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
                                        callback(target, *key);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn scan_occluded<'a>(
        &'a self,
        key: SignalKey,
        // layer_mask: Mask,
        occlusion_mask: Mask,
        mut callback: impl FnMut(&Signal, SignalKey, Mask),
    ) {
        let view = self.store.get(&key).expect("Invalid key");
        let mut signal_buffer = SmallVec::<[(&'a Signal, SignalKey, f32); 64]>::new();
        //
        let scan_callback = |target: &'a Signal, key| {
            let dist = (target.origin - view.origin).length();
            let edge_dist = dist - target.outer_radius;
            signal_buffer.push((target, key, edge_dist));
        };

        // scan all signals in the view
        self.scan(key, /* layer_mask, */ scan_callback);

        // Sort everything by distance (closest first).
        signal_buffer
            .sort_unstable_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

        // projection and occlusion
        let mut shadow_mask: Mask = BitArray::ZERO;
        for (target, key, _distance) in signal_buffer {
            let target_projection = Self::project_shadow(view, target);
            let visible_bits = target_projection & !shadow_mask;

            // if it's a relevant signal
            if (view.sense_mask & target.emit_mask).any() {
                // if we see it
                if visible_bits.any() {
                    callback(target, key, visible_bits);
                }
            }

            // if it's an occluder, update the shadow
            if (target.emit_mask & occlusion_mask).any() {
                shadow_mask |= target_projection;
            }

            // early exit on full occlusion
            if shadow_mask.all() {
                // println!("shadow_mask.all()");
                break;
            }
        }
    }

    //=================
    // PRIVATE HELPERS

    fn internal_remove(
        grid: &mut SpatialGrid,
        active_levels: &mut Mask,
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
        active_levels: &mut Mask,
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

    pub fn check_intersection_point_circle(&self, point: Vec2, target_circle: &Signal) -> bool {
        // 1. Calculate squared distance (glam handles the x/y math for you)
        let dist_sq = point.distance_squared(target_circle.origin);

        // 2. Squared radius
        let radius_sq = target_circle.outer_radius * target_circle.outer_radius;

        // 3. Compare
        dist_sq <= radius_sq
    }

    // NOTE: fully implemented by hand. you may as well not go into the trouble of touching it again
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
        // It's better to ^2 the radius sum than to calculate a sqrt on the distance squared

        let v = viewer_cone;
        let t = target_circle;
        let v_o = v.origin;
        let t_o = t.origin;
        let distance_squared =
            (v_o.x - t_o.x) * (v_o.x - t_o.x) + (v_o.y - t_o.y) * (v_o.y - t_o.y);

        let hide_limit = v.outer_radius + t.outer_radius;
        let hide_limit_sq = hide_limit * hide_limit;
        if distance_squared > hide_limit_sq {
            return false; // they can't possibly intersect
        }

        // also handle the inner radius
        // the max(0.0) is to avoid negative squares, which causes unwanted intersetion behavior
        let hide_limit = (v.inner_radius - t.outer_radius).max(0.0);
        let hide_limit_sq = hide_limit * hide_limit;

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

        // NOTE:
        // those edge vectors only change when the view cone mutates. calculating them externally,
        // possibly caching it, and passing it to this function would be the most optimal approach
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

    // WARN: untested
    fn project_shadow(view: &Signal, target: &Signal) -> Mask {
        // 1. define the vector from the viewer to the target.
        let to_target = target.origin - view.origin;

        // 2. calculate exact position (in angles) along the arc the signal center falls in
        //
        //    in short, if we draw a line from the center point of our target onto the view arc
        //    in which point would it fall it we consider the right edge to be angle 0?
        //
        //    change of basis:
        //      atan2 gives an angle starting from the x axis, so we need to shift our basis.
        //      the forward center vector becomes our x and it's perpendicular vector becomes our y
        //
        //      x -> project the target onto the forward direction
        //      y -> project the target onto the perpendicular vector of the forward direction
        //
        //      x = dot(target, direction)
        //      y = dot(target, perpendicular(direction))
        //
        //    atan2 -> the range is strictly −π to +π (−180∘ to +180∘).
        //      [0,  1] -> +π/2
        //      [0, -1] -> -π/2
        //
        //    we also need to add an offset to consider the edge not the center direction
        //    angle = atan2(y, x) + viewer_half_angle

        let perp_dir = Vec2::new(-view.unit_direction.y, view.unit_direction.x);
        let x = to_target.dot(view.unit_direction);
        let y = to_target.dot(perp_dir);
        let angle = y.atan2(x) + (view.angle_radians * 0.5);

        // 3. calculate the width of our signal
        //
        //    we know the center point in angles, but not how large the coverage actually is
        //
        //    given a right angled triangle
        //      hypotenuse -> length(viewer origin, target center)
        //      adjacent -> length(viewer origin, signal edge)
        //      side -> target radius
        //
        //    by SOHCAHTOA
        //      sin(theta) = opposite/hypotenuse = radius/target_center_length
        //
        //    therefore:
        //      theta = arcsin(radius/target_center_length)
        //      angle_range_min = angle - theta
        //      angle_range_max = angle + theta

        let coverage = target.outer_radius / to_target.length().max(1e-5);

        if coverage >= 1.0 {
            // println!("coverage {} >= 1", coverage);
            let mut mask = Mask::ZERO;
            mask.fill(true);
            return mask;
        }

        let theta = (coverage).asin();
        let angle_range_min = angle - theta;
        let angle_range_max = angle + theta;

        // 4. Simply map the range onto our bitmask and return it

        // transforms the [0, viewer_angle] range into [0, 63]
        let scale = 64.0 / view.angle_radians;
        let bit_min = (angle_range_min * scale).floor() as i32;
        let bit_max = (angle_range_max * scale).ceil() as i32 - 1;

        let bit_min = bit_min.max(0);
        let bit_max = bit_max.min(63);

        let mut mask = BitArray::<[u64; 1], Lsb0>::ZERO;
        if bit_min <= bit_max {
            mask[bit_min as usize..=bit_max as usize].fill(true);
        }

        mask
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

    pub fn get_level_mask(&self) -> Mask {
        self.active_levels
    }
}
