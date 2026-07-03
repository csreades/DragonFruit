// GPU downsample (SSAA) + RLE of the winding buffer -> compact row-major runs.
//
// The winding buffer is rendered at N× (super) resolution. Passes:
//   0. downsample : one invocation per NATIVE pixel averages the aa×aa block of
//                   solid subpixels -> grayscale coverage (0..255) at native res.
//                   This is DragonFruit's "Coverage" SSAA, done on-GPU.
//   1. count_runs : per native row, count grayscale runs -> row_run_count[]
//   2. prefix_sum : exclusive scan over rows -> row_offset[] (+ total_runs)
//   3. write_runs : per row, write its runs at row_offset[row] as (len, value)
// Only the compact `runs` buffer is read back; winding + coverage stay in VRAM.

struct Params {
    native_w: u32,
    native_h: u32,
    super_w: u32,  // native_w * aa
    aa: u32,       // supersample factor per axis (1 = no AA)
    threshold: i32,
    runs_cap: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var<storage, read> winding: array<i32>;          // super res
@group(0) @binding(1) var<storage, read_write> coverage: array<u32>;    // native res, 0..255
@group(0) @binding(2) var<storage, read_write> row_run_count: array<u32>;
@group(0) @binding(3) var<storage, read_write> row_offset: array<u32>;
@group(0) @binding(4) var<storage, read_write> runs: array<u32>;        // [len,val] pairs
@group(0) @binding(5) var<storage, read_write> total_runs: atomic<u32>;
@group(0) @binding(6) var<uniform> P: Params;

fn subpixel_solid(sx: u32, sy: u32) -> u32 {
    let w = winding[sy * P.super_w + sx];
    return select(0u, 1u, abs(w) >= P.threshold);
}

fn gray_at(x: u32, y: u32) -> u32 {
    return coverage[y * P.native_w + x];
}

// ── Pass 0: aa×aa box downsample -> grayscale coverage ─────────────────────
// 2-D dispatch (8×8 workgroups) to stay under the 65535 workgroups-per-dim cap
// at 16K native resolution.
@compute @workgroup_size(8, 8)
fn downsample(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= P.native_w || y >= P.native_h) { return; }
    let idx = y * P.native_w + x;
    if (P.aa <= 1u) {
        // Fast path: no supersampling, winding is already native res.
        coverage[idx] = select(0u, 255u, abs(winding[idx]) >= P.threshold);
        return;
    }
    var solid: u32 = 0u;
    let bx = x * P.aa;
    let by = y * P.aa;
    for (var j: u32 = 0u; j < P.aa; j = j + 1u) {
        for (var i: u32 = 0u; i < P.aa; i = i + 1u) {
            solid = solid + subpixel_solid(bx + i, by + j);
        }
    }
    let samples = P.aa * P.aa;
    // Round(solid * 255 / samples)
    coverage[idx] = (solid * 255u + samples / 2u) / samples;
}

// ── Pass 1: count grayscale runs per native row ────────────────────────────
@compute @workgroup_size(64)
fn count_runs(@builtin(global_invocation_id) gid: vec3<u32>) {
    let row = gid.x;
    if (row >= P.native_h) { return; }
    var count: u32 = 1u;
    var prev: u32 = gray_at(0u, row);
    for (var x: u32 = 1u; x < P.native_w; x = x + 1u) {
        let v = gray_at(x, row);
        if (v != prev) { count = count + 1u; prev = v; }
    }
    row_run_count[row] = count;
}

// height is small; serial scan in one invocation is ample for v0.
// TODO(perf): Blelloch block-scan (goo_cpp) for very tall frames.
@compute @workgroup_size(1)
fn prefix_sum(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x != 0u) { return; }
    var acc: u32 = 0u;
    for (var r: u32 = 0u; r < P.native_h; r = r + 1u) {
        row_offset[r] = acc;
        acc = acc + row_run_count[r];
    }
    atomicStore(&total_runs, acc);
}

// ── Pass 3: write each row's grayscale runs at its offset ───────────────────
@compute @workgroup_size(64)
fn write_runs(@builtin(global_invocation_id) gid: vec3<u32>) {
    let row = gid.x;
    if (row >= P.native_h) { return; }
    var idx: u32 = row_offset[row];
    if (idx >= P.runs_cap) { return; }
    var prev: u32 = gray_at(0u, row);
    var len: u32 = 1u;
    for (var x: u32 = 1u; x < P.native_w; x = x + 1u) {
        let v = gray_at(x, row);
        if (v == prev) {
            len = len + 1u;
        } else {
            if (idx < P.runs_cap) {
                runs[idx * 2u] = len;
                runs[idx * 2u + 1u] = prev;
            }
            idx = idx + 1u;
            prev = v;
            len = 1u;
        }
    }
    if (idx < P.runs_cap) {
        runs[idx * 2u] = len;
        runs[idx * 2u + 1u] = prev;
    }
}
