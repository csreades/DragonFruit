# Pre-Flight Check 2 — Buildability Sweep (Support Safety)

> Status: design. See [README.md](./README.md) for the split architecture and the seven feasibility verdicts.
>
> **This is a full redesign.** The original Check 2 tried to reverse-engineer support mechanics from the full-bed, post-slice RLE stream and **failed adversarial review** — verdicts 3, 4, 6, and 7 came back infeasible. The redesign **changes the data source**: work per **single part**, from the **mesh + support primitive geometry**, not the production-bed slices. §2 explains exactly how that dissolves verdicts 3/6/7 and reframes 4.

## 1. Root cause of the redesign

The abandoned plan's root failure was **reverse-engineering support mechanics from post-slice pixels on a full bed after the metadata is gone.** By the time the RLE exists:

- **Orientation is lost** — the stream is axis-aligned XY footprints, so min XY area *over-states* an inclined strut's true cross-section by `1/cos θ` (Verdict 3, non-conservative → unsafe).
- **Per-strut identity is gone** — `raster.rs` OR-merges all supports into one mask; the emitted `RleRun{length, value:u8}` is coverage, not provenance (Verdict 6).
- **The full bed doesn't fit a bounded window** — the island scan already holds the whole compressed volume and does a global filter pass (Verdict 7).

The redesign removes the constraint that created all three: it stops reading pixels and reads the **support objects themselves**.

## 2. How the new data source dissolves the infeasibility verdicts

Scope: **single part**, from **mesh + support primitive geometry** (`Trunk`/`Branch`/`Leaf` in `src/supports/types.ts:128-152`; `ContactCone` in `src/supports/SupportPrimitives/ContactCone/types.ts:39-48`), **NOT** the production bed slices.

- **Verdict 3 (min XY area is a non-conservative A_strut) — DISSOLVED.** We no longer use the XY slice. We compute the **true perpendicular minimum cross-section directly from the primitive**: each `Segment` carries a `diameter` and, for a `BezierSegment`, the tangent/control geometry (`types.ts:101-119`) that gives the member axis; the `ContactCone` profile gives the tip cross-section (`contactDiameterMm`/`bodyDiameterMm`, `ContactCone/types.ts:54-63`). Cross-section normal to the member axis is a direct geometric computation, not a rasterization artifact.
- **Verdict 6 (no per-pixel support provenance) — DISSOLVED.** We **have the support objects**, each with a stable `id`. There is no pixel-provenance recovery problem because provenance was never lost — `resolveSceneSupportColor(modelId, supportId)` in `src/supports/SupportRenderer.tsx:1750` already addresses colour per support instance.
- **Verdict 7 (can't hold the full-bed volume in a bounded window) — DISSOLVED.** A **single part fits in memory.** We hold the part's volume and do genuine 3D analysis; the peel-load precompute (§4) is a one-time single-part pass, cached.
- **Verdict 4 (per-strut load attribution is statically indeterminate) — REFRAMED, not dissolved.** True peel reaction *is* indeterminate; that physics is unchanged. We do not pretend to solve it. We use an **explicit nearest-support 3D approximation** (§5) with a named error mode, biased safe (§6).

## 3. The model

A support fails when the **peel force** the FEP exerts on the part above it exceeds what the strut's cross-section can carry in the green (uncured/partially-cured) state. Per support, at its **critical layer**:

```
        sigma_green * A_strut          (tension the strut can carry)
   SF = ---------------------------    ------------------------------------
        sigma_peel  * A_peel           (peel load the strut must resist)
```

- `sigma_green` — green strength of the cured resin. **Calibratable material constant** (per-resin).
- `A_strut` — **true perpendicular minimum cross-section** from the primitive geometry (§2, Verdict 3 dissolved).
- `sigma_peel` — effective peel/separation stress from FEP release. **Calibratable printer/film constant.**
- `A_peel` — the peel demand at this support, from the precomputed peel-load field (§4) plus the support's own contact footprint.

**Output the ratio, colour-mapped — never a boolean.** `SF < 1` predicts failure; `SF` near 1 is marginal; `SF >> 1` is safe. A ratio lets the user see *how close to the edge* each support is, and keeps us honest that the inputs are approximations (a boolean would falsely imply certainty).

## 4. Reactive architecture — the key trick

The reactive mode must recolour supports **live** as the user edits, without re-slicing the part on every drag. The trick is to move all the heavy work into a **one-time precompute** and make each edit an O(1-ish) lookup.

**Precompute once per part + orientation — the peel-load field.** A per-Z-band map of the part's contact area / peel demand (a one-time single-part slice or geometric projection). **Cached. Invalidated on reorient** (a reorientation changes which faces are down-facing and therefore the entire peel demand). This is where verdict 7's "single part fits in memory" pays off — it is a bounded, cacheable single-part pass, not a full-bed stream.

**Per-support evaluation (cheap).**
1. `A_peel` ← look up the peel-load field at the support's contact Z-band + the support's own contact footprint (`ContactCone.pos`/profile).
2. `A_strut` ← true perpendicular minimum cross-section from the primitive geometry (segment diameters + tangents; `ContactCone` profile).
3. `SF = (sigma_green · A_strut) / (sigma_peel · A_peel)`.

**On a support edit (add / move / delete).** Re-evaluate **only that support + its load-sharing neighbours** (a cheap field lookup + a nearest-support recompute for the affected overhang region) and recolour live. The heavy precompute already happened; the edit is local.

## 5. Load attribution — an honest approximation

Multiple supports feeding one overhang are split by **nearest-support 3D attribution, NOT a truss solve.**

- **Named error mode.** Under one rigid bridged overhang plate, one support near the adhesion-pressure centroid and one at a long cantilever edge do **not** split load equally — the true reaction depends on span geometry, relative stiffness, and moment arms, and can load the centre support with the large majority. Nearest-support attribution can misassign that majority. It is an area proxy, not a statically-determinate load.
- **Tree topology.** Where a branch and its parent trunk both feed the same overhang, their reactions sum along the tree; nearest-support attribution approximates this and can double-count or misattribute at the junction.

This is deliberately not FEA. It is a fast, explainable proxy — and, per §6, it is always biased so its error adds support rather than removing it.

## 6. The fail-safe principle (non-negotiable)

This is the **core safety design rule.** The check MUST be conservative / pessimistic — it must err toward *"add more support,"* **never** toward a false *"you're fine."* The abandoned version failed **optimistic** (min XY footprint over-states true cross-section 1.4–3×), which for a safety check is a liability.

Every approximation is biased toward a **lower** SF:

| Quantity | Bias | Where it happens | Effect on SF |
|---|---|---|---|
| `A_strut` | round **DOWN** — use the true perpendicular *minimum* section; give no strength credit for partial-cure / AA edge material | §2, §4 step 2 | lowers SF ✓ |
| `sigma_peel` | round **UP** — conservative peel demand | calibration default | lowers SF ✓ |
| `sigma_green` | conservative (low) default | calibration default | lowers SF ✓ |
| Load attribution | never credit load-sharing relief the model can't prove; assign the pessimistic share | §5 | never inflates capacity ✓ |

A safety check that is wrong should be wrong in the direction that adds support. This is why the tension-only, nearest-support, minimum-section model is acceptable as a v1 **floor**: it is necessary-not-sufficient, and it fails safe.

## 7. Two tiers

### 7.1 v1 — NATIVE (from support primitives; reactive; tractable)

The support model exists in TS world-space: `Trunk`/`Branch`/`Leaf` (`src/supports/types.ts:128-152`), each a `SupportEntity` with `Segment[]` and a terminal `ContactCone` (`ContactCone/types.ts:39-48`). This is the full v1 data source:

- `A_strut` from segment diameters + tangents + cone profile (true perpendicular section).
- `A_peel` from the peel-load field + `ContactCone.pos` contact footprint.
- **Reactive recolour** of each support by SF via `resolveSceneSupportColor` (`SupportRenderer.tsx:1750`), mapping SF through a gradient like `resolvePlacementPreviewMaterial` (`:363`).

### 7.2 v2 — IMPORTED pre-supported (no metadata → real 3D geometric detection)

For imported pre-supported parts there are no support primitives. Fall back to **pure 3D geometric detection** on the held single part:

1. **Voxelize** the part (single-part scope makes this tractable — Verdict 7 dissolved).
2. **Skeletonize / medial-axis** to find thin load-bearing columns.
3. Compute **true cross-sections** normal to each column's axis (same conservative rounding, §6).

**On-demand only, NOT reactive — too heavy.** Heatmap the risky columns in a voxel/layer overlay. Flagged as v2.

## 8. Two operating modes

- **(a) Reactive, while supporting.** As the user adds/moves/deletes a support, recompute the affected support's SF (+ load-sharing neighbours) and **recolour live** (§4). Native tier only.
- **(b) On-demand Buildability Sweep.** A full pass over one part → **report card + worst-first risk list** (following the `IslandListCard` pattern, `src/components/controls/IslandListCard.tsx`). Both tiers.

## 9. Deliberately NOT in scope

- **Bending.** v1 models axial tension survival only — a **floor** on failure risk (necessary-not-sufficient). Bending/peel moments are the acknowledged **v2 refinement**.
- **Exact load distribution.** Nearest-support attribution, not a truss/FEA solve (§5).
- **A substitute for a real print.** This is a pre-flight risk indicator, not a physics simulator.

## 10. Touched files

| File | Change |
|---|---|
| `src/supports/types.ts` (128-152), `.../ContactCone/types.ts` (39-63) | Data source: `Trunk`/`Branch`/`Leaf` segments (diameter + tangent) and `ContactCone` profile for `A_strut`; `ContactCone.pos` for contact footprint |
| Peel-load field precompute **(new)** | One-time per part+orientation; cached; invalidated on reorient (§4) |
| SF + nearest-support attribution **(new)** | Per-support ratio; nearest-support 3D load split (§5); fail-safe rounding (§6) |
| `src/supports/SupportRenderer.tsx` (1750, 363) | `resolveSceneSupportColor` → SF gradient (native recolour, reactive); precedence vs selection/hover/brace colours |
| `src/supports/SupportPrimitives/Shaft/InstancedShaftGroup.tsx` | Quantize SF into colour buckets OR add a per-instance colour attribute to avoid one draw call per unique SF |
| `src/components/controls/IslandListCard.tsx` | Reuse row/list + `useFloatingPanelCollapse` pattern for the worst-first risk list |
| v2 imported detection **(new)** | Voxelize + skeletonize held single part; risky-column heatmap overlay (on-demand) |

## 11. Visualization

- **Native (v1) — reactive recolour.** Tint each support primitive by SF (green → amber → red) via `resolveSceneSupportColor` (`SupportRenderer.tsx:1750`). Bucket SF into bands to avoid one draw call per unique color (batch keys fragment on continuous gradients) — or add a true per-instance colour attribute to the `Instanced*Group` meshes (`InstancedShaftGroup.tsx`). Recolour updates live as supports are edited.
- **Native (v1) — risk list.** A collapsible Buildability card following `IslandListCard` (`src/components/controls/IslandListCard.tsx`): supports sorted by SF ascending (worst first), row click → select + fly-to the support. Reactive: the list reorders as edits change SFs.
- **Imported (v2) — column heatmap.** Heatmap the detected risky columns in the voxel/layer overlay, coloured by SF. On-demand only.

## 12. MVP build order

1. **v1 native, on-demand sweep first.** Precompute the peel-load field; compute `A_strut` (true perpendicular section) + `A_peel` per support; `SF = (sigma_green·A_strut)/(sigma_peel·A_peel)` with fail-safe rounding; recolour + worst-first list. Emit per-support `{supportId, SF, criticalLayer, A_strut, A_peel}`.
2. **Reactive incremental updates.** Cache the peel field; on a support edit re-evaluate only that support + neighbours; live recolour + list reorder.
3. **v2 imported detection.** Voxelize/skeletonize the held part; risky-column heatmap; on-demand only.

MVP = step 1 (trustworthy per-support ratios + recolour + list on demand). Steps 2 and 3 layer reactivity and imported-part coverage on top.

## 13. Open questions

- **Peel-load field construction.** One-time single-part slice vs geometric down-face projection — which is cheap enough to precompute yet accurate enough for `A_peel`? What Z-band resolution?
- **Critical-layer selection.** Is the critical layer the minimum-section layer, the maximum-`A_peel` layer, or the minimum-SF layer of the support's span?
- **`A_strut` from Bezier segments.** Recovering the true perpendicular min section along a curved trunk — sample the tangent at min-diameter, or integrate along the segment?
- **Nearest-support attribution radius.** How far does a support's tributary region extend on a bridged overhang, and how are ties/overlaps split (§5)?
- **Calibration.** `sigma_green` and `sigma_peel` have no in-repo source. Per-resin/per-printer profiles, or a single conservative default with manual override? Whatever the default, it must be biased per §6.
- **Reorient invalidation cost.** How expensive is recomputing the peel-load field on reorient, and can it be incremental for small rotations?
