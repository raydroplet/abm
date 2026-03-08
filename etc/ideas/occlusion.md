# Occlusion
Occlusion refers to the situation where one object blocks another from the viewer’s perspective, making the hidden object partially or fully invisible. Our current signal field allows for efficient signal representation and querying, but it lacks the properties of being blocked, like light being blocked by walls.

The current architecture is entirely made to answer the question: "what is affecting me?". We have a point or an area and we query the signals that affect those. This is efficient because of our multi level system, but when we do occlusion the goal is inverted: we know a field is affecting us but we don't know what is in it's path that could block the signal. The question then becomes "what is being affected?" for the ray coming from our signal source. 

This is a problem because the SignalField can't do this, but in truth it actually does. We simply create a ephemeral scan field in the direction from the source signal using a view cone, specialized for a scan only. Since we can filter by tags we simply specify the occlusion mask for the query. If any result returns we do a more expensive intersection calculation, knowing if we are in a shadow or not.

## Passive Perception
- There's a signal with a tag that enables occlusion, it is (for now) a sphere of a certain radius.
- For every agent we query the signals interacting with us for our bounding volume (also a sphere)
- For every signal that may be occluded, first we determine the signal source and the projected line onto our bounding volume (considering it's angle). This line represents how the signal can reach our bounds.
- We then cast a ephemeral signal querying for any field occluders. this signal is cone shaped with the tip at the signal and the base being our projection line
- Every of those signals has a bounding volume, we project it onto our line to see what's occluded. We repeat this process for every occluder that we have found
- Finally, we end up with a projecting of which sections of the signal actually hit our volume

## Active Perception
- We have a vision cone
- We query the signals that intersects it, sorting them by distance
- For every one we project them onto our line, handling occluders in the path
- We end up with a mask projection for every relevant signal in the path, while also handling occlusion

## Features
- Perception: Signal-Signals, **Point-Signals**, **Area-Signals**
  - we can query cell regions, but not the signals intersecting a signal area. useful, not only in this specific case. realistically needed for proper agent perception
- Fields
  - **Shapes**: **Circular**, Cone (but for 2d)
    <!-- - > squares do not support occlusion, ideal for bounding volumes -->
  - **Types**:
    - Enduring: reading, readable
    - Ephemeral: reading only
      - > should not be inserted on the main signal storage, only passed to the scan functions
- Debugging: visualization tools

## Details
- The projection line is represented as a bitmask (u64 or u128), the bits indicates which sections are being occluded or not. this is necessary for viable performance. the alternative is a `Vec<range>` with `range` being a pair of floating values. For higher levels 

## Extensions
- Model occlusion
  - We can project the bounding box of the occluders onto the line, but what if they encompass a model? we just add another bit flag, the pipeline checks it at the end, if it's enabled it runs a narrow check on the geometry of said entity.

# Technical Specification
We have the entities we are searching for and the entities that may occlude them
The signature looks like `scan_cone_occluded(&self, cone_information, callback(&Signal, bitmask: f32))`
So for each relevant signal found the callback receives it and the occlusion bitmask

### Algorithm
1. Tile Collection Phase
  - Iterate all active_levels
  - For each level, calculate the grid range min_x..max_x that completely encloses our vision cone
    - the range is expanded by -1/+1 on all sides (min_x - 1 to max_x + 1). This captures neighbor tiles whose signals might bleed into our view.
  - Push every valid coordinate into a single flat list: `Vec<(Distance, TileKey)>`
    - Distance Formula: `dist_sq_to_aabb(player_pos, tile_aabb) - max_radius(level)`
    - The distance for each tile is calculated from the edge (of the tile), not the center
    - `max_radius(level)` changes the sorting approach from which tile is closest to which potential occluder is closest, leveraging the 2^N level division we have.
2. Sort phase
  - sort the tiles by distance
  - This intermingles the levels, they are sorted by distance instead
  - we don't short considering the center, but the edge of the tile instead
3. Execution phase
  - fully occluded: `if mask.is_full() then return`
  - tile cull: `if visibility.is_occluded(tile_aabb) then continue`
    - project the entire tile dimension (square) onto a depth mask
    - compare it to my current occluded mask and see if any points are visible
    - if the tile is fully occluded, simply skip all subsequent checks
  - bucket sort:
    - get all signals from the bucket
    - collect them into a small temporary buffer (`SmallVec` is usefl)
    - sort the elements by `(sig.origin - player).len() - radius`
  - processing
    - iterate the sorted signals
    - `if Signal == Wall then visibility.add_occluder(sig)`
    - `if Signal == Entity then { mask = visibility.calc(sig); callback(sig, mask)`

