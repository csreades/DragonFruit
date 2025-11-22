# Pixel-Based Island Scanner Design

## Definitions
- **Pixel size (`px_mm`)**: Square pixel edge length in millimeters. Default 0.10 mm (100 µm). User adjustable.
- **Layer index (`L`)**: Integer 0..N. Layer 0 is the first printable layer above the build plate.
- **Cross-section mask (`M[L]`)**: A 2D boolean grid at pixel size `px_mm` for layer `L`. `true` means solid resin at that pixel in the cross-section.
- **Pixel group (component)**: A 4-neighbor connected set of `true` pixels within `M[L]` (adjacent only via up/down/left/right; diagonals do not connect groups).
- **Island**: A pixel group at some layer that is not supported by any overlapping pixels from the previous layer.
- **Island ID (`id`)**: Stable identifier assigned when an island first appears (becomes unsupported) and tracked upward across layers while it persists.
- **Parent/Child**: When two islands merge, the island with the largest previous-layer area becomes the parent; the other becomes a child and is marked complete at the merge layer.

## High-level Rules
1. Rasterize each cross-section into a square grid using `px_mm` resolution. No subpixel geometry—pixels are either inside or outside.
2. Layer 0 (first layer): there is no prior support; every 4-connected pixel group in `M[0]` is an island. Assign a unique ID to each group.
3. Layer L ≥ 1:
   - Compute `M[L]`. Compute `supported[L] = M[L] ∧ dilate(M[L-1], support_buffer_px)`.
   - Compute `islandCandidates[L] = M[L] ∧ ¬supported[L]`.
   - Find connected components in `islandCandidates[L]`. For each component, determine its ancestry by overlapping with previous-layer island labels (propagation) and assign or continue IDs.
4. Island propagation across layers: A component at `L` inherits the ID from the previous layer component(s) it overlaps. If multiple previous IDs overlap, a merge occurs.
5. Merge rule: when multiple previous IDs overlap a current component, the island with the largest area on layer `L-1` becomes the parent; all others become children and are marked complete at `L`.
6. Completion: An island is complete when it merges into another island. Record its last layer index and final area stats.

## Parameters
- `px_mm` (default 0.10 mm) — pixel size; affects speed and precision. The smallest detectable island is one pixel at this size.
- `support_buffer_mm` (default 0.20 mm) — buffer radius; applied as morphological dilation on `M[L-1]` in pixels: `support_buffer_px = round(support_buffer_mm / px_mm)`.
- `connectivity` (default 4) — connected-component connectivity (4; diagonals do not connect groups). 8-connectivity can be revisited later.
- `overlap_tolerance` (default strict 0) — a group is considered supported only if any pixel overlaps the dilated prev layer. Optional future epsilon (< 1 pixel) can be introduced if needed.

## Data Structures
- `LabelGrid[L]`: 2D int32 grid parallel to `M[L]` storing island IDs for unsupported pixels at layer `L` (0 if none).
- `islands`: Map `id -> { id, firstLayer, lastLayer, totalAreaMm2, perLayerAreaMm2: Map<L, area>, status: 'active' | 'complete' }`.
- `nextId`: monotonically increasing integer for new islands.

## Algorithm
1. Precompute geometry bounds and allocate grid dimensions from `px_mm`.
2. For each layer `L` from 0..N:
   - Rasterize cross-section polygons into `M[L]` (sample pixel centers; point-in-polygon; optional anti-alias later).
   - If `L == 0`:
     - `supported[L] = false`.
     - `islandCandidates[L] = M[L]` (groups are 4-connected components of `M[0]`).
   - Else (`L >= 1`):
     - Build `prev = M[L-1]`; dilate by `support_buffer_px` -> `prevDilated`.
     - `supported[L] = M[L] ∧ prevDilated`.
     - `islandCandidates[L] = M[L] ∧ ¬supported[L]`.
   - Run connected-components on `islandCandidates[L]` (using `connectivity`). For each component `C`:
     - Compute area_px and `area_mm2 = area_px * px_mm^2` (for reporting/metrics only; no minimum area filter is applied).
     - Determine overlaps with previous layer labels: collect `prevIds = { LabelGrid[L-1][p] | for each pixel p in C }` (ignore 0s).
     - Cases:
       - `prevIds` empty: New island. `id = nextId++`. Mark `LabelGrid[L][p] = id` for all pixels in `C`. Create island record with `firstLayer = L`, `status = 'active'`.
       - `prevIds` has one element `{id}`: Continuation. Assign `id` to `C` in `LabelGrid[L]`.
       - `prevIds` has multiple IDs: Merge. Choose parent `id_parent` = argmax(area at L-1). For all other `id_child` in `prevIds \ {id_parent}`: set `islands[id_child].lastLayer = L`, `status = 'complete'`. Assign parent ID to `C` in `LabelGrid[L]`.
     - Update `islands[id]` per-layer area: `perLayerAreaMm2[L] += area_mm2`; update `totalAreaMm2`.
   - Optional: snapshot per-layer stats for debug overlays.
3. Output:
   - `islands` list with their lifetimes and areas.
   - For cap rendering at any `L`, contours come from connected components of `islandCandidates[L]` (marching squares on the pixel mask or polygonization of component boundaries).
   - For mesh tinting, build prisms using per-layer contours and requested thickness.

## Marching Squares for Caps
- Run marching squares on `islandCandidates[L]` for each labeled component to extract vector contours.
- Convert contour pixels to `THREE.Shape` paths; cap color per island ID for differentiation.

## Performance Notes
- Complexity scales with `grid_width * grid_height * layers`. Choose `px_mm` to balance quality and speed.
- Use a Web Worker pool to process layers in parallel; maintain per-layer order for progress UI.
- Optimize rasterization by limiting to geometry/loop bounding boxes per layer.
- Use typed arrays (`Uint8Array` for masks, `Int32Array` for labels) for cache-friendly passes.

## Edge Cases
- Thin features near pixel resolution: may flicker between supported/unsupported; consider minimum support ratio over 3×3 neighborhood to stabilize (optional).
- First appearance spanning multiple disjoint components: each component gets its own new ID.
- Degenerate merges (touch by a single corner pixel): with 4-connectivity, diagonals do not merge (kept separate). We may revisit 8-connectivity later if diagonal merges are desired.

## User Controls
- Pixel size (mm): default 0.10; range [0.05, 0.30].
- Support buffer (mm): default 0.20.
- Connectivity: 4 (current). 8-connectivity optional in the future.

## Outputs for UI
- Island list: `id`, first/last layer, total area, current-layer area.
- Caps overlay: per-island color using contours at visible layer.
- Support debug overlay: current `supported[L]` vs `islandCandidates[L]` grid (green/orange).

## Overlay Modes (Mesh Painting)
- **Island Overlay**
  - Paints the outside of the mesh where islands exist.
  - Within each island, color ramps by height from its lowest point upward: red → orange → yellow → green → base mesh color.
  - Non-island regions remain at the base color (or lightly tinted if desired).

- **Surface Area Heat Map**
  - Think “heat map,” but the intensity represents increasing surface area instead of temperature.
  - Colors show how large the cross-sectional surface area is at each layer.
  - Two views:
    - Per‑island: within each island, map its own per‑layer area to a color scale (small → blue/green, large → yellow/orange/red).
    - Total model: rank all island layers across the model and color by the global scale.
  - The color ramp communicates increasing area (e.g., blue/green → yellow → orange → red for greatest area).

## Summary
This design defines islands strictly as pixels that are solid in the current layer and not covered by the previous layer (with buffer), assigns stable IDs from their first appearance, propagates them upward, and resolves merges with a clear parent/child rule. It is fast, deterministic, and directly controls precision via the pixel size.
