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

The non-default backends drive the shared RLE streaming encoder and **bypass the
3DAA pump** — faster, but the output is not pixel-equivalent to the full CPU
path (e.g. a filled 16K bed emits ~10× less data on GPU than CPU).

**CLI:** `dragonfruit-cli slice run model.stl -o out.ctb --backend gpu`
**GUI:** launch with `DF_SLICE_BACKEND=gpu` set.

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

Observed (Elegoo Saturn 4 Ultra 16K, 30 packed copies of a ~560k-tri model,
RTX 3060): scene load ≈ 86 s; **CPU** (full 3DAA) slice ≈ 92.5 s → ≈ 963 MB
`.ctb`. Smaller beds (24 copies): CPU ≈ 792 MB vs GPU ≈ 79 MB (GPU seam path
skips 3DAA — outputs are not quality-equivalent).

### Known issue: GPU backend crashes on very large scenes

On the full 30-copy 16K bed the GPU backend hard-crashes the process with
`Queue::submit: Validation Error / Parent device is lost` (wgpu 0.20, Vulkan).
Chunking the initial full-mesh winding accumulate into bounded 1.5M-element
submissions (commit `fix(gpu): chunk winding draws…`) was not sufficient — the
device is still lost, so the remaining suspects are per-chunk rasterization/fill
cost at 2× super-resolution (scissored 30k×12k target), the ~1.5 GB winding-bank
clear, VRAM pressure from the 30-copy vertex buffer + banks + pipelined slots,
or the downsample/RLE compute passes. Next steps: log VRAM at init, cap
per-chunk *fill* (split by bank/scissor rows, not just vertex range), submit the
winding clears separately, and try `wgpu::Features::…` device-lost callbacks to
get a precise reason. GPU slicing works on lighter scenes (e.g. 24-copy bed and
single models) on this hardware.
