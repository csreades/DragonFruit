# GPU slice backend (`feature = "gpu"`)

A wgpu port of `goo_cpp`'s GPU slice generator, mounted behind the engine's
`SliceBackend` seam (`src/backend.rs`). It produces per-layer `Vec<RleRun>` that
feed DragonFruit's existing streaming encoders (goo/ctb/…) unchanged.

## Design (fused slice → RLE, small readback)

Per layer:

1. **Winding render** (`slice.wgsl`) — the mesh is uploaded once. For each
   layer we render only geometry **above** the slice plane (near-plane clip at
   `slice_z`), at N× (super) resolution, and accumulate a signed winding number
   per subpixel via a fragment-stage `atomicAdd` into a VRAM storage buffer
   (`+1` front-facing, `-1` back-facing). `|winding| >= 1` ⇒ solid. This avoids
   float-blend/stencil portability issues.
2. **AA downsample** (`rle.wgsl::downsample`) — box-average each aa×aa block of
   subpixels into a native-res grayscale coverage value (0..255). This is
   DragonFruit's "Coverage" SSAA, on-GPU. `aa` comes from the `--anti-aliasing`
   level (`4x` ⇒ 4× per axis). `aa = 1` is the binary fast path.
3. **GPU RLE** (`rle.wgsl`, 3 passes) — `count_runs` (per row) → `prefix_sum`
   (exclusive scan over rows) → `write_runs` (each row writes its runs at its
   prefix offset). Order-preserving, so the output is a row-major run list.
4. **Readback** — only the compact `runs` buffer (`2·total` u32) is copied to
   the host. The dense (super-res) winding buffer never leaves VRAM. This is the
   *fused* path that the headroom analysis identified as the ~10–40× win, and
   GPU AA is where the 4×-AA headroom actually lands (near-free vs 16× CPU px).

## Build & run

```sh
# engine unit build
cargo build -p dragonfruit-slicing-engine --features gpu
# CLI with the GPU backend
cargo build -p dragonfruit-cli --release --features gpu
dragonfruit-cli slice run model.stl -o out.goo \
  --layer-height 0.025 --build-width-mm 211.68 --build-depth-mm 118.37 \
  --source-width-px 15120 --source-height-px 6230 --mirror-x \
  --backend gpu --json
```

The default build (no `gpu` feature) is unchanged; `--backend gpu` errors out
telling you to rebuild with `--features gpu`.

## STATUS: compiles, **runtime-unvalidated**

Authored on a host with **no discrete GPU**, but the whole pipeline was
executed and validated on a **software Vulkan adapter (lavapipe)** at reduced
resolution:
- binary (aa=1): convex box 0.08% pixel diff, concave cow 0.05% vs `cpu-seam`.
- **4× AA**: matches the engine's own SSAA path — white 91556 vs 91654,
  grey 1901 vs 1894, 17 grey levels both, coverage 9.8% both, mean pixel diff
  **0.07**. AA is correct.

Still validate on real hardware in this order (`cpu-seam` = binary oracle; the
**default** path = grayscale/SSAA oracle):

1. **Winding sign / `front_face`.** STL is CW-outward; pipeline uses
   `FrontFace::Cw`. If slices come out inverted (holes solid / solid hollow),
   flip the `select` in `slice.wgsl` `fs_main` or switch `front_face`.
2. **Y / mirror orientation.** Compare a mid-layer PNG (`--backend gpu` to
   `.nanodlp`, then `print read-layer`) against `--backend cpu-seam`. If flipped
   in Y or X, adjust `ay`/`ax` signs in `mod.rs` (the CPU path mirrors Y via
   `project_triangles_inplace`; verify parity).
3. **Correctness vs cpu-seam.** White-pixel counts and bounding boxes should
   match within AA differences (v0 is binary; cpu-seam AA-off is also binary).
4. **Overflow.** `runs_cap = 8M` run-slots and the winding buffer must fit
   `max_storage_buffer_binding_size`. At 16K the winding buffer is ~377 MB; if
   your adapter caps storage bindings at 128 MB, `new()` errors — see tiling
   below.
5. **Then** time it vs the CPU numbers in the session benchmark.

## Known limitations / follow-ups (roughly in ROI order)

- **AA — DONE (SSAA).** Winding is rendered at N× and box-downsampled to
  grayscale coverage; validated vs the engine's SSAA path (mean diff 0.07).
  Memory note: the super-res winding buffer is `native·aa²·4` bytes — at 16K/4×
  that's ~6 GB VRAM, so high AA at full 16K needs a big GPU or the tiling below.
  A future MSAA variant (`sample_count` + resolve) would cut that memory.
- **Per-layer `map + poll(Wait)` stalls the GPU.** Double-buffer readback and
  pipeline layer N+1's render while N's runs map back (async). Upload the mesh
  once (already done); batch several Z-slices per submit to amortize dispatch.
- **`prefix_sum` is a single serial invocation.** Fine for H≈6230; swap in
  `goo_cpp`'s Blelloch block-scan for very tall frames.
- **No tiling.** If the winding buffer exceeds the storage-binding cap, split
  the frame into horizontal strips (`x0..x1`), RLE per strip, and concatenate —
  runs are already row-major so strips stitch by row.
- **`runs_cap` overflow** clamps silently (shader guards writes; host `.min`s).
  A pathological alternating-pixel layer could exceed 8M runs — raise the cap or
  fall back to `cpu-seam` for that layer.
- **Determinism.** GPU rasterization rules vary by driver; if bit-reproducible
  slices matter, pin rasterization rules or accept small edge differences.
```
