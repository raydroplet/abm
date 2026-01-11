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
pub type SignalMask<const N: usize = 1> = BitArray<[u64; N]>;
pub type LayerMask<const N: usize = 1> = BitArray<[u64; N]>;
type Bucket<const N: usize = 1> = SmallVec<[(SignalKey, SignalMask); 4]>; // The Bucket: A list of (ID, Mask) tuples
type SpatialGrid<const N: usize = 1> = FxHashMap<TileKey, Bucket>; // The Grid: Map of Coordinates -> Bucket

new_key_type! { pub struct SignalKey; }

// level: how big is the tile? are we looking at a 1km square or a 1m square?
// x,y: grid positions for this tile
//
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
struct TileKey {
    level: u8,
    x: i32,
    y: i32,
}

// CONST GENERIC SIGNAL
// N = Number of BYTES (u8).
// Signal<10> = 10 bytes = 80 bits.
#[derive(Clone, Debug)]
pub struct Signal<const N: usize = 1> {
    pub entity: Entity,
    pub origin: Vec2, // WARN: f32 can't represent the level based positions of the signal field.
    pub outer_radius: f32,
    pub inner_radius: f32,
    pub intensity: f32,   // How strong?
    pub falloff: f32,     // How fast it fades?
    pub mask: SignalMask, // SignalType bit mask
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

    active_levels: LayerMask,
    level_counts: LevelCount,
}

impl SignalField {
    pub fn new() -> Self {
        Self {
            store: SlotMap::with_key(),
            grid: FxHashMap::default(),
            active_levels: LayerMask::default(),
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
        // 1. SPLIT SELF (The Magic Step)
        // We unpack the struct so we can borrow fields independently.
        let Self {
            store,
            grid,
            active_levels,
            level_counts,
            ..
        } = self;

        // 2. Get the signal (Mutable Borrow of STORE)
        // We can hold this reference because we aren't passing 'store' to the helpers.
        let signal = if let Some(sig) = store.get_mut(key) {
            sig
        } else {
            return;
        };

        // 3. Remove from OLD grid using current data
        // We pass 'grid' separately. 'signal' is still alive and readable.
        Self::internal_remove(
            grid,
            active_levels,
            level_counts,
            key,
            signal.outer_radius,
            signal.origin,
        );

        // 4. Update the Signal (In Place)
        // No cloning needed. We are writing directly to the heap memory.
        signal.origin = new_pos;
        signal.outer_radius = new_radius;

        // 5. Add to NEW grid
        // We pass the updated 'signal' reference.
        Self::internal_add(grid, active_levels, level_counts, key, signal);
    }

    /// READ: The Scan Loop
    pub fn scan_point(
        &self,
        pos: Vec2,
        signal_mask: SignalMask,
        layer_mask: LayerMask,
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
                            if self.check_collision(pos, sig) {
                                callback(sig);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn scan_volume(
        &self,
        min: Vec2,
        max: Vec2,
        query_mask: SignalMask,
        layer_mask: LayerMask,
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
                                    // WARN: box and circles implementaions needed
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

    // =========================================================================
    // PRIVATE HELPERS (The deduplication magic)
    // =========================================================================

    fn internal_remove(
        grid: &mut SpatialGrid,
        active_levels: &mut LayerMask,
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
        active_levels: &mut LayerMask,
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

    fn check_collision(&self, pos: Vec2, sig: &Signal) -> bool {
        let dx = pos[0] - sig.origin.x;
        let dy = pos[1] - sig.origin.y;
        let dist_sq = dx * dx + dy * dy;
        dist_sq <= (sig.outer_radius * sig.outer_radius)
            && dist_sq >= (sig.inner_radius * sig.inner_radius)
    }

    fn check_aabb_circle_collision(&self, min: Vec2, max: Vec2, sig: &Signal) -> bool {
        // Find the point on the AABB closest to the sphere center
        let closest_x = sig.origin[0].clamp(min[0], max[0]);
        let closest_y = sig.origin[1].clamp(min[1], max[1]);

        // Calculate distance from that point to the circle's center
        let dx = sig.origin[0] - closest_x;
        let dy = sig.origin[1] - closest_y;

        let distance_squared = (dx * dx) + (dy * dy);

        // Check if less than radius squared
        distance_squared < (sig.outer_radius * sig.outer_radius)
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

    // this can be severely optimized, but it will stay like that until needed
    fn get_level(radius: f32) -> usize {
        // let r = radius.max(1.0);
        // f32::log2(r) as usize // or: 31 - (r as u32).leading_zeros() as usize

        if radius < 1.0 {
            return 0;
        }
        let shift = radius.log2() as usize;
        if shift > 63 { 63 } else { shift }
    }
}
