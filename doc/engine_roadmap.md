## Phase 1: The Logic Prototyping (2D)
> The goal is to validate the engine features in the simplest environment possible, following an incremental iteractive software development approach

### ~~0.1 (canvas)~~
- core: `eframe`
- features
  - windowing
  - render loop

### ~~0.2 (agents)~~
- core: `hecs`
- features:
  - agents
  - stress test
- demo: particle bouncing

### 0.3 (waves)
- features
  - wave propagation
  - influece fields
  - agent interactions
    - waves: reflection, occlusion
    - logic: field perception and decision making
    - rotation
- demo: the dark room
  - agents are blind. you place "sound sources" (clicks).
  - agents flow like water toward the sound. Walls block the sound (Shadow casting).

### 0.4 (control)
- features
  - agent decisions based on field input
  - the director (as a narrative control)
  - fate tokens and influence field interactions
  - abm logic prototypes
- demo: hunting field
  - carrots, rabbits, wolves and predation
  - population and interactions influence
  - equilibrium seeking

### 0.5 (aggregation)
- goal: to cluster similar agents together into a single one (eg: rocks -> mountain)
- challenges
  - agent clustering
  - dynamic aggregation and separation
    - which criteria is used for those?
  - wave propagation on clusters
- demo: `to define`

### 0.6 (continuity)
> once we have aggregation, complex shapes emerge that may not map well with the current implementation

- goal: to move away from the simple unity geometry nature of agents into complex aggregate ones
- possible features
  - real-time voxelization
- challenges
  - wave propagation in a aggregate world
  - rotation
  - collisions
    - > the square nature of agents made this problem simple, but now we need a more capable way to check for collisions
- demo: `to define`

### 0.7 (infinite worlds)
> some concepts to be used here have already been layed out but it needs more research and most importantly to have the previous versions implemented. you can't optimize a engine you haven't even written yet
- demo: `to define`

