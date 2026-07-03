// GPU slice generator — INCREMENTAL winding-number cross-section.
//
// winding(x, y) at plane z counts signed surface crossings of an upward ray:
// front-facing +1, back-facing -1; |winding| >= 1 => inside the solid.
//
// Instead of re-rendering the whole mesh per layer, the winding buffer is
// PERSISTENT and updated incrementally:
//
//   mode 0 (initial):  one full-mesh pass accumulating  +s  where
//                      mesh_z > z_hi           (all surfaces above the plane)
//   mode 1 (slab):     per layer, only the slab-candidate triangles are drawn,
//                      accumulating  -s  where  z_lo < mesh_z <= z_hi
//                      (surfaces the plane moved past stop counting)
//
// Both modes evaluate the SAME interpolated `mesh_z` varying against the same
// predicate family, so the update is exact set arithmetic on identical
// fragment sets: after subtracting slab (z_N, z_N+1] from "mesh_z > z_N" the
// buffer holds exactly "mesh_z > z_N+1". Integer atomics — no drift.
//
// No hardware near-plane clip is used (clip z is constant 0.5); layer
// selection happens entirely in the fragment predicate so the initial and
// slab passes rasterize identically.

struct Uniforms {
    ax: f32,      // NDC.x = ax*mesh.x + bx  (ax encodes mirror sign)
    bx: f32,
    ay: f32,      // NDC.y = ay*mesh.y + by
    by: f32,
    z_lo: f32,    // slab lower bound, exclusive (mode 1)
    z_hi: f32,    // plane (mode 0) / slab upper bound, inclusive (mode 1)
    width: u32,   // super-res raster width  (fragment guard)
    height: u32,  // super-res raster height (fragment guard)
    mode: u32,    // 0 = initial accumulate, 1 = slab subtract
    _p0: u32,
    _p1: u32,
    _p2: u32,
};

// The super-res winding buffer is split into row BANKS so it can exceed the
// device's max storage-binding size (6 GB at 16K/4×AA vs a ~4 GB cap). Each
// bank covers a contiguous range of rows; the render pass runs once per bank,
// scissored to it, and indexes the bank's buffer relative to its first row.
struct Bank {
    ny0: u32,    // first native row (inclusive)
    ny1: u32,    // last native row (inclusive)
    sy0: u32,    // first super-res row
    srows: u32,  // super-res rows in this bank
};

// Supersampling is done with subpixel-JITTERED passes at native resolution:
// hardware texture dimensions cap at 32768, so a 60480-wide 4× super target
// is impossible. Instead the geometry is offset by one subpixel per pass
// (aa² passes); native fragment (x, y) in pass (i, j) samples exactly the
// super-res pixel centre (x·aa+i+0.5, y·aa+j+0.5)/aa — the identical sample
// lattice to true super-res rasterization.
struct SubPass {
    dx_ndc: f32, // geometry offset for this subpixel, NDC units
    dy_ndc: f32,
    i: u32,      // subpixel column within the native pixel
    j: u32,      // subpixel row
    aa: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
};

@group(0) @binding(0) var<uniform> U: Uniforms;
@group(0) @binding(1) var<storage, read_write> winding: array<atomic<i32>>;
@group(0) @binding(2) var<uniform> BK: Bank;
@group(0) @binding(3) var<uniform> SP: SubPass;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) mesh_z: f32,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> VsOut {
    var out: VsOut;
    let cx = U.ax * pos.x + U.bx + SP.dx_ndc;
    let cy = U.ay * pos.y + U.by + SP.dy_ndc;
    out.pos = vec4<f32>(cx, cy, 0.5, 1.0);
    out.mesh_z = pos.z;
    return out;
}

@fragment
fn fs_main(
    in: VsOut,
    @builtin(front_facing) ff: bool,
) -> @location(0) vec4<f32> {
    // Native fragment coords; U.width/height are NATIVE dims.
    let x = u32(in.pos.x);
    let y = u32(in.pos.y);
    if (x < U.width && y >= BK.ny0 && y <= BK.ny1) {
        // Bank-relative super-res index of subpixel (x·aa+i, y·aa+j).
        let super_w = U.width * SP.aa;
        let idx = ((y - BK.ny0) * SP.aa + SP.j) * super_w + x * SP.aa + SP.i;
        // STL is conventionally CW-outward; front_face is set to Cw on the
        // pipeline so `ff` is the outward-facing surface.
        let s: i32 = select(-1, 1, ff);
        if (U.mode == 0u) {
            if (in.mesh_z > U.z_hi) {
                atomicAdd(&winding[idx], s);
            }
        } else {
            if (in.mesh_z > U.z_lo && in.mesh_z <= U.z_hi) {
                atomicAdd(&winding[idx], -s);
            }
        }
    }
    return vec4<f32>(0.0, 0.0, 0.0, 0.0); // dummy target drives rasterization
}
