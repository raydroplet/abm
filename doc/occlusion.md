# occlusion

Occlusion refers to the situation where one object blocks another from the viewer’s perspective, making the hidden object partially or fully invisible. Our current signal field allows for efficient signal representation and querying, but it lacks the properties of being blocked, like light being blocked by walls.

The current architecture is entirely made to answer the question: "what is affecting me?". We have a point or an area and we query the signals that affect those. This is efficient because of our multi level system, but when we do occlusion the goal is inverted: we know a field is affecting us but we don't know what is in it's path that could block the signal. The question then becomes "what is being affected?" for the ray going in the direction to our signal source. 

This is a problem because the SignalField can't do this, but in truth it actually does. We simply create a ephemeral field in the direction of the source signal using a Rectangular pyramid frustum, specialized for a scan only. Since we can filter by tags we simply specify the occlusion mask for the query. If any result returns we do a more expensive intersection calculation, knowing if we are in a shadow or not.

# The Pipeline
- There's a signal with a tag that enables occlusion, it has a certain range and can be circular, radial, boxed, etc.
- For every agent we query the signals interacting with us for our bounding volume (likely a box or a circle)
- For every signal that may be occluded, first we determine the signal source and the projected line onto our bounding volume (a projection line of sorts). This line represents how the signal can "touch" our bounds.
- We then cast a ephemeral signal querying for any field occluders. this signal is pyramid shaped with the tip at the signal and the base being our projection line
- Every of those signals has a bounding volume, we project it onto our line to see what's occluded. We repeat this process for every occluder that we have found
- Finally, we end up with a projecting of which sections of the signal actually hit our volume

## Features
- Perception: Signal-Signals, **Point-Signals**, **Area-Signals**
  - we can query cell regions, but not the signals intersecting a signal area. useful, not only in this specific case. realistically needed for proper agent perception
- Fields
  - **Shapes**: Square, **Circular**, Cone (but for 2d)
    - > squares do not support occlusion, ideal for bounding volumes
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
