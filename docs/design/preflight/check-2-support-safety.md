# Pre-Flight Check 2 — Support / Strut Survival

> Status: design. See [README.md](./README.md) for shared architecture and the seven feasibility verdicts. This check rests on **Verdict 2 (feasible-with-approximation)** and is bounded by **Verdicts 3, 4, and 6 (infeasible)**. Those infeasibility results are load-bearing for this design — they define exactly what the check may and may not claim.

## 1. The model

A support strut fails when the **peel force** the FEP exerts on the part above it exceeds what the strut's cross-section can carry in the green (uncured/partially-cured) state. The first-order safety factor:

```
        sigma_green * A_strut          (strength the strut can carry)
   SF = ---------------------------    ------------------------------------
        sigma_peel  * A_contact        (peel load the strut must resist)
```

- `sigma_green` — green-strength of the cured resin (calibratable, per-resin).
- `A_strut` — the strut's load-bearing cross-sectional area (the thinnest section of the column).
- `sigma_peel` — separation stress from FEP release (calibratable, per-printer/film).
- `A_contact` — the contact/overhang area the strut is holding up.

**Output a ratio, not a boolean.** `SF < 1` predicts failure; `SF` near 1 is marginal; `SF >> 1` is safe. A ratio lets the user see *how close to the edge* each strut is, and lets us be honest that the inputs are approximations (a boolean would falsely imply certainty).

## 2. Slice-derived, source-agnostic strut detection

Struts are detected **from the slice geometry**, not from support metadata — because per-pixel support provenance does not exist in the RLE (**Verdict 6, infeasible**: `model_triangle_count` drives an AA-bypass split that is destructively OR-merged into one mask before encoding; a strut pixel and a thin part-wall pixel are both just `value=255`). Detecting from slices means the check works identically for **native** and **imported/pre-supported** parts.

Pipeline (all from RLE, per **Verdict 1 feasible** and **Verdict 2 feasible-with-approximation**):

1. **RLE -> per-layer CCL blobs.** `rle_label_components` (`dragonfruit-islands/src/rle.rs:298-441`) labels each layer's solid cross-section into connected components (run-based union-find, no dense 3D volume).
2. **Cross-layer overlap -> columns.** `IslandTracker::process_layer` (`tracker.rs:48,84-157`) links each solid component to the previous layer's islands via dilated pixel-overlap (`find_overlapping_island_ids`, `tracker.rs:320-377`). Each solid vertical column becomes one `Island` with `first_layer`/`last_layer` and `per_layer_area_mm2`.
3. **Min-area over a small persistent column = A_strut candidate.** `min(island.per_layer_area_mm2.values())` gives the thinnest cross-section (`featureHooks`: one-liner; `px = round(min_mm2 / px_mm^2)`, exact inverse of `tracker.rs:76,122`).
4. **Upward merge = what it holds.** When a small column and a body fuse into one component above (`active_prev_ids.len() >= 2`, `tracker.rs:134-145`), `merge_islands` records the strut as a **child** of the body it merged into (parent chosen by highest overlap, `tracker.rs:253`, so the thin strut is reliably the child). `merge_layer = child.last_layer + 1` (`tracker.rs:211`).
5. **Load attribution.** Attribute the contact/overhang load of the merged-into body (and the overhang candidates from `scan.rs:21-31`) down to the struts beneath it.

### 2.1 Consuming the right data source

The public `run_island_scan` output is the **wrong source** — Phase 3 (`pipeline.rs:102-119`) filters out exactly the small `max_area_mm2 < min_island_area_mm2` islands (the strut population) and drops merged placeholders. **Consume `tracker.get_islands()` directly** (`tracker.rs:532`), which returns all islands including placeholders and small struts before filtering.

### 2.2 Strut classifier heuristic (new, small)

Flag an `Island` where: `status == Complete` AND `parent_id.is_some()` (merged upward) AND `max_area_mm2` below a strut threshold AND `(last_layer - first_layer)` large (persistent). All inputs already exist on the `Island` struct (`model.rs:155-176`).

### 2.3 Required tracker settings (from risks)

- **`min_overlap_px = 1`** — a 1-2px strut whose layer-to-layer overlap is below `min_overlap_px` (`tracker.rs:106`) fails to link and its column shatters into many short islands, destroying persistence/merge detection.
- **Connectivity.** The solid relabel hardcodes `Connectivity::Four` (`tracker.rs:96`), ignoring `job.connectivity`. A 1px-diagonal strut fragments under 4-conn, breaking the single-column assumption. Either force 8-connectivity for strut analysis or accept fragmentation as a known limitation.
- **End-of-run merge flush.** `evaluate_pending_merges` never runs at end-of-scan (`tracker.rs:239-246`; `finalize_islands` is a no-op, `tracker.rs:539-541`), so struts merging within the last 30 layers (`merge_eval_window`) never get `parent_id` set — "merged upward" is undetectable near the top of the part. **This is the single biggest blocker** and requires adding an explicit flush.

## 3. Two operating modes

### 3.1 Native supports
The support model exists in TS world-space (`src/supports/types.ts`, `ContactCone/types.ts:41 pos: Vec3`) and `model_triangle_count` provenance (`types.rs:202`) distinguishes model from support triangles *upstream*. Use it to:
- Cross-check slice-detected struts against known support positions (injected via `GridRef`, `model.rs:277-284`).
- **Recolor support primitives** by safety factor in the 3D view via `resolveSceneSupportColor(modelId, supportId)` (`SupportRenderer.tsx:1750`), mapping SF through a gradient like `resolvePlacementPreviewMaterial` (`363-424`).

> Provenance caveat (Verdict 6): the split is only binary model-vs-support and only per *triangle range*, never per *support instance*. Per-instance recolor needs a per-support-id map that does not exist in the slice contract — a contract change beyond the single `model_triangle_count` boundary. For v1, drive recolor from the TS support model + slice-detected SF keyed by nearest support, not from the RLE.

### 3.2 Imported / pre-supported parts
No support metadata at all. Rely on **pure geometric detection** (§2) and **heatmap the risky columns** in 3D. This is the source-agnostic path and the reason detection is slice-derived rather than metadata-derived.

## 4. The honest hard parts

These are not caveats to bury — they are the design's spine. A skeptic must see them foregrounded.

- **A_strut is a non-conservative approximation (Verdict 3, infeasible as stated).** Min XY footprint area equals the true normal cross-section only for a perfectly vertical prismatic strut. For inclined struts (this app generates curved Bezier tree supports) the XY footprint is inflated by `1/cos(theta)` — ~41% at 45 deg, ~3x at 70 deg — so SF is **overstated (unsafe)**. Mitigation: if native support orientation is available, apply a `cos(theta)` correction; for imported parts, report SF with an explicit "vertical-strut assumption; inclined struts overstate strength" warning band. Also: grayscale/AA edge pixels are partially cured and mechanically weaker, so pixel-count area over-counts load-bearing area even for vertical struts.
- **Load-sharing among multiple struts is approximated (Verdict 4, infeasible as stated).** True peel reaction is statically indeterminate (span geometry, relative stiffness, moment arms, tree topology where reactions SUM along trunks). We use a **Voronoi/equal-split** tributary-area heuristic, **not a truss solve**. This is an area proxy, not a load. It also requires injecting strut world-positions from outside the RLE (Verdict 4) — it is explicitly *not* "purely from the masks." Present shared-load SF as approximate.
- **Tension-only is a v1 floor; bending is the refinement.** v1 models axial tension survival only. Real struts also see bending/peel moments. Tension-only gives a *floor* on failure risk (necessary-not-sufficient); bending is the acknowledged v2.
- **Strut-vs-thin-part is geometrically ambiguous — and that is fine.** A slice-detected thin persistent column might be a support strut or a genuine thin part feature. We **cannot** disambiguate from RLE (Verdict 6). But **both are real risks**: a thin part feature that necks to a few pixels is *also* a fragility the user wants flagged. So the check reports "thin load-bearing column, SF = X" without asserting which it is. Ambiguity is a feature, not a bug.

## 5. Touched files

| File | Change |
|---|---|
| `rust/dragonfruit-islands/src/tracker.rs` | Consume `get_islands()` (532); **add end-of-run merge flush** (239-246); consider 8-conn for struts; run with `min_overlap_px=1` |
| `rust/dragonfruit-islands/src/model.rs` (155-176) | Read `Island` fields; optionally add a `min_area_px` field (currently derivable but lossy — `area_px` discarded at `tracker.rs:122`) |
| `rust/dragonfruit-islands/src/pipeline.rs` (102-119) | **Bypass** Phase-3 strut filtering for pre-flight (it drops exactly the strut population) |
| `rust/dragonfruit-islands/src/rle.rs` (298-441) | `rle_label_components` reused for per-layer blobs |
| `rust/dragonfruit-islands/src/scan.rs` (21-31) | Overhang candidates -> `A_contact` load source |
| Strut classifier **(new)** | Heuristic pass over `get_islands()` (§2.2); SF computation; Voronoi load-split |
| `rust/dragonfruit-slicing-engine/src/types.rs` (202) | `model_triangle_count` (native cross-check only) |
| `src/supports/SupportRenderer.tsx` (1750) | `resolveSceneSupportColor` -> SF gradient (native recolor); precedence vs selection/hover/brace colors |
| `src/supports/SupportPrimitives/Shaft/InstancedShaftGroup.tsx` (143) | Quantize SF into color buckets OR add `instanceColor` to avoid batch fragmentation |
| `src/components/controls/IslandListCard.tsx` | Reuse row/list pattern for a strut findings list |
| `src/components/scene/CameraFocusController.tsx` (20-32) | Fly-to a flagged strut column |
| `src/components/scene/IslandVoxelVisualization.tsx` / `IslandOverlay.tsx` | Sibling colored strut-column overlay (use `createCircleFromPixels`/`createBoxFromPixels`, `islandOverlayLogic.ts:343-434`) |

## 6. Visualization

- **3D risk columns.** Build lightweight instanced column geometry at each strut's centroid (`IslandMarker.centerX/centerY/baseZ`) via the ready-made `createCircleFromPixels`/`createBoxFromPixels` helpers (`islandOverlayLogic.ts:343-434`), colored by SF through the shared ramp. Respect the existing Z-clipping-plane pattern (`IslandVoxelVisualization.tsx:386-399`) so columns honor the current layer window. Build as *lightweight instanced* geometry — `IslandVoxelVisualization` is already heavy (greedy meshing + 20M-index chunking).
- **Native recolor.** Tint support primitives by SF via `resolveSceneSupportColor`. Quantize SF into buckets (green/amber/red) to avoid one draw call per unique color (`${modelId}:${color}` batch keys fragment on continuous gradients) — or add a true `instanceColor` attribute to the `Instanced*Group` meshes.
- **Findings list.** A collapsible Pre-Flight card in `FloatingPanelStack` listing struts sorted by SF ascending (worst first). Row click -> select -> `CameraFocusController` fly-to + `LayerSlider` jump to the strut's `first_layer`/min-area layer (with `zOffsetMm`/`layerHeightMm` correction, `IslandListCard.tsx:239`).

## 7. MVP

1. **Detection core (Rust).** `get_islands()` consumer + strut classifier (§2.2) + `min_overlap_px=1` + **end-of-run merge flush** (the critical fix). Output per-strut `{first_layer, last_layer, min_area_px, min_area_mm2, parent_id}`.
2. **SF computation.** Compute `SF = (sigma_green * A_strut) / (sigma_peel * A_contact)` with equal-split load attribution and calibratable `sigma_*` defaults. Emit as a ratio to the sidecar + aggregate (worst SF, count `SF<1`) to the return payload.
3. **Findings list + fly-to.** Pre-Flight card, sorted worst-first, click -> camera + layer jump.
4. **3D risk columns / native recolor.** Bucketed SF coloring.

MVP = steps 1-2 (trustworthy per-strut ratios in a sidecar). The end-of-run merge flush in step 1 is mandatory or near-top struts are silently missed.

## 8. Open questions

- **End-of-run flush semantics.** How to finalize `PendingMerge`s at scan end without a 30-layer look-ahead — flush immediately at `last_layer` or with a shorter window? Affects near-top strut detection reliability.
- **A_contact definition.** Is contact load the overhang-candidate area (`scan.rs`), the merged-into body's cross-section, or something cumulative through Z? Verdict 4 says cumulative through-Z load is the physical truth but not mask-derivable — decide the v1 proxy.
- **Load-split algorithm.** Equal-split vs Voronoi vs stiffness-weighted — and the strut positions Voronoi needs come from the TS support model (native) but are unavailable for imported parts. What is the imported-part fallback (equal split among columns under the same body)?
- **Inclination correction.** For native supports, can we recover per-strut orientation to apply the `cos(theta)` fix (Verdict 3)? If not, how prominent must the "overstates strength" warning be?
- **Calibration.** `sigma_green` and `sigma_peel` have no in-repo source. Per-resin/per-printer profiles, or a single conservative default with a manual override?
- **8-conn vs 4-conn.** Forcing 8-connectivity fixes diagonal-strut fragmentation but diverges from the shipping island scan's 4-conn semantics — run strut analysis as a separate pass or unify?
- **Bifurcation corruption (Verdict 2 #2).** `per_layer_area_mm2` is overwritten (not summed) at split layers (`model.rs:238`), corrupting min-area for bifurcating columns. Does strut min-area need the tracker augmented to sum per-layer areas, or is the strut population (which merges *up*, not splits) largely unaffected?
