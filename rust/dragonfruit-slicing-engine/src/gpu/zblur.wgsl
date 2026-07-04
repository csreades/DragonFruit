// 3DAA Stage B: cross-layer Z-blur over the coverage ring.
//
// The blurred layer M is a weighted average of the same (x, y) across the
// window layers M-R..M+R (those that exist). Rather than binding all 2R+1
// ring planes at once (13 storage buffers at R=6 — over the per-stage limit,
// and a single ring buffer can exceed the max storage-binding size at 16K),
// the blur runs as one small accumulate dispatch per window plane into
// `cov` (the native coverage buffer, zeroed first), then a finalize dispatch
// normalizes in place:
//
//   accum:    cov[i] += w[|d|] * plane_{M+d}[i]        (one dispatch per d)
//   finalize: cov[i]  = min((cov[i] + wsum/2) / wsum, 255)
//
// wsum is the sum of the weights actually included (edge layers renormalize
// rather than darken), matching the CPU's apply_z_weighted_blur_to_rle_layer:
//   out = min((Σ w·cov + denom/2) / denom, 255)   in u32 arithmetic.
// Max accumulator value: 255 · 13 · 1024 ≈ 3.4M — far inside u32.
//
// Both entry points run over the window's union bbox; outside it `cov` is
// zeroed by the driver and the RLE passes treat it as background.

struct Meta {
    x0: u32,   // inclusive union bbox of the window planes
    x1: u32,
    y0: u32,
    y1: u32,
    width: u32, // native row stride
    wsum: u32,  // Σ of the weights included for this output layer
    _p0: u32,
    _p1: u32,
};

// Per-|d| kernel weight (static per job; one tiny uniform per distance).
struct Weight {
    w: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
};

@group(0) @binding(0) var<storage, read> plane: array<u32>;      // one ring plane
@group(0) @binding(1) var<storage, read_write> cov: array<u32>;  // accumulator / output
@group(0) @binding(2) var<uniform> M: Meta;
@group(0) @binding(3) var<uniform> W: Weight;

@compute @workgroup_size(8, 8)
fn zblur_accum(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = M.x0 + gid.x;
    let y = M.y0 + gid.y;
    if (x > M.x1 || y > M.y1) {
        return;
    }
    let i = y * M.width + x;
    cov[i] = cov[i] + W.w * plane[i];
}

@compute @workgroup_size(8, 8)
fn zblur_finalize(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = M.x0 + gid.x;
    let y = M.y0 + gid.y;
    if (x > M.x1 || y > M.y1) {
        return;
    }
    let i = y * M.width + x;
    cov[i] = min((cov[i] + M.wsum / 2u) / max(M.wsum, 1u), 255u);
}
