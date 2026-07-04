# Pre-Flight Check 1 — Lateral Bottom-Layer Resin Escape

> Status: design. See [README.md](./README.md) for the split architecture and the seven feasibility verdicts. This check rests on **Verdict 5 (feasible-with-approximation)**.
>
> **Scope in one line:** runs **pre-slice**, on the **arranged bed**, over just the **bottom ~N layers** (default 20), via a **cheap partial rasterization** — not the production RLE stream. It measures **worst lateral in-plane escape distance per layer**, explicitly *not* "worst resin escape, full stop."

## 1. The physics

On MSLA/resin printers the build plate descends toward the FEP film, squeezing a fresh layer of resin. Over a **flat, landlocked area** the resin trapped under the descending plate must **flow out laterally** to the nearest edge before the plate can reach its commanded Z.

If the lateral escape path is short (a thin wall, a spindle), resin drains out sideways and the real gap equals the commanded layer height. If the escape path is **long** — the deep interior of a large flat cross-section — the resin cannot get out in time. The resulting hydraulic **back-pressure flexes the compliant FEP downward** instead of letting the plate close. The real gap then **exceeds** the commanded layer height, producing a **thick, over-cured layer** (and, cumulatively, dimensional drift and elephant-foot-like artifacts on flat undersides).

**Why the bottom band.** These over-fill / adhesion / Z-drift effects bite hardest at the **base** of the part, where each layer is peeled off the FEP with the least cured material above to stiffen it and where adhesion failures cascade. So Check 1 only needs the bottom ~N layers (default 20, configurable) — no full slice, no GPU.

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

## 2. The metric (unchanged)

For each of the bottom N layers, compute the **2D Euclidean distance transform** of the solid mask: for every solid pixel, the straight-line distance to the nearest non-solid (empty) pixel.

- **Per-layer risk scalar = MAX of the distance field** over that layer = worst escape-path length (the inscribed-circle radius of the most landlocked region).
- Convert to physical units using the **anisotropic pixel pitch** (e.g. 14 µm X vs 19 µm Y). This must be a **per-axis weighted** distance, not an isotropic one.

### 2.1 What is MEASURED vs ASSUMED vs CALIBRATED

Foreground this separation — a skeptic must see exactly where physics stops and modelling begins.

| Category | Item | Basis |
|---|---|---|
| **Measured (from the partial raster, exact)** | Per-layer solid mask; 2D Euclidean/anisotropic distance transform; MAX distance = inscribed-circle radius of the most landlocked region; location of that maximum | Bottom-band mask → `rle_decode` (`rust/dragonfruit-islands/src/rle.rs:53-71`); new separable DT. Mathematically exact per Verdict 5. |
| **Modelling assumption** | Escape length ≈ straight radial distance to nearest edge; risk is a monotone function of that length; single-layer 2D is a stand-in for a 3D squeeze-flow problem; bottom N layers are representative of the base-adhesion-critical region | Verdict 5: the straight-line segment to the nearest background pixel is provably obstruction-free *within the layer*, but real film-suction footprint and 3D drainage are not captured. N is a scoping assumption, not a measurement. |
| **Cannot be measured (3D, out of scope)** | Sealed internal cavities (a hollow sliced mid-body reads as a thin annulus = "easy" but is actually a trapped, un-ventable volume); large flats *above* the bottom band | Verdict 5 failure scenario. Sealed cavities are a 3D drainage problem owned by the hollowing/drain-hole feature (`dragonfruit-mesh-repair/src/hollowing.rs`), NOT by a per-layer DT. Flats above layer N are simply out of v1 scope. |
| **Calibratable constant** | N (bottom-band depth, default 20); distance threshold (px/µm) above which a layer is flagged; amber vs red bands; whether to weight by cured value; per-resin/per-printer FEP compliance factor | No physical model in-repo; these are tuning knobs surfaced in the Pre-Flight card and eventually a resin/printer profile. |

> **Honesty banner (must appear in the UI):** this check measures **worst in-plane lateral escape distance per layer, over the bottom N layers only**. It is explicitly NOT "worst resin-escape path" — it is blind to sealed 3D cavities and to flats above the bottom band. Those are the drain-hole/hollowing feature's and (future) full-height job.

## 3. Algorithm (cheap partial raster, pre-slice)

Per README Verdict 5, the DT belongs in **`dragonfruit-islands`**, NOT `dragonfruit-sdf` (that crate is a 3D distance-to-mesh-surface field with an isotropic scalar `cell_size`, a hard `shell_thickness` cap, and i16-quantized coords — the wrong crate/quantity/range for pixel-resolution 2D).

1. **Rasterize the bottom band only.** After bed arrangement, before committing a slice, rasterize just the bottom N layers of the arranged bed into per-layer solid masks. This is a small, self-contained CPU pass — it does **not** run the production slice pipeline and does **not** tap the production RLE stream. Reuse the existing single-layer dense-scratch pattern (`rasterize.rs:190-191`); never build a 3D volume.
2. **Materialize each layer mask.** Decode runs to a dense per-layer `Vec<bool>`/bitpacked buffer (single layer only), or run the DT directly over run intervals.
3. **Separable anisotropic EDT.** New module `rust/dragonfruit-islands/src/distance2d.rs` implementing a two-pass Felzenszwalb-Huttenlocher / Meijster separable EDT with **per-axis weights** `(dx_um, dy_um)`.
4. **Reduce.** Track the running MAX and its `(x,y)` location as the transform is computed. Emit `{ layer, max_escape_um, argmax_xy, local_maxima: [...] }`.
5. **Accumulate** into a small per-layer metrics vector (only N entries — no multi-thousand-layer sidecar needed) returned to the frontend for the report card + heatmap.

### 3.1 Grayscale handling

If the partial raster produces grayscale coverage (`value: u8`, `0..255`), decide the solid threshold explicitly. The simplest v1 treats `value > 0` as solid (matches `rle_encode`'s `data[..] != 0` binarization, `rle.rs:13-49`). A refinement weights partially-cured edge pixels as fractionally solid, since partially-cured resin drains differently — a calibration refinement, not a v1 requirement.

## 4. Touched files

| File | Change |
|---|---|
| `rust/dragonfruit-islands/src/distance2d.rs` **(new)** | Separable anisotropic 2D EDT; per-axis `(dx_um,dy_um)` weights; MAX + argmax + local-maxima reduction |
| `rust/dragonfruit-islands/src/rle.rs` (53-71) | `rle_decode` reused to build the per-layer mask (or iterate run intervals directly) |
| `rust/dragonfruit-islands/src/rasterize.rs` (190-191) | Reuse single-layer dense-scratch pattern for the bottom-band partial raster |
| Bottom-band partial raster entry **(new)** | A pre-slice, arranged-bed, bottom-N rasterization path independent of the production slice pipeline |
| `src/features/hole-punching/HolePunchPanel.tsx`, `HolePunchGizmo.tsx` | Consume suggested drain-hole centers (local maxima) — see §5.3 |
| Pre-Flight report card (frontend) | Host the per-layer heatmap + Z-mini-profile over the bottom band |

## 5. Visualization

### 5.1 Layer-view heatmap

Overlay a per-pixel heatmap on the 2D layer preview for each bottom-band layer. Paint each pixel by its distance value through a shared ramp:

- **Cool (blue/green) near edges** — every pixel there escapes easily (low DT).
- **Hot (red) where landlocked** — the deep interior of compact flats (high DT).

Because the DT field is computed once (pre-slice) over only N layers, the frontend only *paints* precomputed scalars — it never recomputes the transform per frame.

### 5.2 Bottom-band risk mini-profile

A small 1D chart: worst-escape-distance (Y) vs layer index within the bottom band (X). Peaks mark the base layers most likely to over-cure. Clicking a peak scrubs to that layer's heatmap.

### 5.3 Drain-hole suggestion — closing the loop

The **local maxima of the distance field** are precisely the most-landlocked points — the natural places to vent. Emit the top-N local maxima (across the worst bottom-band layers) as candidate drain-hole centers and **hand them to the existing Hole Punch tool**. Feed each candidate's world XY as the gizmo placement (`HolePunchGizmo.tsx:228` positions the cutter at `placement.worldPoint`); the user tunes diameter/depth in `HolePunchPanel.tsx` and hits **Apply**. This turns a diagnostic into a one-click fix and reuses shipped drilling machinery instead of building a new path.

> Honest caveat: a local maximum of the 2D DT is the worst *in-plane* point on that layer; a true vent for a sealed 3D cavity must reach that cavity. For open flats the suggestion is directly actionable; for sealed cavities the hollowing feature's 3D analysis is authoritative. Present drain suggestions as "vent the flat interior here," not "guaranteed drainage."

## 6. MVP + build order

1. **DT core (Rust, no UI).** New `distance2d` module + anisotropic separable EDT + unit tests (thin rect → low max; compact disk → radius; ring/annulus → wall-half-thickness to document the sealed-cavity blindness). Assert exactness against a brute-force reference on small masks.
2. **Bottom-band partial raster + metrics.** Wire the pre-slice, arranged-bed, bottom-N rasterization; compute per-layer `{max_escape_um, argmax_xy}`; return the small N-entry vector to the frontend.
3. **Report card + mini-profile.** Render the bottom-band Z-profile; wire click → layer scrub.
4. **Layer heatmap overlay.** Extract the JS colormap; paint precomputed values on the layer preview.
5. **Drain-hole suggestion → Hole Punch.** Emit local maxima; wire the hand-off to `HolePunchGizmo`/`HolePunchPanel`.

MVP = steps 1-3 (a trustworthy number and a bottom-band profile). Steps 4-5 are the visual/actionable payoff.

## 7. Open questions

- **Bottom-band depth N.** Default 20 — is that the right base-critical window across resins/printers, and should it scale with layer height rather than layer count?
- **Anisotropic EDT correctness at extreme pitch ratios.** Felzenszwalb-Huttenlocher with per-axis weights is standard but needs test coverage at real 14/19 µm ratios; verify no lower-envelope degeneracy.
- **Threshold calibration.** What DT-max (in µm) actually correlates with measurable over-cure? Needs empirical per-resin/per-printer data; ships as a calibratable constant with a conservative default, not a hard physical law.
- **Grayscale weighting.** Do we count partially-cured edge pixels as solid, fractional, or empty for drainage? v1 = binary `value>0`; refinement TBD.
- **Sealed-cavity coordination.** Should Check 1 query the hollowing/3D-drainage subsystem to suppress false "easy" readings on annular cross-sections, or just carry the honesty banner? Recommend: banner in v1, cross-feature coordination later.
- **Local-maxima selection.** How many drain suggestions, and how to suppress clustered maxima (non-max suppression radius) before handing to Hole Punch?
