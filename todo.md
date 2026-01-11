## Todo

- improvements and fixes
  1. Move away from the software rendering for agents
  2. Implement camera movement and zoom
  3. revise the current wave.rs code

- features
  1. signal occlusion
  2. signal reflection

- advanced
  1. physics (possible, but needs some time to implement)

## Notes
- For camera perception we use **Bounding Volumes**. Each agent that cares about being rendered has a square field encompassing the model entirely. the camera field then perceives those bounding volumes and queries the respective entities associated with them
