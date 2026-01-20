# Representing Component Relationships
> some concepts laid out for future reference

## Entity Relationships

### Hierarchy (Tree)
- **What it is:** A recursive structure where transforms propagate (Parent -> Child).
- **Why ALL major engines use it:**
  - **Tooling/UX:** Intuitive for Designers (Drag-and-drop).
  - **Visuals:** Essential for Scene Graphs and Skeletal Animation.
- **Benefits:**
  - **Recursive Despawn:** Delete parent -> Engine automatically deletes children.
  - **Automatic Transform:** Child moves with parent automatically.
- **Disadvantages:**
  - **Random Memory Access:** Pointer chasing causes cache misses.
  - **Rigidity:** Hard to detach a child (e.g., dropping a weapon) without reparenting logic.

### Flat (Relational / Database)
- **What it is:** Data-Oriented Design. Entities are independent; linked only by ID fields.
- **Benefits:**
  - **Contiguous Arrays:** Linear iteration (CPU cache friendly).
  - **Multithreading:** Easy to parallelize (no parent/child read/write conflicts).
  - **Flexibility:** Relationships are just data. A "Turret" can easily become a static building by removing the `Attached` component.
- **Disadvantages:**
  - **"The Wall":** Extremely difficult to do complex Skeletal Animation/IK chains.
  - **Manual Cleanup:** You must write systems to clean up "dependent" entities when the "owner" dies.

## Mitigations / Patterns
> Strategies to manage relationships without a Scene Graph

### Order of Execution
- **Velocity**: Apply physics to all agents (Parents move here).
- **Spatial Sync**: Snap Children to the new Parent positions.
- **Signal Sync**: Update SignalField with the final positions of everyone.
- **Lifecycle**: Cleanup dead entities.

### 1. The Attachment Pattern (Spatial Sync)
* **Concept:** The "child" actively looks up and mimics the "parent's" position.
* **Implementation:**
    * **Component:** `Attached { target: Entity, local_offset: Vec2 }`
    * **System:** Runs after movement. Overwrites child's `Transform` (`pos = parent_pos + rotated_offset`).
* **Why use it:** Decouples movement from ownership. Easy to "detach" items (drop a weapon) by removing the component.

### 2. The Owner/Dependent Pattern (Lifecycle Sync)
* **Concept:** Solves the "Orphan Problem" (e.g., Tank dies, Turret stays floating).
* **Implementation:**
    * **Component:** `Dependent { owner: Entity }`
    * **System:** Checks if `owner` exists. If missing/dead, despawn the dependent entity.
* **Why use it:** Mimics recursive despawn without tree overhead.

### 3. State Propagation (Visibility/Activity Sync)
* **Concept:** Ensures children respect the Parent's state (e.g., hiding Player hides the Gun).
* **Implementation:**
    * **Logic:** Do not duplicate state on the child. Instead, relevant systems (Render/AI) must check the Parent's state before processing the Child.
    * **Code:** `if !world.get::<&Visible>(attached.target).0 { return; }`
* **Why use it:** Prevents visual bugs where "invisible" entities still have visible attachments or active behaviors.

### 4. Fire & Forget (Transient Objects)
* **Concept:** Objects spawned with no persistent link to their creator.
* **Use Case:** Footsteps, Gunshots, Blood Splatters.
* **Implementation:** Spawn with a `Lifetime` component. Do **not** store their ID on the creator.
* **Why use it:** Prevents bloat. The player shouldn't track the 50 footsteps they left behind.


