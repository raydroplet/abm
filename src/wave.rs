// wave.rs

use bitvec::prelude::*;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use slotmap::{SlotMap, new_key_type};

// ==================================================================================
// 1. DATA STRUCTURES
// ==================================================================================

pub type SignalMask<const N: usize = 1> = BitArray<[u8; N]>;
type Bucket<const N: usize> = Vec<(SignalKey, SignalMask<N>)>; // The Bucket: A list of (ID, Mask) tuples
type SpatialGrid<const N: usize> = FxHashMap<TileKey, Bucket<N>>; // The Grid: Map of Coordinates -> Bucket

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
    pub origin: [f32; 2],
    pub outer_radius: f32,
    pub inner_radius: f32,
    pub intensity: f32,      // How strong?
    pub falloff: f32,        // How fast it fades?
    pub mask: SignalMask<N>, // SignalType bit mask
}

///////////////////
// for the renderer

// Defines the Region we want to render
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

// ==================================================================================
// 2. THE SYSTEM
// ==================================================================================

pub struct SignalLayer<const N: usize = 1> {
    pub store: SlotMap<SignalKey, Signal<N>>,

    // Mask stored as bytes
    // stores signal identifiers (SignalKey) for a specific tile (TileKey)
    //
    // the value includes the SignalType for filtering the
    // ones the agent querying cares about, without having
    // to lookup (memory acess) the actual Signal struct
    //
    grid: SpatialGrid<N>,

    active_levels: u64,
}

impl<const N: usize> SignalLayer<N> {
    pub fn new() -> Self {
        Self {
            store: SlotMap::with_key(),
            grid: FxHashMap::default(),
            active_levels: 0,
        }
    }

    // =========================================================================
    // PUBLIC API
    // =========================================================================

    /// CREATE: Generates a NEW Key
    pub fn emit(&mut self, signal: Signal<N>) -> SignalKey {
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
    pub fn reposition(&mut self, key: SignalKey, new_pos: [f32; 2], new_radius: f32) {
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

    /// READ: The Query Loop
    pub fn query(&self, pos: [f32; 2], query_mask: &BitArray<[u8; N]>) -> Vec<&Signal<N>> {
        let mut results = Vec::new();
        let mut scanning = self.active_levels;

        while scanning > 0 {
            let level = scanning.trailing_zeros() as u8;
            scanning &= !(1 << level);

            let cell_size = (1 << level) as f32;
            let grid_x = (pos[0] / cell_size).floor() as i32;
            let grid_y = (pos[1] / cell_size).floor() as i32;

            if let Some(bucket) = self.grid.get(&TileKey {
                level,
                x: grid_x,
                y: grid_y,
            }) {
                for (key, sig_mask) in bucket {
                    if (*sig_mask & *query_mask).any() {
                        if let Some(sig) = self.store.get(*key) {
                            if self.check_collision(pos, sig) {
                                results.push(sig);
                            }
                        }
                    }
                }
            }
        }
        results
    }

    // =========================================================================
    // PRIVATE HELPERS (The deduplication magic)
    // =========================================================================

    fn internal_remove(
        grid: &mut SpatialGrid<N>,
        key: SignalKey,
        outer_radius: f32,
        origin: [f32; 2],
    ) {
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
        grid: &mut SpatialGrid<N>,
        active_levels: &mut u64,
        key: SignalKey,
        sig: &Signal<N>,
    ) {
        let level = Self::get_level(sig.outer_radius);
        *active_levels |= 1 << level;

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

    fn check_collision(&self, pos: [f32; 2], sig: &Signal<N>) -> bool {
        let dx = pos[0] - sig.origin[0];
        let dy = pos[1] - sig.origin[1];
        let dist_sq = dx * dx + dy * dy;
        dist_sq <= (sig.outer_radius * sig.outer_radius)
            && dist_sq >= (sig.inner_radius * sig.inner_radius)
    }

    // this can be severely optimized, but it will stay like that until needed
    fn get_level(radius: f32) -> u8 {
        if radius < 1.0 {
            return 0;
        }
        let shift = radius.log2() as u8;
        if shift > 63 { 63 } else { shift }
    }

    fn get_bounds(origin: [f32; 2], radius: f32, level: u8) -> (i32, i32, i32, i32) {
        let cell_size = (1 << level) as f32;
        (
            ((origin[0] - radius) / cell_size).floor() as i32,
            ((origin[0] + radius) / cell_size).floor() as i32,
            ((origin[1] - radius) / cell_size).floor() as i32,
            ((origin[1] + radius) / cell_size).floor() as i32,
        )
    }

    // NEW: The Bridge function
    pub fn query_snapshot(&self, view: Viewport, layer_mask: &SignalMask<N>) -> Vec<Signal<N>> {
        // 1. DEDUPLICATION SET
        // Signals exist in multiple tiles. We use this to ensure we don't draw them twice.
        let mut visited = FxHashSet::default();
        let mut results = Vec::new();

        // 2. Iterate only ACTIVE spatial levels (Optimization)
        let mut scanning = self.active_levels;
        while scanning > 0 {
            let level = scanning.trailing_zeros() as u8;
            scanning &= !(1 << level);

            let cell_size = (1 << level) as f32;

            // 3. Calculate which Grid Tiles the Camera sees (The Region)
            let min_x = (view.min[0] / cell_size).floor() as i32;
            let max_x = (view.max[0] / cell_size).floor() as i32;
            let min_y = (view.min[1] / cell_size).floor() as i32;
            let max_y = (view.max[1] / cell_size).floor() as i32;

            // 4. Scan those tiles
            for x in min_x..=max_x {
                for y in min_y..=max_y {
                    if let Some(bucket) = self.grid.get(&TileKey { level, x, y }) {
                        for (key, sig_mask) in bucket {
                            // CHECK 1: Is this the "Layer of Choice"?
                            // bitvec .any() checks if any shared bits are set
                            if (*sig_mask & *layer_mask).any() {
                                // CHECK 2: Have we already drawn this signal?
                                if visited.insert(*key) {
                                    if let Some(sig) = self.store.get(*key) {
                                        // Optional: Check if sig is actually on screen
                                        // (Simple circle-rect collision)
                                        results.push(sig.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        results
    }
}
