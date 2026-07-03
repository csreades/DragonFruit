// GPU slice generator — winding-number cross-section (port of goo_cpp's
// stencil-based slicing, adapted to wgpu).
//
// To test whether pixel (x,y) is inside the solid at height slice_z, count
// signed surface crossings of a ray cast upward from slice_z: front-facing
// surface +1, back-facing -1. Non-zero winding => inside. We render only
// geometry ABOVE the plane (near-plane clip at slice_z) and accumulate the
// signed contribution per pixel with an atomic add into a storage buffer
// (portable across backends; avoids float-blend / stencil-readback issues).
// The winding buffer lives in VRAM and is consumed on-GPU by rle.wgsl — it is
// never read back, so no dense 16K mask crosses the bus.

struct Uniforms {
    ax: f32,      // NDC.x = ax*mesh.x + bx  (ax encodes mirror sign)
    bx: f32,
    ay: f32,      // NDC.y = ay*mesh.y + by
    by: f32,
    slice_z: f32, // mesh-Z of this layer's sampling plane (mm)
    z_top: f32,   // mesh max-Z (mm), normalizes clip-Z into (0,1]
    width: u32,
    height: u32,
};

@group(0) @binding(0) var<uniform> U: Uniforms;
@group(0) @binding(1) var<storage, read_write> winding: array<atomic<i32>>;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    let cx = U.ax * pos.x + U.bx;
    let cy = U.ay * pos.y + U.by;
    // Near-plane clip at slice_z: pos.z < slice_z -> clip.z < 0 -> clipped.
    let denom = max(U.z_top - U.slice_z, 1e-4);
    let cz = (pos.z - U.slice_z) / denom; // 0 at plane, ->1 at model top
    return vec4<f32>(cx, cy, cz, 1.0);
}

@fragment
fn fs_main(
    @builtin(position) frag: vec4<f32>,
    @builtin(front_facing) ff: bool,
) -> @location(0) vec4<f32> {
    let x = u32(frag.x);
    let y = u32(frag.y);
    if (x < U.width && y < U.height) {
        let idx = y * U.width + x;
        // STL is conventionally CW-outward; front_face is set to Cw on the
        // pipeline so `ff` is the outward-facing surface. Validate sign on HW.
        let s: i32 = select(-1, 1, ff);
        atomicAdd(&winding[idx], s);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 0.0); // dummy color target drives rasterization
}
