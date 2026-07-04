# GPU slice backend (wgpu)

A GPU implementation of DragonFruit's slice generator behind a pluggable
backend seam. On a filled plate it slices 1.6–4× faster than the CPU engine
at equivalent output quality, and it degrades safely: **any GPU failure falls
back loudly to the full CPU engine**, so a slice request always produces
correct output.

Status: feature-gated (`--features gpu` on `dragonfruit-slicing-engine`,
`src-tauri`, and the CLI). **The default CPU path is untouched** — every
change is behind the seam, and binary-output seam paths are verified
byte-identical before/after each post-processing change.

Headline numbers (30-part filled 16K plate, 15120×6230 px, 317 layers,
0.05 mm, RTX 3060 12 GB / Vulkan):

| Configuration | CPU engine | GPU backend |
|---|---|---|
| Coverage 4x | 62.6 s | 25.3 s (2.5×) |
| 3DAA 4x (Stage A) | 147 s | 36 s (4×) |
| 3DAA 4x + Z-blur r2, full post | 163.8 s → 1406 MB | 103.3 s → 1423 MB (1.6×) |

Output parity for the last row (every layer compared pixel-by-pixel):
file sizes within 1.2%, mean gray difference 0.31%, zero pixels lit by the
GPU that the CPU leaves dark, and the residual is energy-neutral dithering
noise (the worst layer's differences are +1 palette step on 13.81 M pixels
and −1 step on 13.81 M pixels — net exposure delta 0.07%).

## Architecture

### The seam (`backend.rs`)

`trait SliceBackend` produces one layer of `Vec<RleRun>` at a time;
`run_backend_to_path_with_progress` drives any backend into the existing
streaming encoders (nanodlp/ctb/goo/…). Two implementations:

- `CpuSliceBackend` — wraps the existing scanline rasterizer; the
  correctness oracle for the GPU (identical driver, only `slice_layer`
  differs).
- `gpu::GpuSliceBackend` — the wgpu backend below.

`run_gpu_with_cpu_fallback` wraps GPU runs: init errors (no adapter, VRAM
caps), mid-slice errors (run-buffer overflow), and panics all fall back to
the full CPU engine with a logged reason. Run-buffer overflow additionally
carries the measured need, so the driver grows the buffers and retries
on-GPU once before surrendering.

### GPU pipeline (`gpu/mod.rs`, `slice.wgsl`, `downsample.wgsl`, `rle.wgsl`)

Per layer, entirely in VRAM until the final compact runs:

1. **Winding accumulation** — the mesh cross-section is rasterized into a
   persistent super-resolution i32 winding buffer (fragment-stage atomic
   add). Layers are *incremental*: layer L only draws the triangles whose Z
   range overlaps its slab (per-layer CSR built once), subtracting the slab
   from the previous layer's winding. AA renders at native resolution with
   one jittered sub-pass per subpixel (texture dims cap at 32768 < 16K·aa).
2. **Downsample** — box-filter the winding to native-resolution grayscale
   coverage (0..255), cropped to the layer's bbox.
3. **GPU RLE** — three compute passes (count → prefix-sum → write) compact
   coverage into row-major runs; only the runs are read back. Three layers
   are kept in flight so the GPU never idles on readback.

Device-safety measures (validated against torture content):
- winding memory is split into **row banks** that each fit the device's max
  storage-binding size;
- draw calls are chunked triangle-aligned and **fill-budgeted** (estimated
  fragments per submission) so no submission exceeds the OS GPU watchdog
  (~2 s TDR), regardless of triangle count *or* per-triangle fill;
- total winding + ring memory is capped (`DF_GPU_MAX_WINDING_GB`, default
  8 GB) — an oversized request fails cleanly into the CPU fallback instead
  of wedging the device.

### 3DAA on the GPU

- **Stage A — within-layer Z-SSAA** (`slice.wgsl`): the aa Y-supersample
  sub-rows each sample a distinct Z plane spanning the layer thickness, so
  XY level `aa` also yields `aa` Z sub-samples per layer — no extra passes
  or memory. Costs ~nothing over Coverage.
- **Stage B — cross-layer Z-blur** (`zblur.wgsl`): when
  `z_blur_radius_layers > 0`, every layer's coverage is archived into a ring
  of 2R+1 native planes; the output layer is the weighted window average
  (box/gaussian weights identical to the CPU's, radius clamped ≤ 6, absent
  neighbours renormalize the denominator), re-RLE'd by the existing passes.
  The blur binds one ring plane per accumulate dispatch, staying inside
  per-stage storage-buffer limits at any radius. Design + validation:
  `docs/gpu-3daa-stage-b-design.md`.

### Post-exposure passes at the seam

The engine applies post passes after rasterization that backends don't
produce: the 3DAA spatial blur brush, the tail-cure LUT (minimum cure dose
for faint gray), and Floyd–Steinberg dithering (sub-level gradients on
non-8-bit panels). `SeamPostProcess` replicates them at the seam using the
engine's own RLE-level functions, on a worker pool overlapping GPU slicing.
Without these, grayscale output under-cures gradient tails on quantizing
panels — this is a print-correctness issue, not cosmetics (measured: a
3-bit panel's cure LUT boosts gray 50 → 126).

`DF_SLICE_SEAM_POST` selects `full` (default, engine-equivalent) / `lut`
(skip dithering: correct mean dose, ~35% faster, may band on quantizing
panels) / `off` (raw coverage, drafts only). Jobs that configure no
blur/LUT/dither skip the pass entirely and are byte-identical to the raw
backend output.

## Validation methodology

Reproducible without a GUI (CLI + env overrides, any STL):

```
# identical jobs on both backends
set DF_SLICE_AA_MODE=Vertical2& set DF_SLICE_AA_LEVEL=4x& set DF_SLICE_ZBLUR_RADIUS=2
dragonfruit-cli slice run model.stl -o cpu.nanodlp --backend default
dragonfruit-cli slice run model.stl -o gpu.nanodlp --backend gpu
# decode any layer of either archive to PNG and diff
dragonfruit-cli slice preview-layer cpu.nanodlp -l 42 -o cpu-42.png
```

Acceptance evidence gathered this way (all 317 layers of the 16K bench
compared): per-layer difference counts, an implied transfer-curve analysis
proving remaining differences are a value remap rather than geometry, and
signed-difference balance proving dither-noise energy neutrality. Env knobs
are documented in `docs/control-api.md` §2.

## GUI integration

- Persistent **Settings → Slicing → GPU Slicing (Experimental)** toggle
  (probes the adapter, shows its name, disabled when no usable GPU).
- One-time **"GPU detected — enable GPU acceleration?"** startup prompt
  (`detect_gpu` Tauri command; software rasterizers report unavailable).
- `DF_SLICE_BACKEND` env var overrides the setting (power-user hook).
- Seam slices report live per-layer progress through the normal
  `slicer://progress` stream.

## Known limitations

- **Mid-slice cancel is not honored** by seam backends (the full CPU path's
  cancel works as before).
- **`aa_on_supports=false` is not implemented at the seam**: seam backends
  rasterize the full triangle list uniformly, so supports receive AA when
  the model does. The full engine's model/support split is untouched.
- `LayerAreaStatsV3` are not computed by the GPU backend (returned as
  defaults when an encoder requests them).
- Stage B runs a sequential driver (no 3-layer submit-ahead); ~2× the
  per-layer cost of Stage A. The ring fill could keep the pipelined path —
  known headroom, not yet needed.
- Very sparse plates favor the CPU (its cost scales with solid content; the
  GPU's per-layer costs are largely fixed). The GPU pays off on busy plates.
