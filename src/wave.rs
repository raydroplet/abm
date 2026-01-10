// wave.rs

/////////////////////

#[derive(Clone)]
pub struct WaveField {}

impl WaveField {
    pub fn new() -> Self {
        Self {}
    }
}

//////////////////

use bitvec::prelude::*;
use rustc_hash::FxHashMap;
use slotmap::{SlotMap, new_key_type};

// ==================================================================================
// 1. DATA STRUCTURES
// ==================================================================================

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
pub struct Signal<const N: usize> {
    pub origin: [f32; 2],
    pub outer_radius: f32,
    pub inner_radius: f32,

    // CHANGED: Now using an array of bytes [u8; N]
    pub mask: BitArray<[u8; N]>,

    pub strength: f32,
    pub data: u32,
}

// ==================================================================================
// 2. THE SYSTEM
// ==================================================================================

pub struct SignalLayer<const N: usize> {
    pub store: SlotMap<SignalKey, Signal<N>>,

    // Mask stored as bytes
    // stores signal identifiers (SignalKey) for a specific tile (TileKey)
    //
    // the value includes the SignalType for filtering the
    // ones the agent querying cares about, without having
    // to lookup (memory acess) the actual Signal struct
    //
    grid: FxHashMap<TileKey, Vec<(SignalKey, BitArray<[u8; N]>)>>,

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
        // 1. Insert into storage (Heap)
        let key = self.store.insert(signal.clone());

        // 2. Add to spatial index (Reuse logic)
        self.internal_add(key, &signal);

        key
    }

    /// DELETE: Removes existing Key
    pub fn cease(&mut self, key: SignalKey) {
        // 1. CLONE first.
        // We cannot pass `&Signal` to internal_remove directly because getting
        // that reference locks `self`. We clone the data to unlock `self`.
        let signal_copy = if let Some(sig) = self.store.get(key) {
            sig.clone()
        } else {
            return; // Key doesn't exist
        };

        // 2. Now `self` is unlocked. We can call internal methods.
        self.internal_remove(key, &signal_copy);

        // 3. Finally remove from storage
        self.store.remove(key);
    }

    /// MOVE: Keeps Key, Updates Position
    pub fn reposition(&mut self, key: SignalKey, new_pos: [f32; 2], new_radius: f32) {
        // 1. Get OLD state and Clone it
        let mut signal_copy = if let Some(sig) = self.store.get(key) {
            sig.clone()
        } else {
            return;
        };

        // 2. Remove from OLD grid locations using the OLD copy
        self.internal_remove(key, &signal_copy);

        // 3. Update the COPY with new data
        signal_copy.origin = new_pos;
        signal_copy.outer_radius = new_radius;

        // 4. Update the STORE with the new data
        // We re-borrow `store` here, but since step 1 is finished, it's safe.
        if let Some(sig) = self.store.get_mut(key) {
            *sig = signal_copy.clone();
        }

        // 5. Add to NEW grid locations using the NEW copy
        self.internal_add(key, &signal_copy);
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

    fn internal_add(&mut self, key: SignalKey, sig: &Signal<N>) {
        let level = self.get_level(sig.outer_radius);
        self.active_levels |= 1 << level;

        let bounds = self.get_bounds(sig.origin, sig.outer_radius, level);
        let mask = sig.mask; // Copy the bitmask

        for x in bounds.0..=bounds.1 {
            for y in bounds.2..=bounds.3 {
                self.grid
                    .entry(TileKey { level, x, y })
                    .or_default()
                    .push((key, mask));
            }
        }
    }

    fn internal_remove(&mut self, key: SignalKey, sig: &Signal<N>) {
        let level = self.get_level(sig.outer_radius);
        let bounds = self.get_bounds(sig.origin, sig.outer_radius, level);

        for x in bounds.0..=bounds.1 {
            for y in bounds.2..=bounds.3 {
                if let Some(bucket) = self.grid.get_mut(&TileKey { level, x, y }) {
                    if let Some(idx) = bucket.iter().position(|(k, _)| *k == key) {
                        bucket.swap_remove(idx);
                    }
                }
            }
        }
    }

    // Common Math
    fn get_bounds(&self, origin: [f32; 2], radius: f32, level: u8) -> (i32, i32, i32, i32) {
        let cell_size = (1 << level) as f32;
        (
            ((origin[0] - radius) / cell_size).floor() as i32, // min_x
            ((origin[0] + radius) / cell_size).floor() as i32, // max_x
            ((origin[1] - radius) / cell_size).floor() as i32, // min_y
            ((origin[1] + radius) / cell_size).floor() as i32, // max_y
        )
    }

    fn get_level(&self, radius: f32) -> u8 {
        if radius < 1.0 {
            return 0;
        }
        let shift = radius.log2() as u8;
        if shift > 63 { 63 } else { shift }
    }

    fn check_collision(&self, pos: [f32; 2], sig: &Signal<N>) -> bool {
        let dx = pos[0] - sig.origin[0];
        let dy = pos[1] - sig.origin[1];
        let dist_sq = dx * dx + dy * dy;
        dist_sq <= (sig.outer_radius * sig.outer_radius)
            && dist_sq >= (sig.inner_radius * sig.inner_radius)
    }
}
