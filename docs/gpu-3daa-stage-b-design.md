# GPU 3DAA Stage B (cross-layer Z-blur) — design

Status: **IMPLEMENTED and validated.** Stage A (per-layer Z-SSAA) is done and
validated (commit `26f390a`). Stage B is now native on the GPU per §3.2 below
(`gpu/zblur.wgsl` + the sequential ring driver in `gpu/mod.rs`); the `af5a0a6`
CPU-fallback guard was replaced by a VRAM guard (winding + ring vs the
`DF_GPU_MAX_WINDING_GB` cap). One deviation from §3.2: the blur binds ONE ring
plane per accumulate dispatch (accumulating into `coverage_buf`, then
normalizing in place) instead of binding all 2R+1 planes at once — sidesteps
both the per-stage storage-buffer limit and any single-binding size cap, so
the ring is 2R+1 separate plane buffers.

Validation (2026-07-04, RTX 3060, 12-rifle STL, 2048×1152, 159 layers,
Vertical2 4x, GPU vs full CPU engine; `DF_SLICE_ZBLUR_*` env hooks added to
the CLI for this):
- radius 0 control: 0.019% aggregate mean-abs (the known Stage A band);
- radius 2 box: 0.016%; radius 3 gaussian: 0.018% — same band, edge layers
  show NO darkening (CPU renormalizes; GPU matches);
- blur is active and grows: GPU r0→r2 differs 0.032%, r0→r4 0.061%;
- default-path regression: GPU Coverage 4x output byte-identical pre/post;
- 16K smoke (15120×6230, radius 2): 159 layers in 2.7 s, 3 winding banks +
  5×377 MB ring, no device loss.

## 1. What Stage B is

3DAA ("Vertical2") has two separable stages:

- **Stage A — within-layer Z-SSAA (done).** Each output layer integrates
  coverage over `aa` Z sub-planes spanning the layer thickness. On the GPU this
  is free: sub-pass `j` samples plane `P_j = (L + (j+0.5)/aa)·h` in the fragment
  shader (`slice.wgsl`, `vaa` path), coupling the Y supersample lattice to the
  Z axis. No extra passes, no extra memory.
- **Stage B — cross-layer Z-blur (this doc).** *Optional*, off by default
  (`z_blur_radius_layers == 0`). When enabled, each pixel's finished grayscale
  coverage is a weighted average of the same `(x,y)` across the center layer and
  its `±R` neighbours.

Stage B only matters when the user sets a nonzero Z-blur radius. At the default
(radius 0) GPU Stage A already equals CPU 3DAA.

## 2. CPU reference algorithm (must match)

From `engine.rs` (`perturb_3daa_rle_z_blur_weights`, `apply_z_weighted_blur_to_rle_layer`)
and `types.rs`:

- **Radius** `R = effective_z_blur_radius_layers()` — `z_blur_radius_layers`
  clamped to `min(8)`, and the production RLE path further clamps to `min(6)`.
  So the GPU should cap `R` at **6**.
- **Weights** `w[d]`, `d = 0..=R` (`z_blur_kernel`, default `"box"`):
  - box: `w[d] = 1` for all d.
  - gaussian: `w[d] = round(exp(-d² / (2σ²)) · 1024)`, `σ = z_blur_sigma.max(0.01)`
    (default `z_blur_sigma = 0.5`).
- **Blend** (symmetric, same `(x,y)`):
  ```
  out(x,y) = clamp( (Σ_{d=-R..R} w[|d|]·cov(x,y,L+d) + denom/2) / denom , 0, 255)
  ```
  where the sum runs over layers that exist (`0 ≤ L+d < total`) and
  `denom = Σ of the w[|d|] that were actually included`. **Edge layers reduce
  denom (renormalize) rather than darken** — verify this against the CPU at the
  first/last few layers during validation; if the CPU instead keeps the full
  denom (darkening edges), match that.
- Order in the pipeline: Stage A raster → optional per-layer XY blur → **Stage B
  Z-blur** → tail-cure LUT → dither → support merge. The GPU seam does not do
  the LUT or dither (same as Coverage), so Stage B parity is measured on the
  pre-LUT/pre-dither grayscale, expecting the same ~1–2% band as Coverage.

## 3. GPU implementation plan

### 3.1 The core problem

The GPU produces per-layer grayscale coverage in `coverage_buf` (native res,
`width·height` u32, 0..255), then RLE-encodes it. Stage B needs a **window of
`2R+1` layers' coverage** available at once, and the output for layer `M` is
delayed until coverage for `M+R` exists.

Two entanglements to handle:
1. `coverage_buf` is a single buffer, overwritten every layer, and the
   downsample only writes the layer's **bbox** (outside the bbox is stale). A
   blur that reads a window must see **0 outside each layer's own bbox**.
2. `submit_layer`/`collect_layer` have `Empty` and `ReusePrev` fast-paths that
   skip the GPU and are expressed in terms of **RLE runs**, not dense coverage.
   Stage B needs a valid dense coverage buffer for *every* layer.

### 3.2 Chosen approach — coverage ring + weighted-blur compute pass, sequential

- **Ring buffer**: one storage buffer of `(2R+1)·width·height` u32 (a ring of
  `2R+1` native coverage planes). At 16K native (~94 M px) that is ~376 MB per
  plane → ~2.6 GB at R=3, ~4.9 GB at R=6. Fits a 12 GB card alongside the
  winding banks (~1.5 GB) and vertex buffer; if it ever doesn't, reuse the
  existing VRAM cap / CPU fallback.
- **Fill the ring without touching the hot path**: run each layer's coverage
  through the *existing* winding + downsample, but for Stage B:
  - **clear `coverage_buf` to 0 before the downsample** (so outside-bbox = 0),
    and
  - **disable the `Empty`/`ReusePrev` fast-paths** (always fully compute; an
    empty slab is a no-op winding subtract, so the downsample reproduces the
    previous coverage correctly — ReusePrev stays correct, just recomputed).
  Then `copy_buffer_to_buffer(coverage_buf → ring[c % (2R+1)])`.
  Guard all of this behind `if self.zblur_radius > 0` so the default path is
  byte-for-byte unchanged.
- **Blur pass** (`zblur.wgsl`, new compute): for each pixel in the output
  layer's bbox (= union of the window layers' bboxes), loop `d = -R..R`, skip
  out-of-range layers, accumulate `w[|d|]·ring[(M+d) % ring][idx]` and `wsum`,
  write `round(acc/wsum)` into `coverage_buf`. Uniforms: `M`, `R`, `total`,
  ring size, native dims; weights in a small uniform/storage buffer; bbox.
- **Re-RLE**: run the existing `count → prefix → write` compute over the blurred
  `coverage_buf` and read back runs (reuse one slot's `runs_buf`/`readback`).
- **Driver** (`slice_layer` when `zblur_radius > 0`): sequential, no
  submit-ahead. Track `sb_next` (next layer whose coverage is in the ring). For
  `slice_layer(M)`: fill the ring forward to `min(M+R, total-1)`, then blur the
  window → `coverage_buf`, re-RLE → runs, return. Reuses `empty_layer()` for the
  degenerate all-zero result.

### 3.3 Alternative considered

Decode each layer's Stage-A **runs → dense** on the GPU (a small expand shader)
instead of clearing `coverage_buf` + disabling fast-paths. Cleaner separation
(works purely from correct runs, no fast-path entanglement) at the cost of an
extra expand pass and a runs upload per layer. Fall back to this if the
clear-and-disable-fast-paths approach proves fragile.

## 4. New/changed code

- `gpu/zblur.wgsl` — new weighted-blur compute shader.
- `gpu/mod.rs`:
  - fields: `zblur_radius: usize`, `zblur_weights: Vec<u32>` (len R+1), the ring
    buffer, `zblur_pipeline` + bind group(s), `sb_next` cursor, a weights buffer.
  - `new_async`: compute `zblur_radius` (cap 6) and weights (box/gaussian, §2);
    allocate the ring + weights buffer + blur pipeline when `R > 0`.
  - `submit_layer`: `if zblur_radius > 0` clear `coverage_buf` first, disable
    `Empty`/`ReusePrev` shortcuts, skip the RLE/readback (coverage only), and
    copy `coverage_buf → ring`.
  - `slice_layer`: branch to the sequential Stage-B driver when `R > 0`.
- Remove the `af5a0a6` fallback guard (or narrow it to a VRAM-overflow guard).

## 5. Validation

Reuse `scripts/bench/validate-3daa.ps1`-style comparison:
1. Slice a scene at `DF_SLICE_AA_MODE=Vertical2` with a nonzero Z-blur radius on
   **GPU** vs the **full CPU engine** (`DF_SLICE_BACKEND` default) — expect the
   same ~1–2% mean-abs band as Stage A/Coverage.
2. Confirm GPU-3DAA-with-blur **differs** from GPU-3DAA-without-blur (proves the
   blur is active), and that the difference grows with radius.
3. Check edge layers (first/last `R`) specifically to confirm the denom /
   renormalization matches the CPU (no darkening — or matching darkening).
4. Confirm the default path (radius 0, Coverage, Off, Blur) is byte-identical to
   before (regression).

## 6. Effort

~150–250 lines across `zblur.wgsl` + `mod.rs`, plus 2–3 build/validate cycles.
Off-by-default, so it carries no risk to the shipped default 3DAA until enabled.
