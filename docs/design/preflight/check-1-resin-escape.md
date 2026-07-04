# Pre-Flight Check 1 — Resin Escape / FEP-Flex

> Status: design. See [README.md](./README.md) for shared architecture and the seven feasibility verdicts. This check rests on **Verdict 5 (feasible-with-approximation)**.

## 1. The physics

On MSLA/resin printers the build plate descends toward the FEP film, squeezing a fresh layer of resin. Over a **flat, landlocked area** the resin trapped under the descending plate must **flow out laterally** to the nearest edge before the plate can reach its commanded Z.

If the lateral escape path is short (a thin wall, a spindle), resin drains out sideways and the real gap equals the commanded layer height. If the escape path is **long** — the deep interior of a large flat cross-section — the resin cannot get out in time. The resulting hydraulic **back-pressure flexes the compliant FEP downward** instead of letting the plate close. The real gap then **exceeds** the commanded layer height, producing a **thick, over-cured layer** (and, cumulatively, dimensional drift and elephant-foot-like artifacts on flat undersides).

### 1.1 Why AREA is the wrong proxy

The intuitive metric — "large flat area = bad" — is wrong. A long thin rectangle has large area but every interior pixel is close to an edge, so resin escapes fine. A compact blob of the *same area* traps resin in its center. **What matters is the worst escape-path length, i.e. the distance from the most-landlocked interior pixel to the nearest cross-section edge.** That is exactly the maximum of the per-layer 2D distance transform.

```
   thin part (large area, escapes fine)     compact blob (small area, traps resin)
   ####################################              ##########
   ####################################            ##############
   ^ every pixel <=1px from an edge              ################
                                                   ##############    <- center pixel
                                                     ##########         is FAR from any edge
   DT max ~ 1 px  -> LOW risk                     DT max ~ R px  -> HIGH risk
```

## 2. The metric

For each layer, compute the **2D Euclidean distance transform** of the solid mask: for every solid pixel, the straight-line distance to the nearest non-solid (empty) pixel.

- **Per-layer risk scalar = MAX of the distance field** over that layer = worst escape-path length (the inscribed-circle radius of the most landlocked region).
- Convert to physical units using the **anisotropic pixel pitch** (e.g. 14 um X vs 19 um Y — see the sdf crate's anisotropy gap in the findings). This must be a **per-axis weighted** distance, not an isotropic one.

### 2.1 What is MEASURED vs ASSUMED vs CALIBRATED

Foreground this separation — a skeptic must see exactly where physics stops and modelling begins.

| Category | Item | Basis |
|---|---|---|
| **Measured (from RLE pixels, exact)** | Per-layer solid mask; 2D Euclidean/anisotropic distance transform; MAX distance = inscribed-circle radius of the most landlocked region; location of that maximum | `RleMask` -> `rle_decode` (`dragonfruit-islands/src/rle.rs:53-71`); new separable DT. Mathematically exact per Verdict 5. |
| **Modelling assumption** | Escape length ≈ straight radial distance to nearest edge; risk is a monotone function of that length; single-layer 2D is a stand-in for a 3D squeeze-flow problem | Verdict 5: the straight-line segment to the nearest background pixel is provably obstruction-free *within the layer*, but real film-suction footprint and 3D drainage are not captured. |
| **Cannot be measured (3D, out of scope)** | Sealed internal cavities (a hollow sliced mid-body reads as a thin annulus = "easy" but is actually a trapped, un-ventable volume) | Verdict 5 failure scenario. This is a 3D drainage problem owned by the hollowing/drain-hole feature (`dragonfruit-mesh-repair/src/hollowing.rs`), NOT by a per-layer DT. |
| **Calibratable constant** | Distance threshold (px/mm) above which a layer is flagged; amber vs red bands; whether to weight by cured value; per-resin/per-printer FEP compliance factor | No physical model in-repo; these are tuning knobs surfaced in the Pre-Flight card and eventually a resin/printer profile. |

> **Honesty banner (must appear in the UI):** this check measures **worst in-plane lateral escape distance per layer**. It is explicitly NOT "worst resin-escape path" — it is blind to sealed 3D cavities. Those are the drain-hole/hollowing feature's job.

## 3. Algorithm (from the RLE)

Per README Verdict 5, the DT belongs in **`dragonfruit-islands`**, NOT `dragonfruit-sdf` (that crate is a 3D distance-to-mesh-surface field with an isotropic scalar `cell_size`, a hard `shell_thickness` cap, and i16-quantized coords — the wrong crate/quantity/range for pixel-resolution 2D).

1. **Tap the seam.** At `backend.rs:127` (and the engine drain loop — see README §1.3), borrow `&runs` for the layer before the move at line 128.
2. **Materialize the layer mask.** Either decode runs to a dense per-layer `Vec<bool>`/bitpacked buffer (single layer only — never 3D; matches the existing transient dense-2D pattern at `rasterize.rs:190-191`), or run the DT directly over run intervals.
3. **Separable anisotropic EDT.** New module `rust/dragonfruit-sdf/src/distance2d.rs` (or better, `dragonfruit-islands/src/distance2d.rs`) implementing a two-pass Felzenszwalb-Huttenlocher / Meijster separable EDT with **per-axis weights** `(dx_um, dy_um)`. Reuse the rayon parallel-row idiom from `heightmap.rs:218-248` (rows then columns).
4. **Reduce.** Track the running MAX and its `(x,y)` location as the transform is computed. Emit `{ layer, max_escape_um, argmax_xy, local_maxima: [...] }`.
5. **Accumulate** into the per-layer metrics vector; write to the `<output>.metrics.json` sidecar and an aggregate (max over all layers, worst layer index) into the return payload.

> **Do NOT reuse** the existing 11-byte i16 3D cell wire format (`grid.rs:128-130`) — a pixel grid at mm/14um can exceed i16 range. Reuse only the LE header/body *pattern* (`heightmap.rs:89-155`) for a fresh 2D result blob.

### 3.1 Grayscale handling

Runs are grayscale (`value: u8`, `0..255`). Decide the solid threshold explicitly (README §3.1): the simplest v1 treats `value > 0` as solid (matches `rle_encode`'s `data[..] != 0` binarization, `rle.rs:13-49`). A refinement weights partially-cured edge pixels as fractionally solid, since partially-cured resin drains differently — but that is a calibration refinement, not a v1 requirement.

## 4. Touched files

| File | Change |
|---|---|
| `rust/dragonfruit-slicing-engine/src/backend.rs` (127-128) | Borrow `&runs`, feed the DT accumulator (seam/GPU path) |
| `rust/dragonfruit-slicing-engine/src/engine.rs` (drain loop) | **Also** tap the default 3DAA path or the check silently no-ops on default slices (README §1.3) |
| `rust/dragonfruit-islands/src/distance2d.rs` **(new)** | Separable anisotropic 2D EDT; per-axis `(dx_um,dy_um)` weights; MAX + argmax + local-maxima reduction |
| `rust/dragonfruit-islands/src/rle.rs` (53-71) | `rle_decode` reused to build the per-layer mask (or iterate run intervals directly) |
| `rust/dragonfruit-slicing-engine/src/metrics.rs` (4) | Add aggregate (worst-layer escape, worst-layer index) to `SlicingPerfV3` or a sibling return |
| `src-tauri/src/main.rs` (2690-2718) | Write + copy `<output>.metrics.json` sidecar next to the saved print file |
| `src/features/slicing/tauri/nativeSlicerBridge.ts` (586-599) | Type the new aggregate return field (camelCase must match) |
| `src/components/controls/PrintingLayerScrubPreview.tsx` (186-205) | Host the per-pixel heatmap canvas overlay (world->canvas mapping already exists) |
| `src/features/shaders/mesh/overhangHeatmap.tsx` (114-137) | Extract its 5-stop ramp into a reusable JS colormap for the 2D canvas |
| `src/components/controls/LayerSlider.tsx` (86) + `CameraFocusController.tsx` (20-32) | Jump-to-worst-layer + fly-to from a finding |
| `src/components/layout/FloatingPanelStack.tsx` | Host the Pre-Flight report card |
| Drain-hole loop: existing **Hole Punch** tool | Consume suggested drain-hole centers (local maxima) — see §5.3 |

## 5. Visualization

### 5.1 Layer-view heatmap

Overlay a per-pixel heatmap on the 2D layer preview. Mount a new `<canvas>` inside the printing preview viewport (`page.tsx ~20180-20220`), decode the current layer, and paint each pixel by its distance value through the shared ramp.

- **Cool (blue/green) near edges** — every pixel there escapes easily (low DT).
- **Hot (red) where landlocked** — the deep interior of compact flats (high DT).

Reuse the mirror-aware, DPR-aware, RAF-batched world->canvas transform in `PrintingLayerScrubPreview.tsx:186-205`. Heed the coordinate gotcha: the scan grid's `originZ` stores `-Y` (`islandOverlayLogic.ts:17-27`) — a reconciliation util between scan-grid pixels and canvas space does not yet exist and must be written carefully or the overlay silently offsets.

> **Performance (from risks):** a naive full-grid decode per scrub frame will jank. Reuse the existing RAF batching + layer quantization + bounded caches. The DT field itself is computed once at slice time and shipped in the sidecar — the frontend only *paints* a precomputed scalar, it does not recompute the transform per frame.

### 5.2 Z-risk profile

A 1D chart: worst-escape-distance (Y) vs layer index / Z height (X), from the per-layer MAX array. Peaks mark the layers most likely to over-cure. Clicking a peak jumps `LayerSlider` (`emitChange`, `LayerSlider.tsx:86`) to that layer and flies the camera via `CameraFocusController` (generalized to accept a `{position, layer}` target, `CameraFocusController.tsx:20-32`). Convert indices with the `zOffsetMm`/`layerHeightMm` correction (`IslandListCard.tsx:239`) or the finding lands on the wrong layer.

### 5.3 Drain-hole suggestion — closing the loop

The **local maxima of the distance field** are precisely the most-landlocked points — the natural places to vent. Emit the top-N local maxima (per worst layers) as candidate drain-hole centers and **hand them to the existing Hole Punch tool**, which already creates cylindrical openings for drainage/venting (`docs/workflows/hollowing.md`). This turns a diagnostic into a one-click fix and reuses shipped machinery instead of building a new drilling path.

> Honest caveat: a local maximum of the 2D DT is the worst *in-plane* point on that layer; a true vent must reach a sealed 3D cavity. For open flats the suggestion is directly actionable; for sealed cavities the hollowing feature's 3D analysis is authoritative. Present drain suggestions as "vent the flat interior here," not "guaranteed drainage."

## 6. MVP + build order

1. **DT core (Rust, no UI).** New `distance2d` module + anisotropic separable EDT + unit tests (thin rect -> low max; compact disk -> radius; ring/annulus -> wall-half-thickness to document the sealed-cavity blindness). Assert exactness against a brute-force reference on small masks.
2. **Seam tap + sidecar.** Wire `backend.rs:127` **and** the engine drain loop; write `<output>.metrics.json` with per-layer `{max_escape_um, argmax_xy}`; copy sidecar on save.
3. **Aggregate return + Z-risk profile.** Thread worst-layer summary through the perf payload; render the 1D profile in a Pre-Flight card; wire click -> layer jump + camera.
4. **Layer heatmap overlay.** Extract the JS colormap; mount the canvas; solve the scan-grid<->canvas coordinate bridge; paint precomputed values.
5. **Drain-hole suggestion -> Hole Punch.** Emit local maxima; wire the hand-off.

MVP = steps 1-3 (a trustworthy number and a Z-profile you can click). Steps 4-5 are the visual/actionable payoff.

## 7. Open questions

- **Anisotropic EDT correctness at extreme pitch ratios.** Felzenszwalb-Huttenlocher with per-axis weights is standard but needs test coverage at real 14/19 um ratios; verify no lower-envelope degeneracy.
- **Threshold calibration.** What DT-max (in um) actually correlates with measurable over-cure? Needs empirical per-resin/per-printer data; ships as a calibratable constant with a conservative default, not a hard physical law.
- **Grayscale weighting.** Do we count partially-cured edge pixels as solid, fractional, or empty for drainage? v1 = binary `value>0`; refinement TBD.
- **Sealed-cavity coordination.** Should Check 1 actively query the hollowing/3D-drainage subsystem to suppress false "easy" readings on annular cross-sections, or just carry the honesty banner? Recommend: banner in v1, cross-feature coordination as a later integration.
- **Which resolution.** Runs are at physical output resolution post-`emit_row` (README §3.1) — confirm the DT runs at output px, not supersampled AA px, to avoid a pitch mismatch.
- **Local-maxima selection.** How many drain suggestions, and how to suppress clustered maxima (non-max suppression radius) before handing to Hole Punch?
