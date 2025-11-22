# Lychee Support System – Reverse Engineering Notes

## 1. Location in JSON

- **File**: `scene.decrypted.json`
- **Section**: Root-level `supports` object (starting around line 80 in the current file)
- This section describes **support system configuration**, not individual support instances.

## 2. High-Level Structure of `supports`

Key children of the `supports` object include:

- **Geometry presets (per support type)**
  - `defaultSupportFDM`
  - `supportMedium`
  - `supportLight`
  - `defaultSupport` (resin default)
  - `autoSupportFDM`
  - `supportHeavy`
- **Presets collection**
  - `supportsPresets` → `favorites` and named `presets` (`sp4`, `sp6`, `sp7`, ...)
- **Bracing configuration**
  - `bracingPreset`
- **Auto-support & creator settings**
  - `supportCreator`
  - `supportMode`, `supportModeFDM`
  - Various painting/grid/interval parameters
- **Island / minima detector presets**
  - `islandsDetector`

This block defines **how supports are shaped and generated**, not where each support is placed.

## 3. Common Geometry Schema (Per Support Preset)

Multiple keys (`defaultSupportFDM`, `supportMedium`, `supportLight`, `defaultSupport`, `autoSupportFDM`, `supportHeavy`, and each `supportsPresets.presets.spX.settings`) all share the same internal structure:

- **`adaptiveBase`**
  - Boolean indicating whether the base adapts (e.g., scales) based on conditions like height or load.

- **`tip`**
  - Describes the geometry at the **contact point with the model**.
  - Fields:
    - `type`: shape type (`"cone"`, `"cube"`, etc.).
    - `pointDiameter`: diameter of the actual contact point on the model.
    - `penetration`: how far the tip penetrates into the model surface (0 = just touching).
    - `diameter`: base diameter of the tip profile (slightly below the contact point).
    - `length`: tip length along the support axis.
    - `breakPoint`: designed weak point along the tip (often 0 in our sample).
    - `angle`: side-wall or cone angle (degrees).

- **`base`**
  - Describes the **base element** near the build plate or macro structure.
  - Fields:
    - `type`: shape type (`"cylinder"`, `"cube"`).
    - `diameter`: main base footprint diameter.
    - `length`: base thickness/height.
    - `angle`: base side angle (often 0).
    - `joinDiameter`: diameter of the neck that connects the base to the column.
    - `joinLength`: length of that neck segment.
    - `joinCone`: 0..1 controlling conical transition between base and neck.

- **`baseTip`**
  - Connector between the **main column** and the **base**.
  - Fields mirror `tip`:
    - `type`: usually `"cone"` or `"cube"`.
    - `pointDiameter`, `penetration`, `diameter`, `length`, `breakPoint`, `angle`.
    - `isStraight`: whether this connector is strictly vertical vs allowed to angle.

- **`mid`**
  - Describes the **column/shaft** geometry between baseTip and tip.
  - Fields:
    - `type`: `"cylinder"` or `"cube"` (indicates cross-section of the column).

- **`extra`**
  - Secondary diameters/parameters for variants or adaptive strengthening.
  - Fields:
    - `tipDiameter2`
    - `baseDiameter2`
    - `baseTipDiameter2`
    - `baseJoinDiameter2`
    - `baseTipPointDiameter2`
    - `tipPointDiameter2`
  - These appear to define alternative diameters used when supports “grow” (e.g., heavier presets, child supports, or adaptive bases).

- **`isStraight`** (sometimes at root of the preset)
  - Boolean indicating whether the **overall support column** is straight vs allowed to bend/angle.

### Conceptual Composition of a Single Support Column

From this schema, a single Lychee support column can be thought of as composed of:

1. **Tip**
   - Contact region with the model surface.
2. **Mid column**
   - Main shaft running from near the tip downwards.
3. **BaseTip**
   - Transition from mid column into the base.
4. **Base**
   - Anchor region near build plate or large foundation structure.
5. **Extra**
   - Alternative dimensions that can be applied based on rules (heavier, children, adaptive).

All dimensions appear to be in **millimeters**, consistent with Lychee and typical slicer conventions.

## 4. Example Presets Observed

From the current JSON section:

- **`supportLight`**
  - Tip: small cone (e.g., `pointDiameter` ≈ 0.22, `diameter` ≈ 0.8, `length` ≈ 2.5).
  - Base: moderate cylinder (e.g., `diameter` 4, `length` 0.2, `joinDiameter` 0.8, `joinCone` 0.7).
  - BaseTip: cone bridging column to base.
  - Mid: cylinder column.
  - Extra: modest second diameters.
  - `isStraight`: false (non‑strict vertical allowed).

- **`supportMedium`**
  - Similar to `supportLight` but with larger tip/base diameters and lengths.
  - Generally stronger than light, lighter than heavy.

- **`supportHeavy`**
  - Tip: larger cone (e.g., `pointDiameter` 0.4, `diameter` 1.2).
  - Base: larger cylinder (e.g., `diameter` 6, `length` 0.3).
  - BaseTip: stronger connector.
  - Extra: much larger secondary diameters for base and baseTip.

- **`defaultSupport`**
  - Similar structure to heavy, used as the **global default resin support**.

- **FDM variants** (`defaultSupportFDM`, `autoSupportFDM`)
  - Use `"cube"` types and larger diameters/lengths appropriate for filament printing.

## 5. Presets and Favorites (`supportsPresets`)

- `supportsPresets` contains:
  - `favorites`: mapping from `fav1`, `fav2`, `fav3` to preset IDs (e.g., `"sp4"`, `"sp6"`, `"sp7"`).
  - `presets`: named presets, each with:
    - `name`: user-facing preset name (e.g., `"Light Mini"`, `"Medium Mini"`, `"Heavy Mini"`).
    - `settings`: full support geometry config (`adaptiveBase`, `tip`, `base`, `baseTip`, `mid`, `extra`, `isStraight`).

This indicates Lychee’s UI exposes a set of **named support styles** that all map back to the same underlying geometry schema.

## 6. Bracing Configuration (`bracingPreset`)

- `bracingPreset` contains a `settings` object with presets `bp1`–`bp4` and a `selected` key.
- Each `bpX` preset includes:
  - `name`: e.g., "Small Object Bracing", "Tall Object Bracing", "Strong Bracing", "Default Bracing".
  - `firstCathedralHeight`, `secondCathedralHeight`:
    - Heights in Z where different bracing tiers are applied.
  - `zStartPoint`:
    - Minimum Z from which bracing is considered.
  - `maxDiameter`:
    - Maximum support diameter eligible for bracing.
  - `bottom`, `middle`, `top`:
    - Each includes:
      - `pattern`: `"simple"`, `"double"`, or `"mix"`.
      - `gap`: spacing between braces.
      - `size`: brace thickness/strength parameter.
  - `gap`: global spacing/gap parameter.
  - `diagonalSettings`: integer indicating diagonal bracing style/aggressiveness.
  - `fakeGridGap`: spacing for an internal/virtual brace grid.

This describes **how supports are cross-linked** (cathedral/bracing structures) based on height and diameter.

## 7. Auto-Support & Creator Settings (`supportCreator` and related)

Key fields:

- **`supportMode`**: `"minimaAndHoverhang"`
  - Indicates the generator will support **minima (islands)** and **overhangs**.

- **`supportModeFDM`**: `"minimaAndHoverhangGrid"`
  - FDM-specific mode; similar concept but with grid support behavior.

- **`supportCreator`**:
  - `onlyIsland`: if true, restricts auto-supports to islands only.
  - `autoBracing`: whether to automatically create bracing using `bracingPreset`.
  - `presetAutoSupport`: named behavior (e.g., `"heavilySupported"`).
  - `interval` (`value`, `unit`): spacing used for scanning/placement (e.g., 3 mm).
  - `minBounds` (`value`, `unit`): minimum feature or tip size to consider (e.g., 0.28 mm).
  - `autoParenting`: whether supports are automatically grouped in a hierarchy.
  - `maxMidLength`: maximum mid column length before adjusting geometry/bracing.
  - `presetMinima`: how minima (islands) are defined (e.g., `"real"`).
  - `presetAutoSupportCustom`:
    - `resolutionXY`, `resolutionZ`: analysis resolutions (mm) for auto-support generation.
  - `autoLift`, `autoLiftValue`: whether and how much to auto-lift the model.

- Additional global support-related flags:
  - `supportGridGap`: spacing when a grid layout is used.
  - `inlineSupports`, `inlineSupportsParameters`: settings for in-object supports.
  - `maxTotalGrowth`, `growthPerChildren`: how support sizes evolve with children.
  - `safeDistToRaycast`: safety margin when raycasting to the model.
  - Various `painting...` flags and thresholds for brush-based support painting.

These fields define the **behavior of Lychees auto-support generator** rather than the geometry of individual supports.

## 8. Island Detector Presets (`islandsDetector`)

- `islandsDetector` provides three presets:
  - `fast`
  - `normal`
  - `detailed`
- Each preset includes:
  - `z`: Z-resolution (mm) for island analysis.
  - `xy`: XY-resolution (mm) for island analysis.
- `filterAreaMin`: minimum island area (0 in this file means no area filter).

This section is analogous to a **voxel/pixel-based island detection configuration**:

- Smaller `xy` and `z` values produce more detailed but slower detection.
- These settings conceptually match a rasterization-based island scan like the one used in the STL Slicer POC.

## 9. Takeaways for Our Future Support Model

From this section alone, we can identify several necessary layers in a new, Lychee-compatible support system:

1. **Support Geometry Preset Schema**
   - Must be able to represent:
     - `adaptiveBase`, `tip`, `base`, `baseTip`, `mid`, `extra`, `isStraight`.
   - Each preset is essentially a reusable **support style**.

2. **Support Preset Management**
   - Named presets with human-facing labels (e.g., "Light Mini").
   - Ability to mark some as favorites.

3. **Bracing Profiles**
   - Configuration of cathedral/bracing behavior across height ranges.
   - Includes patterns (`simple`, `double`, `mix`), gaps, sizes, and diagonal settings.

4. **Auto-Support Generator Settings**
   - Modes like `minimaAndHoverhang`.
   - Interval/minBounds/resolution parameters controlling where supports are proposed.
   - Options for auto-bracing, auto-parenting, and auto-lift.

5. **Island Detection Presets**
   - Separate but closely related: defines how to find islands that drive support placement.

We still need to locate and analyze the **per-support instance data** (specific positions and orientations) elsewhere in the JSON to fully reconstruct Lychee scenes. However, this `supports` block gives us a clear view of the **parametric support style system** that our own model must be able to represent and import.

## 10. Per-Support Instance Data (`supports.present.byId`)

A second root-level `supports` object (later in the JSON, around line 1718) contains **actual support instances**:

- Path: `supports.present.byId`
- Keys: support IDs like `"s410"`, `"s531"`, `"s652"`, `"s773"`, `"s894"`, ...
- Each entry describes **one support element in 3D space** with its own geometry settings and relationships.

### 10.1 Common Fields per Support Instance

For each support (e.g., `s410`):

- **`id`**
  - String identifier of the support (e.g., `"s410"`).

- **`type`**
  - Observed value: `1` in the sample entries.
  - Likely indicates the support category (e.g., standard resin column).

- **`settings`**
  - Full per-support geometry configuration, mirroring the preset schema:
    - `adaptiveBase`
    - `tip` (type, pointDiameter, penetration, diameter, length, breakPoint, angle)
    - `base` (type, diameter, length, angle, joinDiameter, joinLength, joinCone)
    - `baseTip` (type, pointDiameter, penetration, diameter, length, breakPoint, angle, isStraight)
    - `mid` (type)
    - `extra` (tipDiameter2, baseDiameter2, baseTipDiameter2, baseJoinDiameter2, baseTipPointDiameter2, tipPointDiameter2)
    - `isStraight`
  - In the sample, these settings closely match known presets (e.g., Medium, Heavy) but are inlined per instance.

- **`objectIdTip`**
  - ID of the **object/mesh** that the support tip is attached to (e.g., `"o5"`).
  - Used to resolve which model the support is supporting at the tip.

- **`objectIdBase`**
  - ID of the object at the **base** of the support (often the same `"o5"` or a plate object).
  - For supports rooted on the build plate, this may reference the plate object.

- **`tip`** (position)
  - 3D coordinates of the tip in scene space:
    - `x`, `y`, `z`
  - Example (s410):
    - `"tip": { "x": -15.6099..., "y": -23.9828..., "z": -19.8206... }`
  - This is where the support touches the model.

- **`base`** (position)
  - 3D coordinates of the base in scene space:
    - `x`, `y`, `z`
  - Example (s410):
    - `"base": { "x": -14.3070..., "y": -27.9618..., "z": -21.3246... }`
  - Defines the other endpoint of the main column / base connection.

- **`tipNormal`**
  - Normal vector at the tip contact point:
    - `x`, `y`, `z`
  - Example:
    - `"tipNormal": { "x": -0.1331..., "y": -0.8176..., "z": -0.5601... }`
  - Likely the **surface normal of the model** at the contact point, used to orient the tip.

- **`newTipNormal`**
  - Often `null` in the sample.
  - Probably used when the tip normal is updated/recomputed after editing.

- **`baseNormal`**
  - Normal vector at the base contact point:
    - `x`, `y`, `z`
  - Example (s652 on build plate):
    - `"baseNormal": { "x": 0, "y": 0, "z": 1 }`
  - For in-fill bases, this normal follows the surface they anchor to.

- **`isBaseTip`**
  - Boolean.
  - When `true`, indicates the support is a **base tip element** (e.g., connecting to another support rather than directly to the plate).
  - Example: several supports (s410, s531, s773, s894) have `isBaseTip: true`, indicating a hierarchical/bracing relationship.

- **`isInFill`**
  - Boolean.
  - When `true`, the base is considered **inside the object/infill** rather than on the plate.
  - Example: `s773` has `isInFill: true`.

- **`parentBaseId`**
  - ID of another support that this support’s base is attached to (e.g., `"s407"`, `"s530"`).
  - When non-null, this indicates **support-to-support attachment** (bracing or multi-level structures).
  - When `null`, the base is attached directly to the build plate or to an object surface.

- **`parentTipId`**
  - ID of another support that this support’s tip is attached to.
  - In the sample, this is often `null`, suggesting tip usually attaches to the model object.

- **`parentId`**
  - Array of parent IDs (empty array in the sample entries).
  - May be used to store more complex hierarchies or legacy parent relations.

- **`gridNodeIndex`**
  - Integer or `null`.
  - Likely indexes into an internal **support grid** used for layout / snapping.
  - Example: `gridNodeIndex: 1476` for `s410`; can be `null` for some supports.

- **`collisionIsAccepted`** / **`isCollidingWithObject`**
  - Booleans indicating collision state and whether the user has accepted a collision.

- **`isVisible`**
  - Boolean flag controlling rendering/visibility.

- **`vertical`** / **`straight`** / **`mini`** / **`group`**
  - Additional booleans:
    - `vertical`: whether the support is considered vertical.
    - `straight`: whether the shape is straight (distinct from `settings.isStraight`).
    - `mini`: when true, indicates a “mini” support variant.
    - `group`: whether this support is part of a group selection or structure.

- **`tags`**
  - Array (often empty), likely used for labeling/categorization.

- **`updatedAt`**
  - Timestamp-like numeric field (0 in the sample), possibly for undo/history.

### 10.2 Interpretation: What a Single Instance Represents

A single `supports.present.byId["sXXX"]` record fully defines:

- Which **object** it supports (`objectIdTip`) and where (`tip` coordinates + `tipNormal`).
- Where the **base** is (`base` coordinates + `baseNormal`) and whether it sits on the plate, in-fill, or on another support (`isInFill`, `parentBaseId`).
- The **exact support geometry** via embedded `settings` (effectively a frozen snapshot of a preset, possibly with per-instance modifications).
- How it is integrated into a **hierarchy** of supports via `parentBaseId`, `parentTipId`, and `parentId`.
- Metadata for rendering and collision handling (`isVisible`, `isCollidingWithObject`, `collisionIsAccepted`, `gridNodeIndex`, `tags`).

### 10.3 Relationship to Presets

- The `settings` inside each instance correspond closely to one of the **support presets** defined in the first `supports` block (e.g., Light/Medium/Heavy presets).
- Lychee appears to **copy the preset settings into the instance** at creation time, rather than storing a preset ID reference.
- For conversion purposes, we can:
  - Either treat each instance’s `settings` as authoritative and import them directly.
  - Or attempt to **map back** to the nearest known preset to reduce redundancy.

### 10.4 Implications for Our Support System

To faithfully reconstruct Lychee supports in our own system, we need to be able to represent:

- **Support instances** with:
  - Unique ID.
  - Tip position and normal.
  - Base position and normal.
  - References to supported objects at tip and base.
  - Optional parent support IDs (for chained/braced structures).
  - Visibility and collision flags.

- **Embedded or referenced geometry settings**:
  - Either store full geometry per instance (like Lychee) or store a preset reference plus any per-instance overrides.

- **Hierarchical / bracing relationships**:
  - Using fields like `parentBaseId`, `parentTipId`, and `parentId` to form graphs of supports.

This instance-level model, combined with the preset-level configuration described in earlier sections, gives us enough information to design a new support system that can **import Lychee support layouts** while still being flexible for our own tools and algorithms.

## 11. Minimas, Objects, and Plates – How They Tie Into Supports

- **Minimas (`minimas.present.byId`)**
  - Each `mXXX` entry represents a detected unsupported island on a specific object.
  - Key fields:
    - `objectId`: which object (e.g., `"o5"`).
    - `zPosition`: height of the island.
    - `area`: island area in mm².
    - `angle`: local surface/overhang orientation.
    - `hasSupport`: whether Lychee has already placed supports for this minima.
  - Conceptually corresponds to the islands produced by our raster scan.

- **Objects (`objects.present.byId`)**
  - Example: `o5` is the main STL object.
  - Stores transform (`position`, `scale`), dimensions, file paths, and other properties.
  - Support linkage:
    - `hasMinima`: indicates associated `minimas` entries.
    - `supportsBase`: array of support IDs (`sXXX`) whose bases belong to this object/plate.
    - `plateId`: which build plate (`pl0`) the object is on.

- **Plates (`plates.present.byId`)**
  - Example: `pl0` with `size { x, y, z }` and `position { x, y, z }`.
  - Defines the printable area and origin for supports and objects.

- **Collection pattern (`present.byId` + `present.allIds`)**
  - Supports, minimas, objects, plates, holes, etc., all use the same normalized structure:
    - `byId`: mapping from ID → full entity data.
    - `allIds`: ordered list of IDs.
  - For supports, we iterate `supports.present.allIds` and resolve each instance via `supports.present.byId[id]`. Objects then reference relevant supports via `supportsBase`.

Together, these pieces show Lychee’s pipeline:

1. Analyze objects → produce `minimas` (islands).
2. Use support presets + creator settings to place `supports.present.byId` instances.
3. Bind supports back to objects and plates via `objectIdTip`, `objectIdBase`, `plateId`, and `supportsBase`.
