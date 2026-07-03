// GPU downsample (SSAA) + parallel RLE of the winding buffer -> compact
// row-major runs.
//
// The winding buffer is rendered at N× (super) resolution. Passes:
//   0. downsample : one invocation per NATIVE pixel inside the layer bbox
//                   averages the aa×aa block of solid subpixels -> grayscale
//                   coverage (0..255) at native res (DragonFruit's "Coverage"
//                   SSAA, on-GPU).
//   1. count_runs : one WORKGROUP per row; threads boundary-detect
//                   (v[x] != v[x-1]) over chunked contiguous domains and
//                   tree-reduce -> row_run_count[]. Rows outside the bbox
//                   emit a single background run in O(1).
//   2. prefix_sum : one 256-thread workgroup; two-level exclusive scan over
//                   rows -> row_offset[] (+ total_runs).
//   3. write_runs : one workgroup per row; boundary-detect -> workgroup
//                   exclusive scan -> scatter boundary POSITIONS (x-ordered
//                   because per-thread domains are contiguous) -> emit
//                   (len, value) runs in parallel.
//
// The row is treated as virtually zero-padded outside the bbox columns
// [x0..x1], which reproduces the CPU emit_row behaviour (leading/trailing
// background runs) without reading coverage outside the bbox — coverage is
// stale there, since downsample is bbox-cropped.
//
// Only the compact `runs` buffer is read back; everything else stays in VRAM.

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

// Layer bounding box in native pixels, inclusive. Solid pixels are
// guaranteed to lie inside; outside is background by construction.
struct BBox {
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
};

@group(0) @binding(0) var<storage, read> winding: array<i32>;          // super res
@group(0) @binding(1) var<storage, read_write> coverage: array<u32>;    // native res, 0..255
@group(0) @binding(2) var<storage, read_write> row_run_count: array<u32>;
@group(0) @binding(3) var<storage, read_write> row_offset: array<u32>;
@group(0) @binding(4) var<storage, read_write> runs: array<u32>;        // [len,val] pairs
@group(0) @binding(5) var<storage, read_write> total_runs: atomic<u32>;
@group(0) @binding(6) var<uniform> P: Params;
@group(0) @binding(7) var<uniform> B: BBox;
@group(0) @binding(8) var<storage, read_write> pos_scratch: array<u32>; // run-start positions
// Per-(row, thread) boundary counts from count_runs, reused by write_runs so
// it never has to re-scan the row to recover scatter offsets.
@group(0) @binding(9) var<storage, read_write> thread_counts: array<u32>;

fn subpixel_solid(sx: u32, sy: u32) -> u32 {
    let w = winding[sy * P.super_w + sx];
    return select(0u, 1u, abs(w) >= P.threshold);
}

// Value of the virtually zero-padded row: 0 outside the bbox columns, so
// stale coverage outside the bbox is never read.
fn vrow(row: u32, x: u32) -> u32 {
    if (x < B.x0 || x > B.x1) {
        return 0u;
    }
    return coverage[row * P.native_w + x];
}

// ── Pass 0: aa×aa box downsample -> grayscale coverage (bbox-cropped) ──────
@compute @workgroup_size(8, 8)
fn downsample(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x + B.x0;
    let y = gid.y + B.y0;
    if (x > B.x1 || y > B.y1 || x >= P.native_w || y >= P.native_h) { return; }
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

var<workgroup> wg_cnt: array<u32, 256>;

// Boundary domain for a row: candidate boundary positions x (where
// vrow(x) != vrow(x-1)) all lie in [max(x0,1) .. min(x1+1, W-1)] because the
// virtual row is constant 0 outside [x0..x1].

// ── Pass 1: count runs per row (workgroup per row, coalesced) ──────────────
@compute @workgroup_size(256)
fn count_runs(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let row = wg.x;
    if (row >= P.native_h) { return; }
    if (row < B.y0 || row > B.y1) {
        if (lid == 0u) { row_run_count[row] = 1u; }
        return;
    }
    let xs = max(B.x0, 1u);
    let xe = min(B.x1 + 1u, P.native_w - 1u);
    var cnt: u32 = 0u;
    if (xs <= xe) {
        let len = xe - xs + 1u;
        let chunk = (len + 255u) / 256u;
        let a = xs + lid * chunk;
        let b = min(a + chunk, xe + 1u);
        // Rolling prev: one coverage read per pixel.
        var prev: u32 = vrow(row, a - 1u);
        var x = a;
        loop {
            if (x >= b) { break; }
            let v = vrow(row, x);
            if (v != prev) { cnt = cnt + 1u; }
            prev = v;
            x = x + 1u;
        }
    }
    thread_counts[row * 256u + lid] = cnt;
    wg_cnt[lid] = cnt;
    workgroupBarrier();
    var s: u32 = 128u;
    loop {
        if (s == 0u) { break; }
        if (lid < s) { wg_cnt[lid] = wg_cnt[lid] + wg_cnt[lid + s]; }
        workgroupBarrier();
        s = s / 2u;
    }
    if (lid == 0u) { row_run_count[row] = wg_cnt[0] + 1u; }
}

var<workgroup> wg_scan: array<u32, 256>;

// ── Pass 2: exclusive scan over rows (single 256-thread workgroup) ─────────
@compute @workgroup_size(256)
fn prefix_sum(@builtin(local_invocation_index) lid: u32) {
    let h = P.native_h;
    let chunk = (h + 255u) / 256u;
    let a = min(lid * chunk, h);
    let b = min(a + chunk, h);
    var s: u32 = 0u;
    for (var r: u32 = a; r < b; r = r + 1u) {
        s = s + row_run_count[r];
    }
    wg_scan[lid] = s;
    workgroupBarrier();
    // Hillis–Steele inclusive scan (read phase / barrier / write phase).
    var off: u32 = 1u;
    loop {
        if (off >= 256u) { break; }
        var v: u32 = wg_scan[lid];
        if (lid >= off) { v = v + wg_scan[lid - off]; }
        workgroupBarrier();
        wg_scan[lid] = v;
        workgroupBarrier();
        off = off << 1u;
    }
    let inclusive = wg_scan[lid];
    var acc: u32 = inclusive - s; // exclusive prefix for this thread's chunk
    for (var r: u32 = a; r < b; r = r + 1u) {
        row_offset[r] = acc;
        acc = acc + row_run_count[r];
    }
    if (lid == 255u) {
        atomicStore(&total_runs, inclusive);
    }
}

// ── Pass 3: scatter boundary positions, then emit runs (workgroup per row) ─
@compute @workgroup_size(256)
fn write_runs(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let row = wg.x;
    if (row >= P.native_h) { return; }
    let roff = row_offset[row];
    if (row < B.y0 || row > B.y1) {
        if (lid == 0u && roff < P.runs_cap) {
            runs[roff * 2u] = P.native_w;
            runs[roff * 2u + 1u] = 0u;
        }
        return;
    }

    // Positions region for this row: one extra slot per row for the leading 0.
    let poff = roff + row;
    let pos_len = arrayLength(&pos_scratch);
    let xs = max(B.x0, 1u);
    let xe = min(B.x1 + 1u, P.native_w - 1u);

    // Phase A: reuse count_runs' per-thread boundary counts (no re-scan).
    var a: u32 = 0u;
    var b: u32 = 0u;
    if (xs <= xe) {
        let len = xe - xs + 1u;
        let chunk = (len + 255u) / 256u;
        a = xs + lid * chunk;
        b = min(a + chunk, xe + 1u);
    }
    let cnt = thread_counts[row * 256u + lid];
    wg_cnt[lid] = cnt;
    workgroupBarrier();
    // Exclusive scan of the 256 per-thread counts (Hillis–Steele inclusive).
    var off: u32 = 1u;
    loop {
        if (off >= 256u) { break; }
        var v: u32 = wg_cnt[lid];
        if (lid >= off) { v = v + wg_cnt[lid - off]; }
        workgroupBarrier();
        wg_cnt[lid] = v;
        workgroupBarrier();
        off = off << 1u;
    }
    let inclusive = wg_cnt[lid];
    let base = inclusive - cnt;
    workgroupBarrier();
    if (lid == 255u) { wg_cnt[0] = inclusive; } // stash grand total
    workgroupBarrier();
    let boundaries = wg_cnt[0];

    // Phase B: scatter boundary positions (x-ordered: contiguous domains).
    if (lid == 0u && poff < pos_len) {
        pos_scratch[poff] = 0u; // first run starts at virtual x = 0
    }
    var k: u32 = base + 1u;
    if (xs <= xe && cnt > 0u) {
        var prev: u32 = vrow(row, a - 1u);
        var x = a;
        loop {
            if (x >= b) { break; }
            let v = vrow(row, x);
            if (v != prev) {
                if (poff + k < pos_len) { pos_scratch[poff + k] = x; }
                k = k + 1u;
            }
            prev = v;
            x = x + 1u;
        }
    }
    storageBarrier();
    workgroupBarrier();

    // Phase C: emit (len, value) runs in parallel.
    let runs_in_row = boundaries + 1u;
    var i: u32 = lid;
    loop {
        if (i >= runs_in_row) { break; }
        var start: u32 = 0u;
        if (poff + i < pos_len) { start = pos_scratch[poff + i]; }
        var end: u32 = P.native_w;
        if (i + 1u < runs_in_row && poff + i + 1u < pos_len) {
            end = pos_scratch[poff + i + 1u];
        }
        let slot = roff + i;
        if (slot < P.runs_cap) {
            runs[slot * 2u] = end - start;
            runs[slot * 2u + 1u] = vrow(row, start);
        }
        i = i + 256u;
    }
}
