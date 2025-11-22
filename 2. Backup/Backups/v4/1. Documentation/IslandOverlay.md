# Island Overlay (Soft-Brush) Design

## Goal
Provide a clear, per-island visual indicator that behaves like a soft round brush stamped at each island’s lowest point. The indicator is independent of layer count and fades smoothly in 3D space.

## Summary
- Each island is detected by the IslandScanner (pixel-based, 4- or 8-connectivity).
- For visualization, we treat the island base as a set of seed pixels and render a 3D soft-brush halo from those seeds.
- Strength is computed from true 3D distance (XZ radial + vertical above base), within a fixed radius (default 2.0 mm).
- Painting is single-pass and triangle-aware to avoid stacking logic.

## Inputs
- Grid and per-layer island masks from the scanner.
- Per-pixel arrays:
  - `firstHit` (Int16Array): first island layer index for each pixel or -1 if never island.
  - `lastHit` (Int16Array): last island layer index for each pixel or -1.
  - `baseLabels` (Int32Array): 2D component labels of the base footprint (all pixels that were ever islands; 4-connectivity by default).
  - `compBase` (Int16Array): per-label base layer (min `firstHit`).
  - `compTop` (Int16Array) [optional for hard clamping]: per-label top layer (max `lastHit`).
- Geometry to paint (THREE.BufferGeometry) and its `bbox` for world Y mapping.

## Soft-Brush Concept
- Define seed set per island label: pixels where `firstHit == compBase[label]`.
- Precompute seed positions in world XZ for each label (sampling/striding to control cost).
- For each triangle (by centroid):
  1. Map centroid (x, z) to grid index; read `label`.
  2. If `label <= 0`, skip (outside island).
  3. Compute base plane Y for this label: `baseY = bbox.min.y + compBase[label] * layerHeightMm`.
  4. If centroidY < baseY, skip (below base).
  5. Compute nearest 2D seed distance dXZ; vertical distance dY = max(0, centroidY - baseY).
  6. 3D distance: `d3 = sqrt(dXZ^2 + dY^2)`.
  7. Strength: `u = clamp01(1 - d3 / brushRadiusMm)`; then apply smoothstep: `s = u*u*(3 - 2u)`.
  8. Lerp vertex colors from baseColor to tint by `s`.

This produces a round, soft “halo” centered at each island’s base, fading out over a fixed metric radius.

## Parameters
- `brushRadiusMm` (default 2.0 mm): size of the soft-brush halo.
- `connectivity` (scanner) 4 or 8: defines component connectivity on the base footprint.
- `px_mm` (scanner): pixel size for rasterization; affects base footprint resolution.
- `support_buffer_mm` (scanner): dilation size for support computation.

## Rendering Behavior
- Single-pass, triangle-aware painting (centroid test) to avoid overdraw stacking.
- Optional gamma on strength (default off for soft-brush; can use 1.2–1.6 to emphasize mid-tones if needed).
- Optional clamp to island vertical extent using `compTop` (skip triangles above label’s top).
- Anti-aliasing (optional): multi-sample ~3–5 points per triangle centroid, average strength to soften boundaries.

## Performance Considerations
- Seed sampling: stride seeds per label (e.g., cap at ~128 samples) to bound search cost.
- Use squared distances in search loop; take sqrt once at the end.
- Early reject if no seeds or centroid outside label.
- Color attribute ensured once; painting sets `needsUpdate = true` only once per pass.

## Integration Plan
1. Scanner (unchanged logic)
   - Run via `ScanOrchestrator` to produce: `grid`, `layers`, `firstHit`, `lastHit`, `baseLabels`, `compBase`, `compTop`.
2. Overlay module (new file)
   - Export a `applyIslandSoftBrush(...)` API that accepts:
     - geometry, baseColor, grid, baseLabels, firstHit, compBase, bbox.y offset (min.y), layerHeightMm, brushRadiusMm, tint.
   - Keep this separate from page code; no UI state inside.
3. Page wiring
   - Page triggers scan (button) and stores results.
   - When overlay is enabled, call the overlay function once after clearing to base.

## Defaults and Tuning
- Brush radius: 2.0 mm (can expose UI knob later).
- Tint: `#ff1744` by default; always lerped from base mesh color.
- Pixel size: 0.10 mm default; decreasing improves island boundary fidelity.
- Support buffer: 0.20 mm default.

## Future Extensions
- Per-island gradient from base to top using compBase/compTop (layer-aware), selectable alongside soft-brush.
- Hybrid: max(strong brush near base, height gradient above).
- Outline pass at base layer for crisp base indication.
- GPU shader path for real-time tweakable overlays.

## Rationale
This approach matches the visual intent: “as if each island base is touched with a soft round brush,” independent of layer counts. It is robust, intuitive, and directly controllable with millimeters. The logic is fully modular, keeping page components simple.
