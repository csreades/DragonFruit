// GPU RLE of the winding buffer -> compact, row-major run list.
//
// Order-preserving with a tiny readback: three passes.
//   1. count_runs : one invocation per row counts its runs -> row_run_count[]
//   2. prefix_sum : exclusive scan over rows -> row_offset[] (+ total_runs)
//   3. write_runs : one invocation per row re-scans and writes its runs at
//                   row_offset[row]; each run is (length:u32, value:u32).
// Only the compacted `runs` buffer (2*total u32) is read back — the full-res
// winding never leaves VRAM. This is the fused slice->encode path.

struct Params {
    width: u32,
    height: u32,
    threshold: i32, // |winding| >= threshold => solid (typically 1)
    runs_cap: u32,  // capacity of `runs` in run-slots (guards overflow)
};

@group(0) @binding(0) var<storage, read> winding: array<i32>;
@group(0) @binding(1) var<storage, read_write> row_run_count: array<u32>;
@group(0) @binding(2) var<storage, read_write> row_offset: array<u32>;
@group(0) @binding(3) var<storage, read_write> runs: array<u32>; // [len,val] pairs
@group(0) @binding(4) var<storage, read_write> total_runs: atomic<u32>;
@group(0) @binding(5) var<uniform> P: Params;

fn solid_at(x: u32, y: u32) -> u32 {
    let w = winding[y * P.width + x];
    return select(0u, 255u, abs(w) >= P.threshold);
}

@compute @workgroup_size(64)
fn count_runs(@builtin(global_invocation_id) gid: vec3<u32>) {
    let row = gid.x;
    if (row >= P.height) { return; }
    var count: u32 = 1u;
    var prev: u32 = solid_at(0u, row);
    for (var x: u32 = 1u; x < P.width; x = x + 1u) {
        let v = solid_at(x, row);
        if (v != prev) { count = count + 1u; prev = v; }
    }
    row_run_count[row] = count;
}

// height (~6230) is small; a serial scan in one invocation is ample for v0.
// TODO(perf): replace with a Blelloch block-scan (goo_cpp) for very tall frames.
@compute @workgroup_size(1)
fn prefix_sum(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x != 0u) { return; }
    var acc: u32 = 0u;
    for (var r: u32 = 0u; r < P.height; r = r + 1u) {
        row_offset[r] = acc;
        acc = acc + row_run_count[r];
    }
    atomicStore(&total_runs, acc);
}

@compute @workgroup_size(64)
fn write_runs(@builtin(global_invocation_id) gid: vec3<u32>) {
    let row = gid.x;
    if (row >= P.height) { return; }
    var idx: u32 = row_offset[row];
    if (idx >= P.runs_cap) { return; } // overflow guard (host falls back to CPU)
    var prev: u32 = solid_at(0u, row);
    var len: u32 = 1u;
    for (var x: u32 = 1u; x < P.width; x = x + 1u) {
        let v = solid_at(x, row);
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
