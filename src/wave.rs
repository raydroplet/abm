// wave.rs

use bitvec::prelude::*;
use glam::Vec2;
use rustc_hash::FxHashMap;
use slotmap::{SlotMap, new_key_type};

// ==================================================================================
// 1. DATA STRUCTURES
// ==================================================================================

pub type SignalMask<const N: usize = 1> = BitArray<[u64; N]>;
pub type LayerMask<const N: usize = 1> = BitArray<[u64; N]>;
type Bucket<const N: usize = 1> = Vec<(SignalKey, SignalMask)>; // The Bucket: A list of (ID, Mask) tuples
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
pub struct Signal {
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
}

impl SignalField {
    pub fn new() -> Self {
        Self {
            store: SlotMap::with_key(),
            grid: FxHashMap::default(),
            active_levels: LayerMask::default(),
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
            ..
        } = self;

        // 2. Insert into storage (Moves 'signal', so we can't use it afterwards)
        let key = store.insert(signal);

        // 3. Read the signal BACK from the store to index it
        // We can do this because 'store' and 'grid' are now separate references.
        // We use the reference inside 'store' so we don't need to clone anything.
        if let Some(stored_signal) = store.get(key) {
            Self::internal_add(grid, active_levels, key, stored_signal);
        }

        key
    }

    /// DELETE: Removes existing Key
    pub fn cease(&mut self, key: SignalKey) {
        // 1. Destructure Self
        // We don't need active_levels for removal
        let Self { store, grid, .. } = self;

        // 2. Look up the signal to see where it was
        if let Some(sig) = store.get(key) {
            // 3. Remove from Grid (Using specific fields, not the whole struct)
            // matching the signature of internal_remove(grid, key, radius, origin)
            Self::internal_remove(grid, key, sig.outer_radius, sig.origin);
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
        Self::internal_remove(grid, key, signal.outer_radius, signal.origin);

        // 4. Update the Signal (In Place)
        // No cloning needed. We are writing directly to the heap memory.
        signal.origin = new_pos;
        signal.outer_radius = new_radius;

        // 5. Add to NEW grid
        // We pass the updated 'signal' reference.
        Self::internal_add(grid, active_levels, key, signal);
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

    // WARN: there is currently two approaches: 1. write in multiple (4) cells so a signal at the
    // edge can always be read for a point query; or 2. each signal is unique to a chunk but we
    // increase the range of search (for a point it's 1 cell + it's 8 neighbors), for a volume the
    // area just increases by 1 in all directions
    //
    // WARN: this decision boils down to what we wish to optimize, reads or writes. I imagine most
    // of the signals move in the world without always being perceived due to the multi level
    // sparse signal field, so optimizing writes may be a wise choice.
    pub fn scan_volume(
        &self,
        min: Vec2,
        max: Vec2,
        signal_mask: SignalMask,
        layer_mask: LayerMask,
        mut callback: impl FnMut(&Signal),
    ) {
        let mut scanning = self.active_levels & layer_mask;

        for level in scanning.iter_ones() {
            //
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
                        level,
                        x: gx,
                        y: gy,
                    }) {
                        for (key, sig_mask) in bucket {
                            // Quick Mask Filter
                            if (*sig_mask & *signal_mask).any() {
                                if let Some(sig) = self.store.get(*key) {
                                    // 3. Precise Collision Check
                                    // We use AABB vs Circle here to be safe
                                    // if self.check_aabb_circle_collision(min, max, sig) {
                                    // WARN: is this bounds cheking really necessary?
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

    fn internal_remove(grid: &mut SpatialGrid, key: SignalKey, outer_radius: f32, origin: Vec2) {
        let level = Self::get_level(outer_radius);
        let bounds = Self::get_bounds(origin, outer_radius, level);

        for x in bounds.0..=bounds.1 {
            for y in bounds.2..=bounds.3 {
                if let Some(bucket) = grid.get_mut(&TileKey { level, x, y }) {
                    if let Some(idx) = bucket.iter().position(|(k, _)| *k == key) {
                        bucket.swap_remove(idx);
                    }
                }
            }
        }
    }

    fn internal_add(
        grid: &mut SpatialGrid,
        active_levels: &mut LayerMask,
        key: SignalKey,
        sig: &Signal,
    ) {
        let level = Self::get_level(sig.outer_radius);
        // *active_levels |= 1 << level; // old code
        active_levels.set(level, true);

        let bounds = Self::get_bounds(sig.origin, sig.outer_radius, level);
        let mask = sig.mask;

        for x in bounds.0..=bounds.1 {
            for y in bounds.2..=bounds.3 {
                grid.entry(TileKey { level, x, y })
                    .or_default()
                    .push((key, mask));
            }
        }
    }

    fn check_collision(&self, pos: Vec2, sig: &Signal) -> bool {
        let dx = pos[0] - sig.origin[0];
        let dy = pos[1] - sig.origin[1];
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

    // this can be severely optimized, but it will stay like that until needed
    fn get_level(radius: f32) -> usize {
        if radius < 1.0 {
            return 0;
        }
        let shift = radius.log2() as usize;
        if shift > 63 { 63 } else { shift }
    }

    fn get_bounds(origin: Vec2, radius: f32, level: u64) -> (i32, i32, i32, i32) {
        let cell_size = (1 << level) as f32;
        (
            ((origin[0] - radius) / cell_size).floor() as i32,
            ((origin[0] + radius) / cell_size).floor() as i32,
            ((origin[1] - radius) / cell_size).floor() as i32,
            ((origin[1] + radius) / cell_size).floor() as i32,
        )
    }
}
