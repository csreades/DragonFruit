# Resin-Slicer Pre-Flight Checks — Shared Architecture

> Status: design. Grounded strictly in the codebase. Every file path and API below is cited from the source tree; where the RLE stream or slice geometry genuinely cannot supply something, this document says so plainly rather than papering over it.

## 0. What changed, and why (read this first)

The two pre-flight checks in this suite were originally conceived as **one mechanism**: tap the per-layer RLE run stream the production slicer emits for the whole arranged bed, and reverse-engineer both resin-escape risk and support-strut safety from those post-slice pixels. An adversarial review (the seven verdicts in §6, reproduced verbatim) demolished the support half of that plan: **verdicts 3, 4, 6, and 7 are infeasible** precisely because you cannot recover support mechanics from a fused, provenance-free pixel stack on a full bed after the metadata is gone.

The redesign **splits the suite into two separate tools with different scope, data source, and timing**:

| | **Check 1 — Resin Escape** | **Check 2 — Buildability Sweep** |
|---|---|---|
| **Scope** | Whole arranged bed | **Single part** |
| **Data source** | Cheap **partial** rasterization of the **bottom ~N layers** (default 20) | **Mesh + support primitive geometry** (not production-bed slices) |
| **Timing** | **Pre-slice** — after bed arrangement, before committing a full slice | **Reactive** (live, while supporting) + on-demand sweep |
| **Verdicts it rests on** | Verdict 5 (feasible-with-approximation) | Reframes around 3/4/6/7 by **changing the data source** (see §6.1) |
| **Core metric** | Per-layer anisotropic 2D distance transform; MAX = worst lateral escape length | Per-support tension safety factor SF = (σ_green·A_strut)/(σ_peel·A_peel) |

The unifying idea is no longer "instrument one RLE seam." It is: **give each check the cheapest data source that can honestly answer its question.** Check 1's question ("can resin escape laterally at the base?") is genuinely a per-layer 2D geometry problem, so a tiny partial raster of the bottom band answers it exactly. Check 2's question ("will this strut survive peel?") is a 3D mechanics problem about *support objects*, so it reads the support primitives directly — the objects the abandoned plan spent all its effort trying to reconstruct from pixels.

---

## 1. Check 1 — Resin Escape: pre-slice, bottom-band, whole bed

### 1.1 Scope and timing

Check 1 runs **pre-slice**, immediately after the user arranges the bed and **before** committing an expensive full slice. It does **not** consume the production RLE stream. It performs its **own cheap partial rasterization of just the bottom ~N layers** (default 20, configurable) of the arranged bed.

Why the bottom band only: the over-fill / adhesion / Z-drift problems that squeeze-flow causes bite hardest at the **base**, where the part is being pulled off the FEP layer after layer with the least cured material above to stiffen it. Rasterizing 20 layers of the arranged bed is trivially fast — no GPU, no full-job commitment, no touching the slice hot path.

### 1.2 Metric (unchanged from the original design)

Per layer, compute the **anisotropic 2D Euclidean distance transform** of the solid mask (physical pixel pitch, e.g. 14 µm X × 19 µm Y). The **maximum** of that field is the worst lateral resin-escape path length — the distance from the most landlocked interior pixel to the nearest cross-section edge. **Area is the wrong proxy** (a long thin part has large area but escapes fine; a compact blob of the same area traps resin) — see [check-1-resin-escape.md](./check-1-resin-escape.md) §1.

### 1.3 Data flow

```
   Arranged bed (post-arrange, PRE-slice)
              |
              v
   +--- CHEAP PARTIAL RASTER (bottom N=20 layers only) ---+
   |  own rasterization, CPU, no GPU, no full slice        |
   +-------------------------------------------------------+
              |  per-layer solid mask
              v
   +--- ANISOTROPIC 2D DISTANCE TRANSFORM (per layer) -----+
   |  MAX = worst lateral escape length                     |
   |  argmax + local maxima = drain-hole candidates         |
   +-------------------------------------------------------+
              |
              v
   +--- VIZ + CLOSE-THE-LOOP -----------------------------+
   |  layer-view heatmap (cool at edges, hot landlocked)   |
   |  local maxima -> Hole Punch tool                      |
   |  (src/features/hole-punching)                         |
   +------------------------------------------------------+
```

### 1.4 Honest limits (kept verbatim from the original design)

- **Blind to sealed 3D cavities.** A per-layer 2D slice through a hollow shell presents a thin annular cross-section whose DT max is ~wall-half-thickness — it reads as *easy escape* while the sealed interior is actually an un-ventable trapped volume. That is a 3D drainage problem owned by the hollowing / drain-hole feature, not a per-layer DT (Verdict 5 failure scenario).
- **v1 covers only the critical bottom band**, not large flats higher up the part. A big flat at mid-height that also over-cures is out of scope for v1.
- **Brand it accurately.** This is **"lateral bottom-layer escape,"** not "worst resin escape, full stop." The UI must carry that banner.

---

## 2. Check 2 — Buildability Sweep: per-part, reactive, mesh + support geometry

### 2.1 The root cause of the redesign

The abandoned Check 2 tried to reverse-engineer support mechanics from **post-slice pixels on a full bed after the metadata is gone**. That is why verdicts 3, 4, 6, and 7 came back infeasible — the information needed (member orientation, per-strut identity, per-strut load) is destroyed at rasterization (`raster.rs` OR-merges all supports into one mask; the emitted `RleRun{length, value:u8}` carries coverage, not provenance).

The redesign **changes the data source**: Check 2 works on a **single part**, straight from its **mesh + support primitive geometry** (the `Trunk` / `Branch` / `Leaf` / `ContactCone` objects in `src/supports/types.ts` and `src/supports/SupportPrimitives/ContactCone/types.ts`), **not** the production-bed slices. Having the primitives dissolves the infeasibility results — see §6.1 for exactly which and how.

### 2.2 The model

A support fails when FEP **peel** load exceeds what the strut's green (uncured/partially-cured) cross-section can carry in tension. Per support, at its critical layer:

```
        σ_green * A_strut        (tension the strut can carry)
   SF = --------------------  =  --------------------------------
        σ_peel  * A_peel         (peel load it must resist)
```

`SF < 1` predicts failure. **Output the ratio, colour-mapped — never a boolean.** `σ_green` (resin green strength) and `σ_peel` (effective peel stress) are **calibratable material/printer constants**. `A_strut` is the **true perpendicular minimum cross-section from the primitive geometry**; `A_peel` comes from a precomputed peel-load field (§2.4).

### 2.3 Two modes

- **(a) Reactive, while supporting.** As the user adds / moves / deletes a support, recompute *only* the affected support's SF (plus its load-sharing neighbours) and **recolour live** via `src/supports/SupportRenderer.tsx`.
- **(b) On-demand Buildability Sweep.** A full pass over one part → **report card + worst-first risk list** (following the `IslandListCard` pattern, `src/components/controls/IslandListCard.tsx`).

### 2.4 Reactive architecture — the key trick

Precompute **once** per part + orientation a **peel-load field**: per-Z-band contact area / peel demand of the part (a one-time single-part slice or geometric projection; **cached**; **invalidated on reorient**). Then:

- Each support looks up `A_peel` from the field plus its own contact footprint.
- `A_strut` = true perpendicular minimum cross-section read from the primitive geometry (segment diameters, `ContactCone` profile, inclination from the segment tangents in `src/supports/types.ts`).
- On a support edit, re-evaluate **only that support + neighbours** (cheap field lookup) and recolour live.

The heavy precompute happens once; edits are O(1-ish).

### 2.5 Load attribution is an honest approximation

Multiple supports feeding one overhang are split by **nearest-support 3D attribution, NOT a truss solve**. Named error mode: when several struts share a rigid bridged overhang, the true peel reaction splits by span geometry / relative stiffness / moment arms; nearest-support attribution can misassign the majority load to the wrong strut. This is an area proxy, not a statically-determinate load.

### 2.6 The fail-safe principle (non-negotiable)

The check **MUST be conservative / pessimistic** — it must err toward *"add more support,"* **never** toward a false *"you're fine."* The abandoned version failed **optimistic** (min XY footprint over-states true cross-section by 1.4–3× for inclined struts, Verdict 3), which for a safety check is a liability. The core safety design rule:

| Input | Bias applied | Effect on SF |
|---|---|---|
| `A_strut` (strut cross-section) | round **DOWN** (true perpendicular min, no AA/partial-cure credit) | lowers SF (pessimistic) ✓ |
| `σ_peel` (peel demand) | round **UP** | lowers SF (pessimistic) ✓ |
| `σ_green` (green strength) | conservative default | lowers SF (pessimistic) ✓ |
| Load attribution | nearest-support split, no truss relief | never credits load-sharing it can't prove ✓ |

Every approximation is biased toward a **lower** SF, i.e. toward flagging more risk. A safety check that is wrong should be wrong in the direction that adds support.

### 2.7 Two tiers

- **v1 — NATIVE** (from support primitives; reactive; tractable). Reads the `Trunk`/`Branch`/`Leaf`/`ContactCone` objects directly. Reactive recolour via `SupportRenderer.tsx`.
- **v2 — IMPORTED pre-supported** (no metadata → real 3D geometric detection). Voxelize the held single part, skeletonize / medial-axis, find thin load-bearing columns + true cross-sections. **On-demand only, NOT reactive — too heavy.** Heatmap the risky columns in a voxel/layer overlay. Flagged as v2.

### 2.8 Deliberately NOT in scope

- **Bending.** v1 is a **tension floor**; bending is the v2 refinement.
- **Exact load distribution.** Nearest-support attribution, not FEA.
- **A substitute for a real print.** This is a pre-flight risk indicator.

### 2.9 MVP build order

1. **v1 native, on-demand sweep first:** precompute the peel field, SF per support, recolour + worst-first list.
2. **Reactive incremental updates:** re-evaluate edited support + neighbours, live recolour.
3. **v2 imported detection:** voxelize / skeletonize held part, on-demand.

---

## 3. Frontend touch-points

| File | Role |
|---|---|
| `src/features/hole-punching/HolePunchPanel.tsx`, `HolePunchGizmo.tsx` | Check 1: consume drain-hole candidates (local maxima) — `placement.worldPoint` seeds the gizmo (`HolePunchGizmo.tsx:228`), `onApply` drills the vent |
| `src/supports/SupportRenderer.tsx` | Check 2 native: recolour each support by SF via `resolveSceneSupportColor` (`:1750`); map SF through a gradient like `resolvePlacementPreviewMaterial` (`:363`) |
| `src/supports/SupportPrimitives/Shaft/InstancedShaftGroup.tsx` | Bucket SF into colour bands or add a per-instance colour attribute (avoid one draw call per unique SF) |
| `src/components/controls/IslandListCard.tsx` | Pattern for the Check 2 worst-first risk list (`useFloatingPanelCollapse`, sortable rows, fly-to on select) |
| `src/supports/types.ts`, `.../ContactCone/types.ts` | Check 2 native data source: `Trunk`/`Branch`/`Leaf` segments + `ContactCone` profile/pos/normal |
| `rust/dragonfruit-islands/src/rle.rs` | Check 1: `rle_decode` (`:53-71`) to build the bottom-band mask; new anisotropic 2D DT module |

## 4. FEASIBILITY VERDICTS

The seven adversarial verdicts below are reproduced **verbatim** from the original investigation. Infeasible and approximate verdicts are **not softened** — a skeptical reader must see the honest picture before trusting any downstream design. Verdict legend: `feasible` = sound as claimed; `feasible-with-approximation` = works but the result is an approximation with named error modes; `infeasible` = the literal claim is false / not computable as stated.

> **These verdicts were rendered against the ABANDONED current-path Check 2** — the "reverse-engineer supports from the full-bed post-slice RLE stream" plan. They remain true *of that plan*. The Buildability Sweep reframe (§2) does not refute them; it **routes around them by changing the data source.** §4.1 states exactly which verdicts the reframe dissolves and how.

| # | Verdict | Claim |
|---|---|---|
| 1 | **feasible** | CCL each print layer cheaply directly from per-layer RLE runs, without holding a full dense 3D volume |
| 2 | **feasible-with-approximation** | Link components across adjacent layers into vertical columns using the existing IslandTracker overlap machinery |
| 3 | **infeasible** | Min per-layer component pixel area over a strut column is a valid load-bearing cross-section (A_strut) |
| 4 | **infeasible** | Attribute peel/contact load to individual struts purely from per-layer masks (Voronoi / nearest-strut) |
| 5 | **feasible-with-approximation** | dragonfruit-sdf (or a small addition) can produce a per-layer 2D distance transform whose max = worst resin-escape path |
| 6 | **infeasible** | model_triangle_count split lets the rasterizer emit per-pixel support provenance to disambiguate struts from thin part features |
| 7 | **infeasible** | The whole pre-check runs streaming with an O(one-layer) sliding window and never holds the full voxel volume |

### 4.1 How the Buildability Sweep reframe dissolves verdicts 3, 6, and 7 (and reframes 4)

The infeasibility results share one root cause: **trying to recover support mechanics from post-slice pixels on a full bed.** The reframe removes that constraint at the source.

- **Verdict 3 (min XY area is a non-conservative A_strut) — DISSOLVED.** The XY footprint over-states an inclined strut's true cross-section by `1/cos θ` because the RLE only has axis-aligned XY slices with no member orientation. With the **support primitive in hand**, we compute the **true perpendicular minimum cross-section directly from the geometry** (segment diameter and tangent direction in `src/supports/types.ts`; `ContactCone` profile). We never use the XY slice that overstates strength. Combined with the fail-safe rule (§2.6, round `A_strut` down), this flips the error from optimistic to conservative.
- **Verdict 6 (no per-pixel support provenance in the RLE) — DISSOLVED.** We are not reading pixels. We **have the support objects** (`Trunk`/`Branch`/`Leaf`/`ContactCone`, each with a stable `id`). There is no pixel-provenance recovery problem because provenance was never lost — `resolveSceneSupportColor(modelId, supportId)` already keys colour per support instance (`SupportRenderer.tsx:1750`).
- **Verdict 7 (can't stream the whole check in an O(one-layer) window on the full bed) — DISSOLVED.** A **single part fits in memory.** We can hold the part's volume and do genuine 3D analysis; there is no need for a bounded sliding window over a multi-thousand-layer full-bed job. The peel-load precompute (§2.4) is a one-time single-part pass, cached.
- **Verdict 4 (per-strut load attribution from masks is infeasible) — REFRAMED, not dissolved.** True peel reaction is statically indeterminate; that physics does not change. But we no longer pretend to derive it "purely from per-layer masks." We use an **explicit nearest-support 3D approximation** (§2.5) with a named error mode, biased safe (§2.6). It is honestly labelled an approximation, not a truss solve.

**Verdicts 1 and 2** (feasible / feasible-with-approximation) described RLE CCL + cross-layer column tracking. The reframe **no longer relies on them** for Check 2 — column tracking was the mechanism for reconstructing struts from pixels, which we abandoned. They remain accurate about the island-scan machinery but are no longer load-bearing for support safety.

**Verdict 5** still underpins **Check 1** unchanged: the per-layer 2D distance transform is exact and cheap; its max is an honest proxy for **single-layer lateral escape only**, blind to sealed 3D cavities.

<details>
<summary><strong>Full verbatim verdict text (§4.2–§4.8)</strong> — retained for the skeptical reader; unchanged from the original investigation.</summary>

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

</details>

## 5. What these verdicts mean for the two checks

- **Check 1 (resin escape)** rests on Verdict 5. The 2D per-layer distance transform is *exact and cheap* from a per-layer mask (do it in `dragonfruit-islands`, NOT `dragonfruit-sdf`). Its max is an **honest proxy for single-layer lateral escape only**. It cannot see sealed cavities (a 3D drainage problem the hollowing/drain-hole feature already owns). Ship it as **"lateral bottom-layer escape,"** explicitly *not* "worst resin-escape path." The pre-slice, bottom-band scope (§1) means it never touches the production RLE stream at all.
- **Check 2 (support safety)** does **not** rest on the RLE verdicts — it **routes around them by changing the data source** (§4.1). Working per-part from the support primitives dissolves verdicts 3, 6, and 7 and turns verdict 4 into an explicit, safe-biased approximation. The check outputs a **ratio, colour-mapped, never a boolean**, and obeys the fail-safe rule: every approximation is biased toward a lower SF (more support), never toward a false "you're fine."

---

## Review Round 1

Five parallel adversarial reviews of this design pack. Lenses: **[R1] Resin print-mechanics**, **[R2] Software architect (reactive perf)**, **[R3] Codebase grounding**, **[R4] Product/UX**, **[R5] Adversarial safety**. Verdicts: R3 **sound**; R1/R2/R4/R5 **needs-work**. Tally: **11 HIGH, 13 MEDIUM, 8 LOW.**

The HIGH concerns collapse into a smaller set of root problems: two are **fail-safe violations** (the model reports SAFE where it actually fails — the one thing §2.6 says must never happen), flagged independently by two lenses each; three attack the **reactive tier's cost claims**; two attack **Check 1's green-means-safe** signalling; two attack the **as-built vs modelled** and **v2 imported** blind spots.

### HIGH-severity concerns (11)

**H1 — Tension-only SF is a CEILING on safety, not a floor; slender/inclined struts report green while failing in bending. [R1, R5, R4]**
- *Evidence:* §2.2/§2.8 and check-2 §3/§9 model "axial tension survival only" and call it "a floor on failure risk (necessary-not-sufficient)." This is backwards: a tall thin or steeply-inclined strut (BezierSegment, `types.ts:112-121`) fails in bending/buckling under the asymmetric peel front at a load far below tensile capacity, so `SF_tension >> SF_true`. Two struts with identical min section but different slenderness get identical SF; only the slender one snaps. No value of `σ_peel` fixes it because bending capacity depends on length/moment-arm, which `A_strut` does not carry. The §11 palette then paints that strut green. §6 says a false "you're fine" must NEVER happen.
- *Fix:* Stop calling tension-only a "floor." Either add a slenderness/bending screen (crude `L/d` or moment-arm penalty, or multiply demand by `~1/cos θ` for inclined struts) so tall/leaning struts cannot report green, or restate the guarantee honestly: "SF>1 is necessary but NOT sufficient." Reconcile §6 and §9 — the safety claim currently promises what §9 withdraws.

**H2 — Nearest-support attribution SPLITS peel load, which IS crediting the load-sharing relief §2.6 swears it never credits; the concentrating strut is under-loaded and shown green. [R5, R1]**
- *Evidence:* check-2 §5 admits nearest-support "can misassign that majority" load; but the §2.6 / §6 fail-safe table claims load attribution "never inflates capacity" / "assigns the pessimistic share." A conserving Voronoi split necessarily under-assigns wherever it over-assigns. For a plate loading its centre strut ~80% while area splits 50/50, that strut's `A_peel` is ~0.6× true, `SF` ~1.6× too high → green while overloaded. Conserving the TOTAL load is not the same as being per-support conservative.
- *Fix:* Do not split. Assign each strut a non-partitioning upper-bound share (each strut sees its full tributary AND a worst-case concentration factor on `A_peel`, e.g. the peak reaction of its neighbourhood). At minimum, **delete the "never inflates capacity" claim** for load attribution in §2.6 — it is false as written — and relabel nearest-support an explicitly optimistic term.

**H3 — The "each edit is local / O(1-ish)" reactive premise contradicts the design's own "reactions sum along the tree"; the neighbour set is O(tree), not O(1). [R2]**
- *Evidence:* check-2 §4 says re-evaluate "only that support + neighbours … the edit is local," but §5 concedes "their reactions sum along the tree." The primitive graph forces it: `Branch.parentKnotId → Knot.parentShaftId → trunk segment`, `Leaf.parentKnotId` likewise (`types.ts:67-96,138-151`). A trunk's lower segments carry the cumulative peel of ALL descendants, so editing one leaf on a 100-leaf trunk dirties ~100 shared-segment SFs. No incremental tree-walk is specced.
- *Fix:* Drop the "local" framing for tree supports. Spec an explicit ancestor-walk (leaf → knot → parent shaft → root) that re-sums cumulative peel along the affected chain, bound to that chain + Voronoi neighbours, and state the deep/wide-tree worst case honestly.

**H4 — "Cheap bounded neighbour recompute" presupposes an incremental spatial index (Delaunay/KD) the design never specs and the repo does not have on the primitive data source. [R2]**
- *Evidence:* Load attribution is a nearest-support partition (§5); a bounded neighbour update only works with a maintained incremental Delaunay/KD over contact points. §13 leaves "attribution radius" and tie/overlap splitting open. The only in-repo nearest-attribution code (`volumeAnalysis/TerritorySystem/TerritoryTracker.ts`) is bolted to the abandoned full-bed RLE island scan and rebuilds over a materialized slice volume — not reusable for the reactive primitive path. Without a new index, each edit is a full O(supports-on-overhang) re-attribution.
- *Fix:* Specify the spatial structure explicitly (e.g. incremental Delaunay over `ContactCone.pos` projected to the down-face) and its update cost, or concede reactive attribution is a full local-region rebuild bounded by overhang, not a fixed neighbour count.

**H5 — Reactive recolour is NOT cheap: colour is an instanced-batch KEY, so one SF change re-partitions and re-uploads the whole model's support buffers. [R2, R3]**
- *Evidence:* `resolveSceneSupportColor` (`SupportRenderer.tsx:1750`) feeds colour into group keys `${modelKey}:${color}` (`:3180/:3298/:3324`); `InstancedShaftGroup` applies ONE colour uniform per mesh (`:34,143-144`) with no `instanceColor`/`setColorAt`. Any group-set change re-runs the `setMatrixAt` upload over every shaft. One edited SF invalidates → re-buckets and re-uploads O(all supports on the part), defeating the "O(1-ish) lookup" claim. §3 buries this as a touched-files bullet.
- *Fix:* Make the per-instance colour attribute (`setColorAt`/`instanceColor`) a **hard prerequisite** of the reactive tier, not an optional bullet — otherwise reactive is O(all supports) per edit regardless of SF-math cost. (Bucketing SF into ~5-8 bands preserves the existing colour-keyed batching and is the low-risk path — R3.)

**H6 — GREEN-ON-HOLLOW: Check 1 reads its greenest exactly where a hollowed part is most dangerous. [R4]**
- *Evidence:* A hollow model sliced through its body presents a thin annulus, so DT-max ≈ wall-half-thickness and the layer reports "easy escape" while the sealed interior is an un-ventable trapped volume (check-1 §2.1 banner, §1.4, Verdict 5 failure scenario). Mitigation today is only a text banner; hollowing-subsystem coordination is deferred to an open question.
- *Fix:* For v1 do not merely banner it — **suppress green when the part is hollow.** The hollowing feature already knows the part is shelled; feed that in and render the interior as "not evaluated — see drain-hole check" instead of a passing colour. Green must never appear on the geometry that most needs a vent.

**H7 — The green→amber→red palette reads "safe" for checks that each model only ONE of several failure modes. [R4]**
- *Evidence:* Check 2 is tension-only with bending explicitly out of scope (§9); Check 1 is single-layer in-plane only. A pessimistic tension floor coloured green still says "safe" when it means only "won't fail in axial tension." Numeric honesty (ratio, pessimistic rounding) does not cover out-of-model failure modes, yet the colour reads as all-clear.
- *Fix:* Use an **asymmetric palette** — risk pops (saturated amber/red), a pass RECEDES (neutral/muted, not a rewarding green). Label the pass state "no tension risk flagged," not "safe." Add a persistent, non-dismissible "What this does NOT check" line (bending, sealed cavities, flats above the bottom band). Warn-only, never reward.

**H8 — v2 imported detection can silently MISS a strut (skeletonization false-negative) and FATTEN thin struts (voxelization) — both optimistic, both unacknowledged. [R5]**
- *Evidence:* check-2 §7.2 voxelizes then skeletonizes and asserts "same conservative rounding," but a sub-voxel-diameter neck either vanishes (missed → zero flag, and its dropped share makes neighbours look fine too) or is over-thickened by partial-voxel fill (`A_strut` too large → SF inflated). Voxelization structurally cannot round `A_strut` below its own resolution, so the "conservative rounding" claim is unbacked for v2. A missed strut produces no flag at all — the ultimate false "you're fine."
- *Fix:* State the minimum reliably-detected strut diameter as a function of voxel pitch; treat everything below it as UNKNOWN/flagged (not absent); derate voxel-derived `A_strut` by the half-voxel-per-side fattening bound; add a coverage check that every down-face region has a detected column or is flagged unsupported.

**H9 — `A_strut` uses MODELLED diameter, but the as-printed strut — especially the 0.4 mm min-feature contact tip — prints thinner or fails to form. [R5]**
- *Evidence:* §2.2/§6 take `A_strut` as the geometric perpendicular min section; `ContactCone/types.ts:54-63` sets `contactDiameterMm` default 0.4 mm (already "reduced to prevent fragility"). Thin MSLA supports and near-min-feature tips print smaller/weaker than modelled, so `σ_green·A_strut(modelled)` over-states real capacity. "Round `A_strut` down" rounds the geometry, not the modelled-vs-printed gap.
- *Fix:* Apply a min-printable-feature / cure-shrinkage derate to `A_strut` (clamp effective diameter to as-printed calibration; treat tips below the printer XY-pitch minimum as capacity ≈ 0). Tie the derate to the printer XY pitch already in the slicer.

### MEDIUM-severity concerns (13, compact)

- **M1 [R1]** `σ_peel·A_peel` linearises a rate-dependent phenomenon: real FEP separation has a suction/Stefan term (~area²/gap³, velocity-dependent) and a perimeter/work-of-adhesion term; a single scalar understates large flats and ignores lift speed. → State it as a first-order area-linear proxy; calibrate against the largest-flat/worst-velocity case or add a super-linear penalty. *(Echoed by M11.)*
- **M2 [R1]** Check 1's "thick, over-cured layer / elephant's-foot" causality is muddled — an oversized gap is over-fill/Z-drift (tends toward UNDER-cure-through), and elephant's foot is bottom-layer over-exposure, not squeeze-flow. → Reframe as over-fill / Z-drift; drop "over-cure" and the elephant's-foot attribution.
- **M3 [R1]** The bottom-band (N=20) limit is justified with peel-stiffening reasoning, but the squeeze-flow metric is Z-independent — a mid-height flat squeezes identically and is silently missed. → Separate the rationales; label the bottom-band cut a **compute-cost scoping choice**, not physics, and flag tall-part mid-height flats as a known miss.
- **M4 [R2]** Invalidation is specced for reorient only, but `A_peel` also changes under hollowing, drain-hole punching, repair, and non-uniform scale (`transformSupportsForModel`, `state.ts:1591-1627`) — a field cached across those reports stale, optimistic demand. → Key the cache on a mesh-geometry hash + orientation; invalidate on any mesh mutation OR non-uniform scale.
- **M5 [R2]** The §13 open questions (field-construction method, Z-band resolution, critical-layer selection, attribution radius, reorient cost) are load-bearing on the reactive cost claim, and `σ_green`/`σ_peel` have no in-repo calibration source — an uncalibrated pessimistic default can drive the gradient all-red, training users to ignore it. → Resolve field-construction + critical-layer before committing to reactive; ship a named calibration profile with a visible "uncalibrated / indicative only" banner.
- **M6 [R3]** Straight-strut orientation is NOT on the segment — only `BezierSegment` carries tangents; `StraightSegment` (`types.ts:108-110`) has none. The axis must be reconstructed from the endpoint chain (Root/Knot base → joints → `ContactCone.pos`). → Reword §2.4/README:97: derive the member axis from segment endpoints for straight segments, tangents for Bezier; handle optional-joint (tip/root) cases.
- **M7 [R3]** Continuous per-support SF colours fragment the colour-keyed batching into one draw call per support. → Bucket SF into ~5-8 bands (preserves batching, low-risk) OR add the per-instance colour attribute (= H5). Keep this requirement front-and-centre.
- **M8 [R4]** No defined home on the path to print — the biggest discoverability risk. Today the island scan hides behind a "Run" button in `scene.mode === 'analysis'` (`page.tsx:19207-19224`); a user going arrange→support→slice never enters it. → Surface ONE Pre-Flight card at the slice gate (`scene.mode → 'printing'/'export'`, near `page.tsx:4379`) running both checks, with manual re-run.
- **M9 [R4]** Reactive-while-supporting is the wrong v1 priority and under-specified: because attribution is a nearest-support proxy, dragging one support repaints neighbours in ways that don't track physics (users can game a green board), and the SF tint contends with existing selection/hover/brace colour channels. → Ship on-demand only in v1; when reactive lands, make risk a distinct channel (emissive/outline) behind a "Show buildability" toggle, and don't repaint neighbours mid-drag.
- **M10 [R4]** Check 2's risk list is diagnostic-only — it locates a risky support but offers no fix affordance, unlike Check 1's Hole-Punch handoff. → Give the red row an action: at minimum select the support for thickening; ideally a "reinforce" affordance (bump diameter / add neighbour).
- **M11 [R5]** Peel demand is a single static scalar × linear area — it cannot be conservative across footprint sizes at once, and ignores dynamic peel spikes and super-linear suction (cupping) on large sealed flats. → Make `σ_peel`/`A_peel` footprint- and down-face-area dependent with a super-linear suction term and a lift-speed/tilt peak factor. *(Same root as M1.)*
- **M12 [R5]** Critical-layer selection is left OPEN (§13); only min-SF-over-span is fail-safe — evaluating at the min-section or contact layer can miss the peak-`A_peel` layer. → Mandate `SF = min over all layers of the span`; do not leave it an open choice.
- **M13 [R5]** How the vertical peel force resolves onto an inclined strut axis is unspecified; if demand becomes `F·cos θ` (axial component only) it is optimistic. → Specify demand as the FULL vertical peel force (do not resolve away the transverse component) until a bending term exists. *(Overlaps H1.)*

### LOW-severity concerns (8, compact)

- **L1 [R1]** Check 1 motivates its metric with adhesion/peel/"cascade" language, but DT-max ignores contact extent (a long stadium and a disk of equal DT-max have very different total peel area). → Keep peel/suction in Check 2's lane; motivate Check 1 strictly by local squeeze back-pressure / over-fill.
- **L2 [R2]** Live recolour during a reorient DRAG is unachievable — state rewrites only on commit (`page.tsx:14531`), so SF colours are stale until commit and "incremental for small rotations" is moot (commit rewrites all support geometry). → Say recolour is deferred to transform-commit; drop the small-rotation goal.
- **L3 [R3]** The peel-load field itself does not exist yet — only raw slice/voxel building blocks do (`rasterize.rs:299` produces solid masks; newly-appearing-area lives in the full-bed scan). → Keep §13 items as explicit MVP-step-1 deliverables; prototype the field as a per-part slice + down-face (`normal.z<0`) accumulator before wiring reactivity.
- **L4 [R3]** Minor citation drift: check-2 §2 points at `DEFAULT_TIP_PROFILE` (`:54-63`, values) for `contactDiameterMm`/`bodyDiameterMm` rather than the `SupportTipProfile` field defs (`:27-32`). Fields exist; repoint the citation.
- **L5 [R4]** Check 1's trigger timing is internally inconsistent ("immediately after arrange" vs the drain-hole close-the-loop that changes geometry afterward), creating a stale-result trap. → Anchor Check 1 to the slice gate (after arrange+hollow+support) and auto-invalidate after a Hole Punch or hollow edit.
- **L6 [R4]** Smallest-lovable-v1 is defined per-doc but not synthesized across the two checks, risking two disconnected UIs. → Define v1 as ONE on-demand card at the slice gate running both checks; explicitly defer reactive recolour, layer heatmap, drain-hole auto-handoff, and v2.
- **L7 [R5]** `σ_green`'s conservative default is unsourced and swings with temperature/resin age/exposure. → Pin the default to a documented lower-bound (or a large divisor over the softest supported resin) and expose it as a required calibration input.
- **L8 [R5]** A coarse peel-load Z-band can smear a concentrated down-face demand across supports, under-counting the peak. → Use MAX (not mean) peel demand within a band; set band resolution to the layer height; validate the projection variant against a cupped test part.

### Overall go / no-go per check

| Check | Verdict | Gating conditions |
|---|---|---|
| **Check 1 — Resin Escape** | **CONDITIONAL GO** | Core DT-max metric is physically sound (R1 confirms squeeze-flow/inscribed-radius theory) and the codebase building blocks exist (R3). Before build: (H6) **suppress green on hollow parts**, not just banner; (H7) asymmetric warn-only palette; (M2) fix the over-cure / elephant's-foot framing; (M3) relabel the bottom-band cut as compute scoping, not physics; (L5) anchor to the slice gate and auto-invalidate after Hole Punch / hollow. None are architectural — all are framing/UX and a hollow-state input. **Buildable now with these fixes.** |
| **Check 2 v1 — NATIVE (on-demand)** | **CONDITIONAL GO** | The data-source pivot (read primitives, not pixels) is sound and codebase-grounded — all five lenses agree it dissolves Verdicts 3/6/7 and R3 verifies the primitives carry the geometry. But two **fail-safe violations must be fixed first**: (H1) tension-only-reads-green for slender/inclined struts, and (H2) load-attribution splitting understates the concentrating strut while §2.6 falsely claims it never inflates capacity. Also resolve (M6) straight-strut orientation sourcing, (M12) min-SF-over-span, (M9) attribution proxy limits, and (M5) a labelled uncalibrated banner. With those, the **on-demand sweep is buildable** — it is a single-part batch pass with no reactivity constraint (R2 confirms). |
| **Check 2 v1 — NATIVE (reactive tier)** | **NO-GO (as specced)** | The three reactive HIGHs are unresolved: (H3) load-summing is O(tree), not local; (H4) no incremental spatial index exists or is specced; (H5) colour is a batch key so recolour is O(all supports) until a per-instance colour attribute lands. Plus (M4) the invalidation set is incomplete. The "O(1-ish) edits" claim is currently unsubstantiated. **Do not build reactive until** the ancestor-walk, spatial index, and per-instance colour attribute are specced; ship on-demand first (matches §2.9). |
| **Check 2 v2 — IMPORTED (voxelize/skeletonize)** | **NO-GO / DEFER** | (H8) unacknowledged optimistic failure modes — skeletonization can miss a strut entirely (zero flag) and voxelization fattens thin necks, neither covered by the "same conservative rounding" claim; (H9) as-printed vs modelled diameter derate is also unaddressed and hits v2 hardest at min-feature tips. Needs a min-detectable-diameter analysis, an UNKNOWN/flagged band, and a coverage check before it can honour the fail-safe rule. **Correctly last in the build order — leave deferred.** |

**Recommended build order:** ship the **Check 1 on-demand card** first (lowest risk, metric already validated, fixes are framing + a hollow-state input), then the **Check 2 v1 on-demand native sweep** once H1 and H2 are fixed and §2.6 is corrected. Defer the reactive tier and v2 until their gating specs exist.
