# Support Mode & Lychee-Style Supports – Development Checklist

> High-level development plan to add a Lychee-like support authoring system to the slicer.
> This is a planning document only (no code).

---

## 0. Foundations & Architecture

This section defines *what a support is* in our slicer and how it fits into the existing scene model. If we get this wrong, everything else (UI, placement, saving/loading) becomes fragile. The goal is to mirror Lychee’s concepts just enough that we can both import/export and build our own tools on top.

- [ ] **0.1 Define core support data structures**
  
  This step designs the in-memory representation of a single support and the collection of all supports in a scene.
  - [ ] Decide where supports live in scene state (e.g. `scene.supports` or per-object `scene.objects[id].supports`).
  - [ ] Define a `SupportInstance` concept (mirrors Lychee):
    - [ ] `id`: unique support ID.
    - [ ] `objectIdTip`: which 3D object the tip attaches to.
    - [ ] `tip`: `{ x, y, z }` world coordinates.
    - [ ] `tipNormal`: `{ x, y, z }` world-space normal at contact.
    - [ ] `base`: `{ x, y, z }` world coordinates.
    - [ ] `baseNormal`: `{ x, y, z }` normal at base contact.
    - [ ] `settings`: geometry sections (see 0.2).
    - [ ] Optional hierarchy: `parentBaseId`, `parentTipId`.
  - [ ] Choose ID generation strategy (incrementing `s1, s2, ...` or UUIDs).
  - [ ] Decide if we want a normalized structure (`byId` + `allIds`) similar to Lychee.

- [ ] **0.2 Define support geometry schema (per instance)**
  
  Here we decide how to break a support into logical sections (tip/shaft/base) and which numeric fields describe each. This mirrors the Lychee `tip` / `mid` / `base` blocks but we keep it as simple as possible for a first implementation.
  - [ ] `settings.tip`:
    - [ ] `type` (start with "cone").
    - [ ] `pointDiameter` (contact size at model).
    - [ ] `diameter` (tip body diameter).
    - [ ] `length` (tip length along support axis).
    - [ ] `penetration` (initially 0 or fixed small value).
    - [ ] `angle` (cone angle; can be a constant initially).
  - [ ] `settings.mid`:
    - [ ] `type` (start with "cylinder").
    - [ ] `diameter` (shaft diameter – we can add this even if Lychee doesn’t store it here explicitly).
  - [ ] `settings.base`:
    - [ ] `type` (e.g. "cylinder").
    - [ ] `diameter` (base disk or column diameter).
    - [ ] `length` (height of base element).
  - [ ] Optional (future): `settings.baseTip`, `settings.extra`, `adaptiveBase`, `isStraight`.

- [ ] **0.3 Decide how supports persist with the scene**
  
  Supports must be saved and restored with the project. This step decides how they are written to and read from our scene file format so that a user can close the app and come back to the exact same supports.
  - [ ] Extend scene serialization to include support instances and their settings.
  - [ ] Decide whether supports are saved per-object or globally with references.
  - [ ] Ensure versioning so older scenes without supports still load.

- [ ] **0.4 Coordinate system & units**
  
  We need to be explicit about units and coordinate space so a support placed at a point on the mesh appears in the same place when reloaded or exported. This also ensures our support geometry lengths match real-world millimeters.
  - [ ] Confirm units (mm) are consistent with existing slicer logic.
  - [ ] Confirm world coordinate system and orientation.
  - [ ] Document how `tip`, `base`, and normals are interpreted when generating geometry.

---

## 1. Application Modes (Prepare vs Support)

The slicer needs two distinct workflows:

- **Prepare mode** for positioning and transforming models.
- **Support mode** for adding/removing supports with a different set of tools.

This section ensures that the rest of the code can reliably know which workflow is active and can show/hide the correct UI and input behavior.

### 1.1 Mode state & transitions

- [ ] **1.1.1 Introduce a global mode flag**
  
  We add a simple, explicit flag in global state so every subsystem (toolbar, sidebar, viewport, hotkeys) can check which mode is active without guessing.
  - [ ] Add `mode: "prepare" | "support"` to global app state.
  - [ ] Default to `"prepare"` when the app loads or a scene is opened.

- [ ] **1.1.2 Mode change APIs**
  
  Mode changes should be done through well-defined functions, not ad-hoc mutations. This makes it easy to hook up UI events and to add side effects (like clearing selections) in one place.
  - [ ] Implement `enterPrepareMode()`:
    - [ ] Update global `mode`.
    - [ ] Notify UI (sidebar, toolbar, viewport tools) of mode change.
    - [ ] Clear any support-specific temporary state (hovered support, pending placement).
  - [ ] Implement `enterSupportMode()`:
    - [ ] Update global `mode`.
    - [ ] Notify UI.
    - [ ] Clear transform-gizmo-specific state.

### 1.2 Toolbar UI

The top bar is how the user discovers and switches between modes. We want a very obvious toggle so it is clear when the user is in Support mode and transform tools are disabled.

- [ ] **1.2.1 Add mode buttons to the top bar**
  - [ ] "Prepare" mode icon/button.
  - [ ] "Support" mode icon/button.
  - [ ] Visual highlight for the active mode.

- [ ] **1.2.2 Wire toolbar interactions**
  - [ ] Clicking "Prepare" calls `enterPrepareMode()`.
  - [ ] Clicking "Support" calls `enterSupportMode()`.
  - [ ] Prevent redundant updates when clicking the already-active mode.
  - [ ] **Interaction flow example:**
    - [ ] User launches app → app starts in **Prepare** mode (current behavior).
    - [ ] User clicks **Support** icon in top bar:
      - [ ] Global mode flips to `"support"`.
      - [ ] Sidebar switches from `PrepareSidebar` to `SupportSidebar`.
      - [ ] All transform tool buttons disappear or are visually disabled.
      - [ ] Mouse cursor/tool hint in viewport changes to indicate "click to place support".
    - [ ] User clicks **Prepare** icon again:
      - [ ] Mode flips back to `"prepare"`.
      - [ ] Sidebar returns to `PrepareSidebar`.
      - [ ] Transform tools and gizmos become available again.

---

## 2. Sidebar Switching & Tool Availability

When the mode changes, the sidebar should completely change its content from general model tools to support-related tools. At the same time, transform tools must be disabled to avoid conflicts with support placement.

### 2.1 Sidebar router

- [ ] **2.1.1 Introduce a sidebar router component**
  
  Instead of manually showing/hiding multiple sidebars, we create a single router component that chooses which sidebar to render based on the current mode.
  - [ ] Reads `mode` from global state.
  - [ ] Renders `PrepareSidebar` when `mode === "prepare"`.
  - [ ] Renders `SupportSidebar` when `mode === "support"`.

### 2.2 Transform tool visibility

In Support mode, the user should not accidentally move or rotate the model. This section ensures that only support-specific actions are available.

- [ ] **2.2.1 Hide transform tools in Support mode**
  - [ ] Identify all transform tools (move/rotate/scale, etc.).
  - [ ] When `mode === "support"`:
    - [ ] Hide transform buttons in the UI.
    - [ ] Disable hotkeys that affect object transforms.
    - [ ] Ensure transform gizmos in the viewport are not shown.

- [ ] **2.2.2 Ensure transform tools still behave correctly in Prepare mode**
  - [ ] Verify transforms are fully functional in `"prepare"` mode.
  - [ ] Confirm mode switching does not leave gizmos in inconsistent states.

---

## 3. Support Sidebar – UI for Support Settings

Support mode replaces the normal sidebar with controls that define *how new supports look* when they are placed. This is where we expose tip size, shaft diameter, base size, and related options, similar to Lychee’s support panels.

### 3.1 Baseline sections (per Lychee concepts)

- [ ] **3.1.1 Layout of SupportSidebar**
  
  We divide the UI into logical sections that match how a support is constructed, making it easier for the user to understand which controls affect which part of the support.
  - [ ] Section: "Placement" (tips / snapping behavior).
  - [ ] Section: "Tip" (tip geometry controls).
  - [ ] Section: "Shaft" (mid/column settings).
  - [ ] Section: "Base" (base settings).
  - [ ] Later sections: "Bracing", "Advanced", etc.

- [ ] **3.1.2 Placement settings (v1)**
  
  Placement controls describe the *behavior* of new supports when you click on the model, rather than their geometry. For v1 we keep this simple: the tip is always perpendicular to the surface.
  - [ ] Option: tip angle behavior (for now always perpendicular to polygon; UI can show it as read-only or simple label).
  - [ ] Option: auto-lift (future; for now, disabled or hidden).

- [ ] **3.1.3 Tip settings (v1)**
  
  These fields directly correspond to the `tip` section in our support schema and in Lychee. Changing them should make the tip visually larger/smaller or longer/shorter when new supports are placed.
  - [ ] Numeric input: `Tip point diameter (mm)`.
  - [ ] Numeric input: `Tip diameter (mm)`.
  - [ ] Numeric input: `Tip length (mm)`.
  - [ ] Optional / future: `Tip angle`, `penetration`.
  - [ ] **Behavior requirement:** When the user adjusts these values, the next support they place must use the new geometry. Existing supports may either stay unchanged (simpler v1) or be updated when explicitly edited.

- [ ] **3.1.4 Shaft settings (v1)**
  
  The shaft is the main column between base and tip. For now, a single diameter is enough; later we could add tapered or adaptive shapes.
  - [ ] Numeric input: `Shaft diameter (mm)`.
  - [ ] Optional / future: shaft type (`cylinder` / `cube`).
  - [ ] **Behavior requirement:** This controls the "orange stick" thickness you see between tip and base in Lychee. Users expect that increasing this value makes supports visibly beefier and more stable.

- [ ] **3.1.5 Base settings (v1)**
  
  Base controls determine how the support attaches to the plate or to large flat regions. Small bases are easier to remove; large bases are more stable.
  - [ ] Numeric input: `Base diameter (mm)`.
  - [ ] Numeric input: `Base height (mm)`.
  - [ ] Optional / future: additional parameters (join diameter/length, adaptive base).
  - [ ] **Behavior requirement:** Base size should be obvious in the viewport (a small disk vs a large disk). It should always sit flush on the build plate (respecting plate normal and position).

### 3.2 State & interactions

The sidebar controls define a *current support profile*. New supports should use this profile, and the UI must stay in sync with whatever values are active.

- [ ] **3.2.1 Support profile state**
  - [ ] Maintain a "current support settings" object in global state (the values bound to the sidebar inputs).
  - [ ] On change of any input, update that object.
  - [ ] New supports created in Support mode will copy these settings into their `settings`.

- [ ] **3.2.2 Editing existing supports (later)**
  - [ ] Future: Clicking an existing support could load its settings into the sidebar.
  - [ ] For v1, we may restrict to creating supports with the current global settings only.

---

## 4. Viewport Interaction in Support Mode

This is the core of the feature: when the user is in Support mode and clicks on the model, we need to compute exactly where and how a support should be created. This involves raycasting, normals, and deciding how the base is positioned.

### 4.1 Raycasting and tip placement

- [ ] **4.1.1 Raycast to model surface**
  
  Every support starts with a tip on the model surface. We raycast from the camera through the mouse cursor into the scene to find the exact triangle that was clicked, its position, and its normal.
  - [ ] In `"support"` mode, repurpose left-click on the viewport to mean "place a support tip".
  - [ ] Implement raycast from camera through mouse cursor into the scene.
  - [ ] Determine which object/triangle is hit:
    - [ ] Get `objectIdTip`.
    - [ ] Get hit point as `tip` `{ x, y, z }`.
    - [ ] Get surface normal at that triangle as `tipNormal` `{ x, y, z }`.
  - [ ] If nothing is hit (empty space), do not create a support.
  - [ ] **Behavior requirement:** The tip orientation must be **perpendicular to the polygon** where it is placed, matching the user’s requirement. That means:
    - [ ] Use `tipNormal` as the local "up" axis for the tip geometry.
    - [ ] When rendered, the cone’s central axis is aligned with `tipNormal` (or its opposite, depending on convention).
  - [ ] **Interaction flow example:**
    - [ ] User enters Support mode.
    - [ ] User moves mouse over model; optional: highlight the triangle under cursor or show a ghost tip.
    - [ ] User left-clicks once → app performs raycast and calculates `tip` and `tipNormal`.
    - [ ] If the click hits a valid triangle, we proceed to base computation; otherwise we ignore the click.

- [ ] **4.1.2 Restrict placement to supported objects**
  - [ ] Optionally restrict to objects flagged as "supportable".
  - [ ] Handle objects that are hidden/locked.

### 4.2 Compute initial base position and orientation

- [ ] **4.2.1 Decide on v1 base behavior**
  
  Once we know the tip, we must decide where the other end of the support (the base) should land. For v1 we keep this deterministic and simple so behavior is predictable.
  - [ ] For a first version, we can:
    - [ ] Project the tip downward along world -Z to the build plate.
    - [ ] Or project along the tip direction (tipNormal inverted) until it hits plate.
  - [ ] Choose one consistent strategy and document it.

- [ ] **4.2.2 Implement base placement**
  - [ ] Given `tip` and `tipNormal`, compute support axis direction:
    - [ ] For v1, axis can be opposite of `tipNormal` (perpendicular to polygon).
  - [ ] Intersect axis with plate plane (z=0 or plate position).
  - [ ] Use that point as `base` `{ x, y, z }`.
  - [ ] Set `baseNormal` to plate normal (`{ 0, 0, 1 }`).
  - [ ] **Behavior requirement:** For this first version, we always drop the support to the build plate, not to other supports or interior fill. Later we can add parent/child supports and bracing.
  - [ ] **Edge cases:**
    - [ ] If the axis does not intersect the plate (e.g. tip below plate), abort support creation.
    - [ ] If intersection is outside the plate bounds, either:
      - [ ] Clamp to plate extents, or
      - [ ] Reject placement and show a user message.

- [ ] **4.2.3 Support validity checks**
  - [ ] Ensure tip and base are not too close (minimum length threshold).
  - [ ] If invalid, either:
    - [ ] Cancel the support creation.
    - [ ] Or snap to a minimum length.

### 4.3 Creating the support instance

Once we have both ends of the support (tip and base) and the user’s chosen settings, we can construct a full `SupportInstance` and add it to the scene.

- [ ] **4.3.1 Build the `SupportInstance`**
  - [ ] On valid click in Support mode:
    - [ ] Allocate new support ID.
    - [ ] Fill `objectIdTip`.
    - [ ] Fill `tip`, `tipNormal`.
    - [ ] Fill `base`, `baseNormal`.
    - [ ] Copy current `SupportSidebar` settings into `settings`.
    - [ ] Set any default flags (e.g. `isVisible = true`).

- [ ] **4.3.2 Store in scene state**
  - [ ] Add new instance to `supports.byId` and `supports.allIds`.
  - [ ] Add ID to owning object’s `supportsBase` (or equivalent).
  - [ ] Mark scene as dirty/changed so user can save.

---

## 5. Rendering Supports in the Viewport

With data in place, we need to actually draw supports so the user can see and trust what they have placed. This section defines how to turn `SupportInstance` data into simple cones/cylinders in the 3D view.

### 5.1 Basic visualization

- [ ] **5.1.1 Generate geometry for one support instance**
  - [ ] Use `tip` and `base` to define a line/axis.
  - [ ] Use `settings` to define radii and lengths along that axis:
    - [ ] Tip cone.
    - [ ] Mid shaft (cylinder) from end of tip to near base.
    - [ ] Base cylinder near the plate.
  - [ ] Rendering approach for v1:
    - [ ] Option A: procedural meshes built on the fly per support.
    - [ ] Option B: reuse simple cylinder/cone primitives and place/scale them.
  - [ ] **Implementation notes:**
    - [ ] Compute a transform matrix that takes a unit cone/cylinder aligned with +Z and scales/rotates it to run from tip to base.
    - [ ] For the tip cone, start at `tip` and extend along the support axis for `settings.tip.length`.
    - [ ] For the shaft, start where the tip ends and stop just above the base by `settings.base.length`.
    - [ ] For the base, create a short cylinder centered at the base position, aligned with plate normal.

- [ ] **5.1.2 Visual style**
  - [ ] Choose colors (e.g. orange shafts, blue bases/tips as in Lychee screenshot).
  - [ ] Make supports semi-transparent or solid based on user preference.

### 5.2 Rendering performance

If the user adds hundreds of supports, naive rendering may become slow. These items ensure we have at least a basic plan for scaling up.

- [ ] **5.2.1 Batch rendering**
  - [ ] Evaluate whether supports should be batched into a single mesh/instance group.
  - [ ] Or render each support as separate geometry (simpler; may be OK for moderate counts).

- [ ] **5.2.2 Update strategy**
  - [ ] Ensure supports are re-generated only when:
    - [ ] A new support is added.
    - [ ] Existing support settings change.
    - [ ] Scene transforms (plate or object) change.

---

## 6. Basic Support Editing (Optional v1.5)

Once basic placement works, we’ll likely want to adjust or delete supports without recreating them. These tasks introduce minimal editing features but can be postponed until v1 of placement is stable.

- [ ] **6.1 Selection**
  - [ ] Allow clicking on a support to highlight/select it.
  - [ ] Show some feedback (outline, color change).

- [ ] **6.2 Deletion**
  - [ ] Keyboard shortcut (e.g. Delete) to remove selected support(s).
  - [ ] Remove from `supports.byId`, `allIds`, and object’s `supportsBase`.

- [ ] **6.3 Editing geometry settings**
  - [ ] When a single support is selected:
    - [ ] Load its `settings` into the SupportSidebar inputs.
    - [ ] Changes in the sidebar update that instance’s settings (and re-render it).

---

## 7. Multi-Segment / Hierarchical Supports (Future)

Lychee models complex supports as chains of simpler supports connected by joints. This section lays out how we could achieve similar behavior later, once single straight supports are solid.

- [ ] **7.1 Hierarchy fields**
  - [ ] Add `parentBaseId` / `parentTipId` to `SupportInstance`.
  - [ ] Define how chains are structured (e.g., parent at base).

- [ ] **7.2 UI to create chained supports**
  - [ ] Interaction option: place a support that attaches to another support instead of the plate.
  - [ ] Raycast to supports (not just objects) when clicking in Support mode.

- [ ] **7.3 Rendering joints**
  - [ ] Visualize joints (e.g. blue spheres) at the connection points between supports.

---

## 8. Integration & UX Polish

These items make the feature feel complete: persistence, undo/redo, helpful errors, and tests. They don’t change the core model but dramatically improve user experience and reliability.

- [ ] **8.1 Mode persistence**
  - [ ] Remember last mode between app runs or per project (optional).

- [ ] **8.2 Undo/redo**
  - [ ] Integrate support creation/deletion and setting changes into the undo system.

- [ ] **8.3 Error handling & messaging**
  - [ ] Provide user feedback when a support cannot be placed (no hit, invalid geometry, etc.).

- [ ] **8.4 Testing**
  - [ ] Unit tests for support data model operations.
  - [ ] Integration tests for mode switching and basic placement.
  - [ ] Visual/manual test cases:
    - [ ] Simple cube with a few supports.
    - [ ] Overhang-only areas.
    - [ ] Edge cases near plate and near model corners.

---

## 9. Alignment With Lychee Concepts (Reference)

Finally, we verify that our design genuinely matches the Lychee concepts we reverse-engineered, so that future import/export or compatibility work is straightforward.

- [ ] **9.1 Verify we can map Lychee supports to this model**
  - [ ] `tip` / `tipNormal` ↔ Lychee `tip` / `tipNormal`.
  - [ ] `base` / `baseNormal` ↔ Lychee `base` / `baseNormal` or plate.
  - [ ] `settings.tip` / `settings.mid` / `settings.base` ↔ Lychee’s `tip`/`mid`/`base` sections.

- [ ] **9.2 Confirm extensibility**
  - [ ] Ensure we can later add `baseTip`, `extra`, bracing, and islands/minimas if desired.
