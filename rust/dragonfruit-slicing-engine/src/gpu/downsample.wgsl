// SSAA box-downsample: super-res winding (one row-bank) -> native grayscale
// coverage. Separated from rle.wgsl because it is the only compute pass that
// reads the winding buffer, which is split into row banks that can each be
// bound within the device's max storage-binding size. One dispatch per bank
// per layer, cropped to (layer bbox ∩ bank rows).

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

struct BBox {
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
};

struct Bank {
    ny0: u32,    // first native row (inclusive)
    ny1: u32,    // last native row (inclusive)
    sy0: u32,    // first super-res row
    srows: u32,  // super-res rows in this bank
};

@group(0) @binding(0) var<storage, read> winding: array<i32>; // this bank, super res
@group(0) @binding(1) var<storage, read_write> coverage: array<u32>; // native, full frame
@group(0) @binding(2) var<uniform> P: Params;
@group(0) @binding(3) var<uniform> B: BBox;
@group(0) @binding(4) var<uniform> BK: Bank;

fn subpixel_solid(sx: u32, sy: u32) -> u32 {
    // sy is a GLOBAL super-res row; the bank buffer starts at BK.sy0.
    let w = winding[(sy - BK.sy0) * P.super_w + sx];
    return select(0u, 1u, abs(w) >= P.threshold);
}

@compute @workgroup_size(8, 8)
fn downsample(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x + B.x0;
    let y = gid.y + max(B.y0, BK.ny0);
    if (x > B.x1 || y > min(B.y1, BK.ny1) || x >= P.native_w || y >= P.native_h) {
        return;
    }
    let idx = y * P.native_w + x;
    if (P.aa <= 1u) {
        // Fast path: no supersampling, winding rows are native rows.
        coverage[idx] = select(0u, 255u, abs(winding[(y - BK.sy0) * P.super_w + x]) >= P.threshold);
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
