1. ~~extend the sphere base signal into an arc sector~~
2. ~~signal-to-signal perception~~
  - ~~simply a scan function specific to the signal shape~~

## The Idea
- Agent checks relevant signals in it's bounding volume
- For any of those that may have the occluded tag, beam cast an arc sector from the signal source point into the bounding volume projection line. cast the shadow/light onto said line, possibly optimizing along the way if portions of the signal are blocked.


---------------

## Todo

- improvements and fixes
  1. ~~Move away from the software rendering for agents~~
  2. ~~Implement camera movement and zoom~~ _(not now)_
  3. ~~revise the current wave.rs code~~

- features
  1. signal occlusion (_ongoing_)
  2. ~~signal reflection~~ (_unplanned_)

- advanced
  1. physics (possible, but needs some time to implement)

## Notes
- For camera perception we use **Bounding Volumes**. Each agent that cares about being rendered has a square field encompassing the model entirely. the camera field then perceives those bounding volumes and queries the respective entities associated with them
