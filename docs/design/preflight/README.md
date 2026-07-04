# Resin-Slicer Pre-Flight Checks — Shared Architecture

> Status: design. Grounded strictly in the multi-agent codebase investigation. Every file path and API below is cited from the findings; where the RLE stream genuinely cannot supply something, this document says so plainly rather than papering over it.

## 1. The core idea: instrument the RLE stream

Both pre-flight checks in this suite (Check 1 — resin escape; Check 2 — support/strut safety) are computed by **tapping the per-layer RLE run stream that the slicer already produces**, rather than by building any new dense 3D voxel volume. The slicer emits, for every layer, a run-length-encoded single-channel mask. That stream is the one seam both the CPU and GPU backends share, so instrumenting it once covers every backend.

The key realization from the investigation: the slicer **already computes** most of the primitives we need (`LayerAreaStatsV3`, RLE runs) and then **throws them away** at the encoder sink. Pre-flight is largely a matter of *retaining* data that already flows past a well-defined point.

### 1.1 Data flow

```
                        triangles_xyz (+ model_triangle_count split)
                                     |
                                     v
   +-------------------------- SliceBackend seam ---------------------------+
   |  CpuSliceBackend / GpuSliceBackend :: slice_layer(...)                 |
   |    -> rasterize_layer_rle (raster.rs:2285)                             |
   |    -> returns (Vec<RleRun>, LayerAreaStatsV3)                          |
   +-----------------------------------------------------------------------+
                                     |
              (per-layer, streaming) |  &runs borrowed here
                                     v
   +----------------- PER-LAYER METRICS ACCUMULATOR (new) -----------------+
   |  backend.rs:127-128  (seam/GPU path)                                  |
   |  engine.rs drain loop (default 3DAA path — MUST also be tapped)       |
   |                                                                        |
   |  Check 1: per-layer 2D distance transform  -> max escape length       |
   |  Check 2: per-layer CCL + cross-layer column tracking -> struts       |
   +-----------------------------------------------------------------------+
                                     |
                                     v
   +------------------- SIDECAR / RETURN PAYLOAD --------------------------+
   |  <output>.metrics.json sidecar   (full per-layer arrays)             |
   |  NativeSlicerPerfMetrics summary (aggregate only, rides perf IPC)    |
   +-----------------------------------------------------------------------+
                                     |
                                     v
   +------------------------ FRONTEND VISUALIZATION -----------------------+
   |  Pre-Flight report card (FloatingPanelStack)                          |
   |  2D layer-view heatmap canvas (PrintingLayerScrubPreview surface)     |
   |  3D risk columns / Z-risk profile                                     |
   |  clickable finding -> CameraFocusController + LayerSlider jump        |
   +-----------------------------------------------------------------------+
```

### 1.2 The exact instrumentation hook point

The single cleanest seam is the per-layer loop in `run_backend_to_path_with_progress` (`rust/dragonfruit-slicing-engine/src/backend.rs:122-135`). Each iteration holds one layer's `(runs, stats)` in scope after line 126, **before** `runs` is moved into `sink.consume_rle_layer(layer, runs)` at line 128.

> **Ordering hazard (from risks):** `runs` is *moved* into the sink at `backend.rs:128`. Any accumulator must borrow `&runs` between lines 127 and 128, or it must clone (which defeats the zero-copy goal). Insert the metric accumulation on line 127.

By deriving metrics directly from `&runs` we do **not** have to force `want_stats`/`requires_area_stats()` true on the encoder (`encoders/mod.rs:90`, default `false`) and we avoid the connected-component cost on the normal slice hot path.

### 1.3 The coverage gap you must not miss

`backend.rs` only covers the **opt-in** `--backend` seam/GPU path. The **default** full-feature engine (`engine::slice_with_progress_v3_to_path`, `engine.rs:4200`) with the 3DAA pump does **not** go through that driver (`backend.rs:13-15`; the CPU fallback re-enters it separately at `backend.rs:288`). A hook placed only in `backend.rs` silently misses every default-path slice. The equivalent per-layer `consume_rle_layer` feed in the engine drain loop **must be instrumented too**, or metrics vanish whenever `DF_SLICE_BACKEND == Default`.

## 2. How CPU and GPU both feed the same seam

The `SliceBackend` trait (`backend.rs:29-51`) defines `slice_layer(...) -> (Vec<RleRun>, LayerAreaStatsV3)` as the contract every backend satisfies.

- **CPU:** `CpuSliceBackend::slice_layer` (`backend.rs:187-200`) delegates to `rasterize_layer_rle`.
- **GPU:** produces its own runs/stats through the *same trait* (`backend.rs:218-298`). Note the loud CPU fallback re-runs via the default engine path (`backend.rs:288`) which bypasses the seam hook — another reason the engine drain loop must be tapped.

Because both backends hand back the identical `(runs, stats)` tuple, a metrics accumulator written against `&runs` is backend-agnostic by construction. A decorator `SliceBackend` wrapping an inner backend is a viable alternative that accumulates without touching the driver.

## 3. Streaming / memory model

- **What the RLE gives cheaply:** `RleRun { length: u32, value: u8 }` (`rle.rs:8-11`). `value` is a single exposure/coverage byte: `{0,255}` in the binary path (`raster.rs:2634`), full `0..255` in the AA/grayscale paths (`raster.rs:2387-2390`, cure LUTs `raster.rs:1457-1472`). Runs are row-major from pixel (0,0) and freely span row boundaries.
- **No dense volume for a single layer's CCL:** connected-component labeling runs on runs via union-find (`dragonfruit-islands/src/rle.rs:298-441`), allocating per-run not per-pixel. The only dense buffer is a single per-layer scratch grid (`rasterize.rs:190-191`), never 3D.
- **Honest limit on the "streaming O(one-layer) window" claim (Verdict 7, infeasible):** the *island pre-check as it exists today* materializes the **entire** RLE volume up front (`rasterize.rs:321-335` collects `Vec<RleMask>`), retains a second full volume of labels (`pipeline.rs:55,71,143`), and does a **global** Phase-3 rewrite (`pipeline.rs:124-138`) because the small-island `max_area_mm2` filter has an unbounded backward dependency. "From RLE, no dense *voxel* volume" is true; "O(one-layer) sliding window" is **not** true for the tracking/filtering stages. Design accordingly: Check 1 (distance transform) *is* genuinely one-layer-local; Check 2 (cross-layer columns) is **not** and must hold compressed per-layer state across the job.

### 3.1 Value-semantics hazards (from risks)

- Grayscale interpretation is path-dependent: `{0,255}` binary vs arbitrary `0..255` AA. A check treating `value > 0` as "solid" must decide how to weight partial-coverage pixels.
- AA `total_solid_pixels` is accumulated at supersampled resolution then divided by `aa_steps` (`raster.rs:2496,2516`). But the **runs handed to the sink are already at physical output resolution** (post-flush `emit_row`), so run-derived metrics are at output resolution — do not re-normalize them like the internal supersampled stats.

## 4. Surfacing: sidecar vs return payload

- **Full per-layer arrays -> sidecar.** Write `<output_path>.metrics.json` in Rust right after `finalize_to_path`, and add a second `std::fs::copy` in `save_print_file_from_path` (`src-tauri/src/main.rs:2690-2718`, alongside the artifact copy at `:2717`) so the sidecar travels with the saved print file. A per-layer array for a multi-thousand-layer job is too large to return over IPC on every slice.
- **Aggregate summary -> return payload.** Add fields to `NativeSlicerPerfMetrics` (`main.rs:466-483`, `#[serde(rename_all="camelCase")]`) or `NativeSliceTempPathResult` (`main.rs:646`), thread through `nativeSlicerBridge.ts:586-599` and the orchestrator benchmark (`sliceExportOrchestrator.ts:857/907`), render in `SliceMetricsDebugModal.tsx`.
- **Control API:** add a `preflight` case to `runControlCommand` (`page.tsx:8866`, switch `~:9158`) and optionally a `handle_preflight` sugar route mirroring `handle_slice_scene` (`control_server.rs:217`). The control model requires the frontend to own the staged mesh — there is no pure-Rust headless slice path.

## 5. Touched files (shared)

| File | Role in pre-flight |
|---|---|
| `rust/dragonfruit-slicing-engine/src/backend.rs` | Seam hook (line 127); trait `slice_layer` (43-47); GPU path (218-298); default-fallback caveat (288) |
| `rust/dragonfruit-slicing-engine/src/engine.rs` | **Must also tap** default 3DAA drain loop (`slice_with_progress_v3_to_path` 4200); `set_area_stats` drop points (4314) |
| `rust/dragonfruit-slicing-engine/src/rle.rs` | `RleRun{length,value:u8}` (8-11); `emit_row` (84-100) — run value semantics |
| `rust/dragonfruit-slicing-engine/src/raster.rs` | `rasterize_layer_rle` (2285); RLE-native CCL (699-836); model/support split (874-891) |
| `rust/dragonfruit-slicing-engine/src/types.rs` | `LayerAreaStatsV3` (498-509); `model_triangle_count` (202-203); `aa_on_supports` (196-197) |
| `rust/dragonfruit-slicing-engine/src/metrics.rs` | `SlicingPerfV3` (4) — where an aggregate summary rides out |
| `rust/dragonfruit-slicing-engine/src/encoders/mod.rs` | `consume_rle_layer` (42-46); `set_area_stats` (50); `requires_area_stats` gate (90) |
| `rust/dragonfruit-islands/src/rle.rs` | `rle_label_components` CCL (298-441); `rle_intersect_dilated`/`rle_subtract` |
| `rust/dragonfruit-islands/src/tracker.rs` | `IslandTracker` cross-layer column linking; merge topology (Check 2) |
| `rust/dragonfruit-islands/src/pipeline.rs` | `run_island_scan`; Phase-3 filter that **drops struts** (102-119) — bypass it |
| `src-tauri/src/main.rs` | IPC command (1471,1620,1654); perf mapping (466-483); sidecar copy point (2690-2718) |
| `src/features/slicing/tauri/nativeSlicerBridge.ts` | Return-field landing (586-599) |
| `src/features/slicing/sliceExportOrchestrator.ts` | Benchmark assembly (469,857,907) |
| Frontend viz (see per-check docs) | `PrintingLayerScrubPreview.tsx`, `CameraFocusController.tsx`, `LayerSlider.tsx`, `FloatingPanelStack.tsx`, `IslandListCard.tsx`, `overhangHeatmap.tsx` |

## 6. FEASIBILITY VERDICTS

The seven adversarial verdicts below are reproduced **verbatim**. Infeasible and approximate verdicts are **not softened** — a skeptical reader must see the honest picture before trusting any downstream design. Verdict legend: `feasible` = sound as claimed; `feasible-with-approximation` = works but the result is an approximation with named error modes; `infeasible` = the literal claim is false / not computable as stated.

| # | Verdict | Claim |
|---|---|---|
| 1 | **feasible** | CCL each print layer cheaply directly from per-layer RLE runs, without holding a full dense 3D volume |
| 2 | **feasible-with-approximation** | Link components across adjacent layers into vertical columns using the existing IslandTracker overlap machinery |
| 3 | **infeasible** | Min per-layer component pixel area over a strut column is a valid load-bearing cross-section (A_strut) |
| 4 | **infeasible** | Attribute peel/contact load to individual struts purely from per-layer masks (Voronoi / nearest-strut) |
| 5 | **feasible-with-approximation** | dragonfruit-sdf (or a small addition) can produce a per-layer 2D distance transform whose max = worst resin-escape path |
| 6 | **infeasible** | model_triangle_count split lets the rasterizer emit per-pixel support provenance to disambiguate struts from thin part features |
| 7 | **infeasible** | The whole pre-check runs streaming with an O(one-layer) sliding window and never holds the full voxel volume |

### Verdict 1 — feasible
**Claim:** You can connected-component-label each print layer cheaply directly from the per-layer RLE runs, without ever holding a full dense 3D volume.

**Evidence:**
- `dragonfruit-islands/src/rle.rs:298-441` `rle_label_components`: two-pass union-find CCL that iterates `mask.rows` (RLE runs) only. Union-find arrays grow per-run (`next_id` increments once per run, `rle.rs:361-366`), never per-pixel. It never allocates a dense grid.
- `dragonfruit-islands/src/rle.rs:375-398`: inter-row connectivity resolved by overlap-testing the current run against previous-row label runs and calling `union()` — pure interval math, no rasterization.
- `dragonfruit-islands/src/pipeline.rs:30-47`: Phase 1 maps over layers; each `scan_layer` reads only `masks[i]` and `masks[i-1]`. No 3D dense volume; working set is a stack of RLE-compressed masks.
- `dragonfruit-islands/src/scan.rs:14-40`: per-layer scan does `candidate = Current MINUS Dilate(Prev)` then `rle_label_components` — entire island pre-check runs on RLE algebra plus run-based CCL.
- `tests/worst_case.rs:332-355` and `191-215`: pass, proving CCL runs correctly on maximally fragmented RLE; comment documents the O(runs) blowup on such inputs.
- `dragonfruit-islands/src/model.rs:36-70`: `RleMask` stores rows as `Vec<Vec<RleRun{start,length}>>` — confirms label input is per-row runs, not a dense buffer.

**Failure scenario:** The 'cheaply' qualifier (not the 'no dense 3D volume' claim) is what breaks. For dithered/anti-aliased or checkerboard-like masks, RLE degenerates to ~W/2 length-1 runs per row (`worst_case.rs:333-343`), so the union-find arrays and label rows grow to O(W*H) per layer and CCL cost matches a dense 2D pass — the run-based savings vanish. This never forces a dense 3D volume (CCL is strictly 2D per layer), so the literal claim holds, but on pathological grey/dithered input it is no cheaper than dense 2D labeling. Separately, note a scope subtlety: this CCL is 2D-per-layer only; true 3D island continuity across layers is NOT done by 3D CCL but by the tracker's overlap heuristic (`min_overlap_px`/`overlap_neighborhood_px` in `tracker.process_layer`), which is an approximation of 3D connectivity, not exact union-find. Also, the shipping `rasterize.rs:191/331` densifies each single layer to `Vec<u8>` before `rle_encode`, so 'directly from RLE runs' is true of the CCL algorithm but the current producer still builds a transient dense 2D (never 3D) buffer.

### Verdict 2 — feasible-with-approximation
**Claim:** You can link components across adjacent layers into vertical columns using the existing IslandTracker cross-layer overlap machinery.

**Evidence:**
- `tracker.rs:84-157` `process_subsequent_layer` already IS cross-layer component linking: runs CCL on `solid_mask` (95), then for each component calls `find_overlapping_island_ids` against the previous layer's island labels (101-102) and assigns 0-overlap=new column, 1-overlap=continuation, N-overlap=merge (124-145).
- Computable from the RLE stream: `find_overlapping_island_ids` (`tracker.rs:320-377`) operates purely on `RleLabels` row-run lists and `RleMask` runs, never decoding to a dense grid.
- Proven correct end-to-end on RLE: `docs/ISLANDS.md:171-187` reports exact island-count parity vs the TS reference (93/93 and 14/14) on real STL models.
- APPROXIMATION #1 — splits collapse to one column: when a column bifurcates (layer N -> two disjoint components on N+1), both children overlap the single prev island so both hit `active_prev_ids.len()==1` (`tracker.rs:134-137`) and both are `update_island`'d to the SAME id. No split/branch concept.
- APPROXIMATION #2 — area corruption on split layers: `Island::update` does `per_layer_area_mm2.insert(layer, area_mm2)` (`model.rs:238`), an overwrite not a sum. Two components mapping to one island on the same layer clobber each other.
- APPROXIMATION #3 — merge parentage deferred/provisional: N-overlap merges create a placeholder and a `PendingMerge` resolved only after `merge_eval_window=30` layers (`tracker.rs:43,228-234,239-243`).
- APPROXIMATION #4 — tunable false-merge/false-break: linking uses a dilated neighborhood window with an x-overlap area proxy (`tracker.rs:343-366`) gated by `min_overlap_px` (106). Near-but-not-stacked towers can link; a pinched continuous column can break.

**Failure scenario:** A vertical column that bifurcates (Y-junction). Layer N: one solid component -> island id=5. Layer N+1: it splits into two disjoint components A and B, each overlapping island 5. Both take the `active_prev_ids.len()==1` branch (`tracker.rs:134-137`) and both call `update_island(5, N+1, ...)`. Outcome: (1) the two distinct vertical columns are indistinguishable — both carry id 5, so any pre-check needing per-column identity/count above the split gets one column instead of two; (2) `per_layer_area_mm2[N+1]` is overwritten (`model.rs:238`) to only B's area, so the column's cross-sectional area at that layer is wrong, corrupting min-cross-section or per-column volume checks. The linking machinery works, but it produces connected-component-over-Z labels, not strict per-branch vertical columns; strict column semantics require augmenting the tracker (per-component ids on splits + summing, not overwriting, per-layer area).

### Verdict 3 — infeasible
**Claim:** The minimum per-layer connected-component pixel area over a strut column is NOT a valid load-bearing cross-section (A_strut) for a tension safety factor computed from the RLE alone. The RLE stream is a stack of purely 2D, axis-aligned XY footprints (`model.rs:36-51,106-114`) with no member orientation, so the min XY area equals the true normal cross-section only for an idealized perfectly-vertical, prismatic, fully-cured strut. For the inclined/branching supports this codebase actually generates it systematically OVER-estimates the true cross-section, which makes the resulting safety factor non-conservative (unsafe) — the opposite of what a pre-check requires. It is at best a coarse, unsafe approximation, not a valid A_strut.

**Evidence:**
- `model.rs:106-114` — `ComponentInfo` carries only `area_px` (raw pixel count) plus centroid sums; there is no orientation, slope, or normal-vector field, so nothing in the RLE lets you convert an XY footprint into a cross-section normal to the member axis.
- `model.rs:36-51` — `RleMask`/`RleRun` are strictly 2D; the stream is a stack of horizontal (XY) slices only. Load-bearing area for axial tension must be measured normal to the load path; for any non-vertical strut that plane is NOT the XY plane.
- `rle.rs:353-366` — component 'area' is literally the sum of run lengths; it carries no cos(theta) correction for strut inclination.
- `rle.rs:298-307`, `scan.rs:15`, `tracker.rs:95-96` — solid components labeled with `Connectivity::Four`. A 1-2px-wide diagonal strut fragments into disconnected single-pixel components per row, so the 'component over the column' can collapse to area 1 (or vanish) — the min is a rasterization artifact.
- `rle.rs:13-49` — `rle_encode` binarizes via `data[..] != 0`, so `area_px` is threshold-dependent. The exported slicer RLE is GRAYSCALE (`slicing-engine/src/rle.rs:6-11`, anti-aliased). Edge/necking pixels are partially cured and mechanically weaker, so pixel-count area does not equal mechanical load-bearing area.
- `tracker.rs:188-237,239-314` — islands merge and branch with no first-class 'strut' object. Where two struts touch or a trunk branches, a single component spans multiple struts, so the min over the column conflates them.
- grep over `rust/` and `src/volumeAnalysis/` finds no safety-factor, A_strut, or normal-cross-section code — the method is unimplemented and the pipeline exposes no orientation data to implement it correctly.

**Failure scenario:** A tree/branch support strut (produced via curved Bezier trunks) runs at 45 deg from vertical with a circular 0.3 mm cross-section. True normal area A_true = pi*(0.15)^2 ~= 0.0707 mm^2. Its XY layer footprint is an ellipse of area A_true / cos(45) ~= 1.41 * A_true, and the minimum such footprint over the column is still ~1.41 * A_true. A tension safety factor SF = A * sigma_allow / F computed with this inflated area overstates strength by ~41% (worse at shallower angles: 3x at 70 deg). A strut that actually fails in peel/tension therefore PASSES the pre-check. Second failure mode: a 1px-wide diagonal segment under 4-connectivity fragments into disjoint single-pixel components, so the 'min component area over the column' reports ~1 px and the check either divides toward zero (false alarm) or, after an anti-aliasing fatten, hides a real neck (missed failure). Either way the min XY pixel-area is not a valid A_strut.

### Verdict 4 — infeasible
**Claim:** You can attribute peel/contact load to individual struts purely from the per-layer masks (e.g. Voronoi / nearest-strut split of the overhang area).

**Evidence:**
- The per-layer masks carry NO strut identity. `model.rs:46-70` defines `RleMask` as a purely binary run structure and `RleLabels` (`model.rs:84-99`) labels only connected-component ISLANDS, not struts.
- The 'overhang area' is itself unlabeled: `scan.rs:21-31` computes `island_candidates = Current MINUS Dilate(Prev, buffer)` — pixels with no strut attribution and no per-strut seeds.
- The slicer fuses ALL supports into a single mask. `slicing-engine/src/types.rs:199-203`: everything after `model_triangle_count` is 'support/raft geometry' as one group. `raster.rs:2231-2247` rasterizes them into one combined `support_mask` and OR's it into the model mask.
- Strut seed positions exist only in the TS support model in WORLD coordinates (`src/supports/types.ts` ContactCone/Trunk/Branch; `ContactCone/types.ts:41 pos: Vec3`). Any Voronoi requires injecting these from outside the RLE and transforming into the grid frame via GridRef (`model.rs:277-284`) — contradicting 'purely from the per-layer masks.'
- Peel/contact load is a statically-indeterminate structural quantity, not a planar area. A per-layer 2D Voronoi ignores through-Z cumulative load, support-tree topology (reactions SUM along the tree), and strut compliance/contact-tip diameter/moment arms.

**Failure scenario:** Two struts under one rigid bridged overhang plate, one near the adhesion-pressure centroid and one at a long cantilever edge. Equal-area nearest-strut Voronoi assigns each ~half the overhang area, but the true FEP peel reaction splits by span geometry, relative stiffness and moment arms — the center strut can carry the large majority. Worse cases the mask cannot express at all: (1) a strut whose contact tip lands a few layers BELOW the current overhang layer gets zero tributary area yet still bears cumulative peel load; (2) a branch strut and its parent trunk both hold the same island — planar per-layer split double-counts or misattributes because it cannot see that their loads sum along the tree; (3) since `raster.rs` merges all supports into one mask, if two struts touch within a pixel or fuse with model geometry, the 'nearest-strut' assignment is undefined because no strut labels survive into the RLE. Net: a Voronoi split yields an area proxy, and only after externally injecting strut world-positions and transforming them into the GridRef frame — it is not 'load,' and it is not computable 'purely from the per-layer masks.'

### Verdict 5 — feasible-with-approximation
**Claim:** The dragonfruit-sdf crate (or a small addition) can produce a per-layer 2D distance transform whose maximum equals the worst resin-escape path length.

**Evidence:**
- The 2D distance transform IS trivially computable from the RLE stream: `model.rs:45-70` defines `RleMask` and `rle.rs:53-71` (`rle_decode`) already expands it to a dense grid. A standard exact Euclidean/chamfer DT (Felzenszwalb-Huttenlocher, two-pass, O(pixels)) over that mask is a small addition. Its maximum exactly equals the largest inscribed-circle radius = the longest straight-line, in-plane distance from the deepest interior pixel to the nearest cross-section edge. That single-layer geometric statement is mathematically sound (Euclidean DT = valid straight escape within the layer).
- But the maximum does NOT equal the 'worst resin-escape path length' in general, because resin escape/suction is a 3D phenomenon and a per-layer 2D slice is blind to it. Decisive counterexample: enclosed cavities — a hollow model sliced through its middle presents an annular cross-section whose 2D DT max is ~wall half-thickness, while the true resin-escape path for the sealed interior is infinite/undefined (needs a vent). The hollowing + drain-hole feature exists precisely because this is a 3D drainage problem: `docs/workflows/hollowing.md`, `dragonfruit-mesh-repair/src/hollowing.rs` operate on 3D voxel volumes, not per-layer masks.
- The dragonfruit-sdf crate specifically CANNOT be reused: it is a 3D signed distance to the mesh SURFACE (`dragonfruit-sdf/src/lib.rs:1-6`, `compute.rs:181-244`, `grid.rs:36-43`), not distance-to-nearest-empty-pixel of a rasterized cross-section. Sampling on a z-plane yields distance to the nearest triangle in 3D (roofs/floors included), not the in-plane escape radius.
- The existing SDF is hard-capped and sparse: only cells within `shell_thickness` (default 8.0mm, `lib.rs:40-41`) of the surface are computed; farther cells return `f32::MAX` and are dropped. Coords stored as i16. A 20mm-deep region saturates at 8mm. The only 2D artifact it derives is a clearance heightmap (`heightmap.rs:1-9`), unrelated to in-plane escape.
- Net: 'per-layer 2D DT from RLE' = feasible and exact; 'from the dragonfruit-sdf crate' = false (wrong crate/quantity, belongs in dragonfruit-islands); 'max equals worst resin-escape path length' = only an approximation valid under a narrow single-layer radial-escape model, and outright wrong for the 3D trapped-volume/cupping cases.

**Failure scenario:** Hollowed model (hollow sphere or closed box with a thin roof) with an internal cavity not open on the layer being analyzed. On any interior layer the solid cross-section is a thin annulus/ring, so max(2D DT) is roughly the wall half-thickness (e.g. 0.5mm), reporting an easy escape. The physical truth is the opposite: resin trapped in the sealed cavity has no escape path (effectively infinite; requires a vent/drain hole). The 2D per-layer DT is structurally incapable of detecting the sealing roof one or more layers above, so its maximum diverges from the true worst resin-escape path by an unbounded margin. A secondary, milder failure: even for open geometry, the inscribed-circle radius only equals worst escape length under a straight-line radial-to-nearest-edge model; it does not capture the film-suction footprint that actually drives MSLA peel forces.

### Verdict 6 — infeasible
**Claim:** For NATIVE supports, the model_triangle_count split lets the rasterizer emit per-pixel support provenance to disambiguate struts from thin part features.

**Evidence:**
- `slicing-engine/src/rle.rs:8-11` — the emitted RLE run is `RleRun { length: u32, value: u8 }`. `value` is grayscale COVERAGE (0-255), not a provenance tag. No bit/channel/field identifies a run as support vs model. The island pre-check struct is thinner still: `dragonfruit-islands/src/model.rs:37-40 RleRun { start, length }` is pure binary geometry.
- The split does NOT emit provenance — it is destructively merged away BEFORE encoding. `raster.rs:2247-2251` combines model_mask and support_mask via max/OR. `engine.rs:625-660` `merge_support_mask_inplace_local` writes `*dst_px = 255` wherever support is set (656). After merge a lit pixel could be model, strut, or both.
- The split's ACTUAL purpose is anti-aliasing bypass, not provenance. `engine.rs:547-553`/`raster.rs:2231-2237` clone the job and force supports to AA Off/Coverage/`blur_brush_radius_px=0`/`minimum_aa_alpha_percent=100`. Comment `raster.rs:2188-2190` confirms 'bypassing AA for support/raft geometry'.
- Disambiguation happens by TRIANGLE INDEX before rasterization, consumed internally, never exported per-pixel. `raster.rs:874-891`/`engine.rs:576` route `candidate >= model_triangle_count` to the support bucket — a pre-raster routing decision, not per-pixel output.
- Pre-checks confirm no provenance downstream: island scan (`scan.rs:28-29`) derives 'supported' purely from geometric overlap between current and previous masks — it has no support-provenance input.

**Failure scenario:** A support strut pixel and a thin part-wall pixel both end up with value=255 in the merged mask, producing identical `RleRun(length, value=255)` entries — a consumer reading the RLE cannot tell strut from part. The AA-bypass design makes this strictly worse: supports are forced to hard 255 and the OR-merge (`engine.rs:656`) overwrites any model AA-edge value to 255 on overlap, so even the fallback heuristic 'gradient value 1-254 = model edge, 255 = support' fails precisely for thin part features, which are themselves sub-AA-kernel and rasterize to solid 255. Additionally the split is inert unless the caller pre-orders model triangles first AND `aa_on_supports` is false AND `0 < model_triangle_count < total` (`engine.rs:537-545`); when absent, everything is uniform 255 with zero provenance signal.

### Verdict 7 — infeasible
**Claim:** The whole resin pre-check computation runs streaming with an O(one-layer) sliding window and never holds the full voxel volume.

**Evidence:**
- REFUTED empirically: the island pre-check materializes the ENTIRE compressed volume up front. `rasterize.rs:321-335` does `(0..num_layers).into_par_iter().map(...).collect()` into `Vec<RleMask>` — every layer's RLE mask held simultaneously before scanning starts.
- `pipeline.rs:20-23` `run_island_scan` takes `masks: &[RleMask]` (the whole stack), and `pipeline.rs:55,71,143` build/return `island_labels_per_layer: Vec<RleLabels>` — a SECOND full compressed volume. Two whole-volume RLE structures coexist.
- GLOBAL dependency breaks any bounded window: `pipeline.rs:114-119,132-134` filter islands whose `max_area_mm2` is below threshold, and `pipeline.rs:82-99` computes max_area over the island's ENTIRE vertical extent. Deciding keep/zero on layer L requires having already seen its top-most layer — an unbounded backward dependency.
- `pipeline.rs:124-138` — Phase 3 is a global post-pass rewriting labels on ALL previously processed layers after the full scan. Only possible because every layer's labels were retained.
- Cross-layer tracker state is retroactive: `tracker.rs:196-205` clones a whole layer of labels into `pre_merge_labels`, kept for `merge_eval_window=30` layers. Bounded, but O(30 layers x concurrent merges) of full-layer copies — not 'one layer'.
- Top-level Tauri command confirms full-volume retention: `src-tauri/src/main.rs:1861` rasterizes all masks, `:1908` scans `&masks`, `:1940-1956` iterates EVERY layer's labels, `:1996-2006` serializes the entire label volume to the frontend.
- PARTIAL truth: 'never holds the full VOXEL volume' is defensible in the dense sense — everything is RLE and the only dense buffer is a single per-layer scratch grid (`rasterize.rs:190-191`). But RLE-compressed != one-layer window: the code holds all layers of RLE at once. 'From RLE' is fine; 'O(one-layer) sliding window' is false.
- CONTRAST proving the distinction is real: the SLICER OUTPUT path (`slicing-engine/src/pipeline.rs:739-893`/`898-1061`) IS genuinely streaming — bounded `sync_channel`, per-layer callback, returns `raw_mask_layers: None`. So streaming-from-RLE with a bounded window exists for PNG/CTB encoding, but the island pre-check deliberately does not use it because tracking + max-area filtering are not window-local.

**Failure scenario:** A model with a thin vertical spindle/spike whose per-layer footprint is below `min_island_area_mm2` for layers 0..500 but widens above the threshold at layer 520. The small-island filter (`pipeline.rs:114-119,132-134`) keys the keep/zero decision on the island's global max_area over its full height. With an O(one-layer) or even a fixed 30-layer window, layer 0's finalized island labels must be emitted and evicted long before layer 520 is observed — yet whether layer 0's pixels belong to a kept island or must be zeroed is unknown until layer 520 is seen. The implementation avoids this precisely by retaining `Vec<RleLabels>` for all layers and doing a global Phase-3 rewrite; a bounded sliding window cannot produce the correct finalized per-layer label output. (Placeholder-merge reassignment is retroactive too, but bounded to 30 layers; the max-area filter is the unbounded blocker.)

## 7. What these verdicts mean for the two checks

- **Check 1 (resin escape)** rests on Verdict 5. The 2D per-layer distance transform is *exact and cheap* from RLE (do it in `dragonfruit-islands`, NOT `dragonfruit-sdf`). Its max is an **honest proxy for single-layer lateral escape only**. It cannot see sealed cavities (a 3D drainage problem the hollowing/drain-hole feature already owns). Ship it as "worst in-plane escape path per layer," explicitly *not* "worst resin-escape path."
- **Check 2 (support safety)** rests on Verdicts 2, 3, 4, 6. Cross-layer column detection is feasible-with-approximation. But min XY area is a **non-conservative** A_strut for inclined struts (Verdict 3), per-strut load attribution from masks alone is infeasible (Verdict 4), and per-pixel strut provenance does not exist in the RLE (Verdict 6). The check must therefore output a **ratio with documented error bounds**, treat strut-vs-thin-part ambiguity as acceptable (both are real risks), and be honest that load-sharing is approximated.
- **Both checks** must respect Verdict 7: pre-flight is *from RLE* but is *not* a one-layer sliding window once cross-layer state is involved. Budget memory for retained compressed per-layer state.
