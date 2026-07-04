# DragonFruit Control API, GPU slicing & scripting

This document covers the additions on the `feat/gpu-slicer-wgpu` fork branch:

- a runtime **CPU/GPU slice backend selector** (`DF_SLICE_BACKEND`);
- an optional **localhost control API** (`DF_CONTROL_PORT`) that lets external
  scripts drive the *running* desktop instance — load meshes, orient, replicate,
  nest, query bounding boxes, and slice the live scene;
- **negative arrange distances** (parts can nest/interlock);
- an **idle-adaptive frameloop** so the 3D view stops pegging the GPU when idle;
- vendored **Elegoo** and **CTB** plugins (enables Elegoo printers incl. the
  Saturn 4 Ultra 16K, and the `.ctb` / `.goo` output formats).

---

## 1. Building

Requirements: Node 20, Rust (stable, MSVC on Windows), and the VS C++ Build
Tools (CMake + Ninja are pulled in — needed by the `manifold-csg-sys` C++ dep).
On Windows, build inside the VS developer environment so `cmake`/`link.exe` are
found:

```bat
call "...\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
```

Plugins are git submodules. A plain `git clone` does NOT fetch them, so run:

```bash
git submodule update --init plugins/elegoo plugins/ctb   # (URLs already point upstream)
npm install
npm run generate:plugin-registry        # regenerates the engine encoder registry
```

**GUI** (Tauri desktop app), with the GPU backend compiled in:

```bash
npm run tauri:build -- --features gpu
# -> src-tauri/target/release/dragonfruit-desktop.exe  (+ MSI / NSIS installers)
```

The build exits non-zero only on the final updater-signing step
(`TAURI_SIGNING_PRIVATE_KEY` unset) — the app + installers are already produced.

**CLI**:

```bash
cargo build --release --features gpu    # in rust/dragonfruit-cli
# -> rust/dragonfruit-cli/target/release/dragonfruit-cli.exe
```

---

## 2. CPU/GPU slice backend (`DF_SLICE_BACKEND`)

A single build runs either slice variant, chosen at launch:

| `DF_SLICE_BACKEND` | Backend |
|---|---|
| _unset_ / `default` / `cpu` | Full CPU path (3DAA pump, live progress, mid-slice cancel) |
| `cpu-seam` | RLE-seam CPU backend (the GPU correctness oracle) |
| `gpu` | wgpu GPU backend (requires the app/CLI built `--features gpu`) |

The GPU backend implements every AA mode (Off / Blur / Coverage / 3DAA
including cross-layer Z-blur) and, since the seam applies the engine's
post-exposure passes (XY blur brush, tail-cure LUT, dithering), its files
match the full CPU path in content **and size** — measured on a 30-copy 16K
bed: within 1.2% of the CPU file size, 0.3% mean gray difference, and the
residual is energy-neutral dither noise (net exposure delta 0.07%). Any GPU
failure falls back loudly to the full CPU engine, so output is always
produced. Perf on that bed (3DAA 4x + Z-blur r2): CPU 164 s, GPU 103 s.

**CLI:** `dragonfruit-cli slice run model.stl -o out.ctb --backend gpu`
**GUI:** launch with `DF_SLICE_BACKEND=gpu` set, or use the persistent
**Settings → Slicing → GPU Slicing** toggle (a one-time "GPU detected" prompt
offers it on first launch; the env var overrides the setting when present).

### Slice-tuning env knobs

All are command hooks for benchmarking/validation — they override the job the
frontend/CLI built, on **any** backend, so A/B runs compare identical jobs:

| Variable | Values | Effect |
|---|---|---|
| `DF_SLICE_AA_MODE` | `Off` / `Blur` / `Coverage` / `Vertical2` | force AA mode |
| `DF_SLICE_AA_LEVEL` | `2x` / `4x` / `8x` | force AA level |
| `DF_SLICE_ZBLUR_RADIUS` | `0`..`8` | force 3DAA cross-layer Z-blur radius (layers) |
| `DF_SLICE_ZBLUR_KERNEL` | `box` / `gaussian` | Z-blur kernel |
| `DF_SLICE_ZBLUR_SIGMA` | float | gaussian Z-blur sigma |
| `DF_SLICE_SEAM_POST` | `full` (default) / `lut` / `off` | seam post passes: `full` = blur+LUT+dither (engine-equivalent, needed for correct exposure on non-8-bit panels); `lut` = skip dithering (correct mean dose, smooth gray, ~35% faster, may band on quantizing panels); `off` = raw coverage (fastest, under-cures gradient tails where a cure LUT exists — drafts only) |
| `DF_GPU_MAX_WINDING_GB` | float (default 8) | VRAM cap for winding + Z-blur ring before CPU fallback |
| `DF_GPU_RUNS_CAP_M` | int | RLE run-buffer capacity (millions) |
| `DF_GPU_MAX_BANK_MB` | int | force smaller winding banks (multi-bank repro/testing) |

Details and validation results: `docs/gpu-3daa-stage-b-design.md` and the
`SeamPostProcess` comment in `rust/dragonfruit-slicing-engine/src/backend.rs`.

---

## 3. Control API (`DF_CONTROL_PORT`)

Launch the desktop app with `DF_CONTROL_PORT` set to bind a loopback HTTP server
(inert if unset). Optional `DF_CONTROL_TOKEN` requires an `X-Control-Token`
header on every request.

```bash
DF_CONTROL_PORT=8765 DF_SLICE_BACKEND=gpu dragonfruit-desktop.exe
```

### Endpoints

- `GET /health` → `{status, engine_version, gpu_feature, slice_backend_env}`
- `POST /command` body `{ "op": "<name>", "params": { ... } }`
- `POST /slice/scene` — sugar for the `slice` op (`{output_path?, download?}`)

Every `/command` returns `{ "ok": true, "result": ... }` or
`{ "ok": false, "error": "..." }` (HTTP 422 on op failure, 504 on timeout).

### Ops

Model selection: ops that act on a model take an optional `id`; without it they
use the active model (or the most-recently added).

| Op | Params | Does |
|---|---|---|
| `ping` | – | liveness + model count |
| `scene.list` | – | all models: id, name, visible, transform, world bbox |
| `scene.bbox` | – | union world AABB of visible models (mm) |
| `scene.arrange` | `scope?`, `spacing_mm?` | real high-precision SAT 2.5D nest of existing models (spacing may be negative) |
| `scene.fillPlate` | `id?`, `spacing_mm?` | real Fill-Plate: probe-and-pack copies of the model onto the plate (spacing may be negative) |
| `scene.save` | `path` | save the current scene to a `.voxl` project at `path` (no dialog) |
| `mesh.load` | `path` | import an STL/3MF/OBJ from an absolute path |
| `model.get` | `id?` | one model's details |
| `model.transform` | `id?`, `position?`, `rotation?` (radians), `scale?` | set transform (absolute) |
| `model.center` | `id?` | center on plate (keeps Z) |
| `model.dropToPlate` | `id?`, `clearance_mm?` | drop so the model sits on the plate |
| `model.replicate` | `id?`, `count`, `spacing_mm?` | duplicate into a grid |
| `model.select` / `model.delete` / `model.setVisible` / `model.rename` | `id?`, … | scene edits |
| `printer.list` | – | current profiles + the preset catalog |
| `printer.set` | `name?` \| `preset_id?` | select a printer (adds it from the catalog if needed) |
| `slice` | `output_path?` | slice the live scene via the real GUI export pipeline; honors `DF_SLICE_BACKEND` |

### Example: fill the Elegoo 16K bed and slice to `.ctb`

```bash
BASE=http://127.0.0.1:8765
post(){ curl -s -X POST "$BASE/command" -H 'Content-Type: application/json' -d "$1"; }

post '{"op":"printer.set","params":{"name":"Saturn 4 Ultra 16K"}}'
post '{"op":"mesh.load","params":{"path":"C:/models/part.stl"}}'
post '{"op":"scene.fillPlate","params":{"spacing_mm":-1}}'      # nest at -1mm
post '{"op":"slice","params":{"output_path":"C:/out/bed.ctb"}}'
```

### Notes / gotchas

- The `slice` op drives the real export view; it auto-switches `scene.mode` to
  `export` and skips the native Save-As dialog (path is supplied).
- `scene.arrange` on a single model just centers it — the SAT **nesting** loop
  needs ≥2 parts. Use `scene.fillPlate` to fill from one model.
- Errors from the frontend slice surface via the op result (`onSliceError`).

---

## 4. Negative arrange distances

The high-precision arranger and the "Arrange Distance" UI field previously
clamped spacing to `>= 0`. They now allow down to `-50mm`, so parts can
nest/interlock. Works in the UI field and via `scene.arrange` / `scene.fillPlate`
`spacing_mm`.

---

## 5. Idle-adaptive frameloop

The main `<Canvas>` (`SceneCanvas.tsx`) had no `frameloop`, so react-three-fiber
rendered every frame continuously (plus `powerPreference:'high-performance'`),
pegging the GPU even when idle. It now renders `always` during interaction
(pointer/wheel/keydown) and drops to `demand` after 2s of no input; R3F still
auto-invalidates on scene-graph changes so programmatic edits repaint.

---

## 6. Plugins (Elegoo, CTB, and enabling more)

Printer/format support ships as git submodules under `plugins/`. Their upstream
repos live under the `Open-Resin-Alliance` org; the default relative submodule
URLs resolve against the fork (404). To vendor another (e.g. `anycubic`,
`sdcp-v3`):

```bash
git config -f .gitmodules submodule.plugins/<name>.url \
  https://github.com/Open-Resin-Alliance/df-plugin-<name>
git submodule sync plugins/<name> && git submodule update --init plugins/<name>
npm run generate:plugin-registry     # complex plugins add a native encoder to the engine
# then rebuild (engine recompiles for a complex plugin)
```

- **Elegoo** (simple plugin): Elegoo printers incl. *Saturn 4 Ultra 16K*
  (output format `.ctb`), plus a `.goo` encoder.
- **CTB** (complex plugin): the native `.ctb` encoder. Required to slice any
  Elegoo/CTB printer — without it, slicing an Elegoo printer fails immediately
  ("no encoder for `.ctb`", surfaced in the UI as a cancelled slice).

Engine output formats after vendoring both: `.nanodlp`, `.ctb`, `.goo`.

---

## 7. Benchmarking

Helper scripts (control-API driven):

- `scripts/bench/fill-and-save.ps1 -Stl <path>` — load an STL, high-precision
  fill the current printer's bed, and save a reusable `bench/*.voxl`.
- `scripts/bench/bench-slice.ps1 [-Backends cpu,gpu]` — for each backend,
  launch the app with the saved `.voxl` as a launch argument (reloads the
  identical bed), time the slice, and report load/slice/size.

For an apples-to-apples backend comparison, force one AA config on both runs
(the GUI can only express Off/Blur/3DAA, and the GPU backend slices 3DAA jobs
binary): `bench-slice.ps1 -AaMode Coverage -AaLevel 4x` sets
`DF_SLICE_AA_MODE`/`DF_SLICE_AA_LEVEL` for every backend.

Observed on the reference bed (Elegoo Saturn 4 Ultra 16K, 30 packed copies of
a ~575k-tri model = 17.3M tris, RTX 3060, Coverage 4×AA forced on both):
scene load ≈ 86 s; **CPU 62.6 s → 190.1 MB**, **GPU 25.3 s → 166.4 MB** `.ctb`
(2.5×). Layer comparison via `dragonfruit-cli slice preview-layer` (decodes
`.ctb` directly): both artifacts 317 layers; on-pixel counts agree within
~2.6% and solid-pixel disagreement stays ≤ 1.8% through layer 280 (GPU is
consistently marginally thinner; only the ~1k-pixel top-sliver layers diverge
further). The GPU output is deterministic — identical layers across runs. At
the profile's own AA the paths are NOT quality-equivalent (CPU 3DAA ≈ 963 MB
vs GPU binary ≈ 79–96 MB).

### GPU robustness (large-scene crash: RESOLVED)

The "device lost on very large scenes" crash had two root causes, both fixed:

1. GUI profiles at 3DAA map to `Vertical2`, and the seam misread the AA level
   as an XY supersample factor (8x → a 24 GB winding request → VRAM
   overcommit wedge). Vertical/3DAA jobs now slice binary on the seam, and a
   total-winding cap (default 8 GB, `DF_GPU_MAX_WINDING_GB`) rejects
   pathological requests cleanly.
2. Draw chunks must be triangle-aligned (multiple of 3): a misaligned
   `draw(start..end)` reassembles every triangle from the wrong vertices
   (garbage winding). Chunks are floor-3-aligned with a 48M-element
   per-submission budget.

Hardening beyond the crash (verified with generated torture meshes):

- **Runs-cap overflow errors loudly** instead of silently truncating rows
  (`DF_GPU_RUNS_CAP_M` raises the 8M-run default).
- **Fill-budgeted render strips**: estimated fill (Σ triangle-bbox areas ×
  aa²) splits render passes into scissor row-strips so plate-stack overdraw
  can't blow the OS GPU watchdog.
- **LOUD CPU fallback**: any GPU failure (init error, run-cap overflow,
  backend error) automatically re-slices via the full CPU engine path with a
  prominent `[gpu] FALLING BACK` banner — the GPU's pathological content
  (fill-heavy, run-dense) is precisely the CPU's easy case. The output is
  always produced; check stderr to know which backend made it.
- Diagnostics at init: adapter/driver, VRAM estimate, device-lost /
  uncaptured-error callbacks; `DF_GPU_MAX_BANK_MB` reproduces any bank
  configuration for testing.
- `DF_SLICE_AA_MODE` / `DF_SLICE_AA_LEVEL` override the frontend job's AA
  before dispatch (the GUI cannot express `Coverage`), enabling like-for-like
  cross-backend benchmarks.
