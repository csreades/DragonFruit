//! wgpu GPU slice backend (v0).
//!
//! Ports goo_cpp's GPU slice generator to wgpu behind the [`SliceBackend`]
//! seam: for each layer it rasterizes the mesh cross-section into a per-pixel
//! winding buffer (fragment-stage atomic add), then a 3-pass compute RLE
//! (count → prefix-sum → write) compacts it into row-major `RleRun`s. Only the
//! compact runs are read back — the dense 16K winding stays in VRAM.
//!
//! STATUS: written without a GPU to test against (this build host has none).
//! It is structured to *compile* cleanly; the runtime kernels (winding sign,
//! Y/mirror orientation, overflow tiling) must be validated on real hardware.
//! v0 is binary (no AA). AA path = render multisampled / at N× then resolve;
//! see notes at the call sites. Fused GPU-RLE = the headroom win from the
//! analysis; this is that path minus AA.

use std::borrow::Cow;

use wgpu::util::DeviceExt;

use crate::backend::SliceBackend;
use crate::geometry::Triangle;
use crate::rle::RleRun;
use crate::types::{LayerAreaStatsV3, SliceJobV3};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    ax: f32,
    bx: f32,
    ay: f32,
    by: f32,
    /// Slab lower bound, exclusive (mode 1 only).
    z_lo: f32,
    /// Plane (mode 0) / slab upper bound, inclusive (mode 1).
    z_hi: f32,
    width: u32,
    height: u32,
    /// 0 = initial full-mesh accumulate, 1 = incremental slab subtract.
    mode: u32,
    /// 3DAA/vertical: 1 = per-subrow Z plane (Stage A). 0 = single centre plane.
    vaa: u32,
    /// Layer height (mm); used only when `vaa != 0`. z_lo/z_hi then carry the
    /// layer BASES ((L-1)·h, L·h) and the shader adds the per-subrow offset.
    layer_h: f32,
    _p2: u32,
}

/// One row-bank of the super-res winding buffer. Banks exist so total
/// winding memory (native·aa²·4 B — 6 GB at 16K/4×) can exceed the device's
/// max storage-binding size; each bank binds within the cap.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BankUniform {
    ny0: u32,   // first native row (inclusive)
    ny1: u32,   // last native row (inclusive)
    sy0: u32,   // first super-res row
    srows: u32, // super-res rows in this bank
}

/// One subpixel-jitter pass. Rasterizing at native resolution with the
/// geometry offset by one subpixel per pass reproduces the exact super-res
/// sample lattice without a >32768-px render target.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SubPassUniform {
    dx_ndc: f32,
    dy_ndc: f32,
    i: u32,
    j: u32,
    aa: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    native_w: u32,
    native_h: u32,
    super_w: u32,
    aa: u32,
    threshold: i32,
    runs_cap: u32,
    _pad0: u32,
    _pad1: u32,
}

/// Number of layers kept in flight on the GPU queue. While the CPU maps and
/// converts layer N's runs, layers N+1..N+DEPTH-1 are already submitted, so
/// the GPU never idles waiting for a readback round-trip.
const PIPELINE_DEPTH: usize = 3;

/// One in-flight layer's private resources. `runs_buf` is per-slot so a
/// too-small estimated readback can be topped up with a remainder copy later
/// without racing subsequent layers (which would overwrite a shared buffer).
struct LayerSlot {
    runs_buf: wgpu::Buffer,
    compute_bg: wgpu::BindGroup,
    total_readback: wgpu::Buffer,
    runs_readback: wgpu::Buffer,
    /// Slab triangle indices for this layer (incremental winding draw).
    index_buf: wgpu::Buffer,
    state: SlotState,
}

enum SlotState {
    Idle,
    /// Layer known-empty (slice plane outside the mesh Z range); no GPU work.
    Empty,
    /// Slab had no candidate triangles → winding (and therefore the runs) are
    /// identical to the previous layer; reuse them without touching the GPU.
    ReusePrev,
    InFlight {
        submission: wgpu::SubmissionIndex,
        /// How many runs were copied into `runs_readback` by the submission.
        copied_runs: u32,
    },
}

/// Union of two optional inclusive px bboxes ([x0, x1, y0, y1]).
fn bbox_union(a: Option<[u32; 4]>, b: Option<[u32; 4]>) -> Option<[u32; 4]> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => Some([
            a[0].min(b[0]),
            a[1].max(b[1]),
            a[2].min(b[2]),
            a[3].max(b[3]),
        ]),
    }
}

pub struct GpuSliceBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,

    total_layers: u32,
    width: u32,   // native
    height: u32,  // native
    super_w: u32, // native * aa
    super_h: u32,
    layer_height_mm: f32,
    z_min: f32,
    z_top: f32,
    ax: f32,
    bx: f32,
    ay: f32,
    by: f32,

    vertex_buf: wgpu::Buffer,
    vertex_count: u32,

    /// Persistent super-res winding, split into row banks (see BankUniform).
    winding_bufs: Vec<wgpu::Buffer>,
    banks: Vec<BankUniform>,
    target_view: wgpu::TextureView,

    render_pipeline: wgpu::RenderPipeline,
    /// One render bind group per bank (winding buffer + bank uniform differ).
    render_bgs: Vec<wgpu::BindGroup>,
    uniform_buf: wgpu::Buffer,

    downsample_pipeline: wgpu::ComputePipeline,
    /// One downsample bind group per bank.
    downsample_bgs: Vec<wgpu::BindGroup>,
    count_pipeline: wgpu::ComputePipeline,
    prefix_pipeline: wgpu::ComputePipeline,
    write_pipeline: wgpu::ComputePipeline,

    total_runs_buf: wgpu::Buffer,
    runs_cap: u32,

    /// Whole-mesh XY extent in native px (bbox for the initial full render).
    mesh_bbox: [u32; 4],
    /// Running per-layer bbox estimate: actual bbox of the latest collected
    /// layer ∪ slab bboxes of everything submitted since. Always a superset
    /// of the true solid extent (solid only changes where slabs touch).
    current_bbox: Option<[u32; 4]>,
    bbox_buf: wgpu::Buffer,

    /// Incremental winding state: has the initial full-mesh pass run, and
    /// which plane the persistent winding buffer currently represents.
    initialized: bool,
    last_plane_z: f32,
    /// 3DAA/vertical: submit_layer advances to layer BASES and the shader
    /// samples per-subrow Z planes (Stage A). false = single centre plane.
    vertical: bool,

    /// Slab CSR: per-layer candidate triangle ids (triangles whose Z range
    /// overlaps that layer's slab) + per-layer slab XY bbox in native px.
    slab_offsets: Vec<usize>,
    slab_indices: Vec<u32>,
    slab_bboxes: Vec<Option<[u32; 4]>>,
    /// Estimated rasterization fill (Σ triangle-bbox areas, native px²) for
    /// the whole mesh and per slab — bounds per-submission fragment work so
    /// fill-heavy content (stacked plate-scale triangles) can't blow the OS
    /// GPU watchdog no matter how few vertices it has.
    mesh_fill_px: f64,
    slab_fill_px: Vec<f64>,

    /// Most recently collected layer's runs (for ReusePrev layers).
    last_runs: Vec<RleRun>,

    slots: Vec<LayerSlot>,
    /// Next layer index to submit (submit-ahead cursor).
    next_submit: u32,
    /// Adaptive per-layer readback size estimate (in runs).
    est_runs: u32,
}

impl GpuSliceBackend {
    pub fn new(job: &SliceJobV3, triangles: &[Triangle]) -> Result<Self, String> {
        pollster::block_on(Self::new_async(job, triangles, None))
    }

    /// Construct with an explicit runs-buffer capacity (in runs) — used by the
    /// grow-and-retry path when the default capacity proved too small for the
    /// content's measured run count.
    pub fn new_with_runs_cap(
        job: &SliceJobV3,
        triangles: &[Triangle],
        runs_cap: u32,
    ) -> Result<Self, String> {
        pollster::block_on(Self::new_async(job, triangles, Some(runs_cap)))
    }

    async fn new_async(
        job: &SliceJobV3,
        triangles: &[Triangle],
        runs_cap_override: Option<u32>,
    ) -> Result<Self, String> {
        let width = job.effective_render_width_px();
        let height = job.source_height_px;
        if width == 0 || height == 0 {
            return Err("zero-size render target".into());
        }
        // Supersample factor from the AA level ("4x" -> 4). Winding is rendered
        // at super resolution and box-downsampled to native grayscale coverage.
        //
        // Coverage mode: XY SSAA at one Z plane (layer center).
        // Vertical2/3DAA: XY SSAA *plus* Z SSAA — each output layer integrates
        // coverage over `z_subsamples` planes spanning the layer thickness
        // (Stage A), then optionally blends across neighbour layers (Stage B).
        // Both use the same `aa` XY factor; 3DAA adds the Z-sweep in submit_layer.
        let is_vertical = job.anti_aliasing_mode_is_vertical();
        let aa = (job.effective_xy_aa_steps() as u32).max(1);
        // 3DAA/vertical Stage A: the `aa` Y-supersample sub-rows each sample a
        // distinct Z plane spanning the layer thickness (coupled to the jitter
        // pass index in the fragment shader), so XY level `aa` also gives `aa`
        // Z sub-samples per layer — no extra passes, no extra memory.
        if is_vertical {
            eprintln!("[gpu] 3DAA/vertical: XY {aa}x SSAA + {aa} Z sub-planes/layer (Stage A)");
        }
        let super_w = width.saturating_mul(aa);
        let super_h = height.saturating_mul(aa);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| "no wgpu adapter (GPU) available".to_string())?;
        let info = adapter.get_info();
        eprintln!(
            "[gpu] adapter: {} ({:?}, {:?}), driver {}",
            info.name, info.device_type, info.backend, info.driver_info
        );

        // Request the adapter's full limits — the winding buffer can be large
        // (width*height*4 bytes), exceeding the 128 MB default binding cap.
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("df-gpu-slicer"),
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
                },
                None,
            )
            .await
            .map_err(|e| format!("request_device failed: {e}"))?;

        // Surface the PRECISE failure reason on device loss / validation
        // errors instead of a bare "Parent device is lost" at the next submit.
        device.set_device_lost_callback(|reason, message| {
            eprintln!("[gpu] DEVICE LOST ({reason:?}): {message}");
        });
        device.on_uncaptured_error(Box::new(|e| {
            eprintln!("[gpu] UNCAPTURED ERROR: {e}");
        }));

        // The super-res winding memory (native·aa²·4 B) can exceed the max
        // storage-binding size — 6 GB at 16K/4× vs a ~4 GB cap. Split it into
        // row banks, each bindable within the cap; bank boundaries align to
        // native rows so the downsample never straddles banks.
        let limits = device.limits();
        let mut bind_cap =
            (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);
        // Test/debug override: force smaller banks (more of them) regardless of
        // device limits, so multi-bank behavior is reproducible everywhere.
        if let Some(mb) = std::env::var("DF_GPU_MAX_BANK_MB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            bind_cap = bind_cap.min(mb * 1_000_000);
        }
        let bytes_per_super_row = (super_w as u64) * 4;

        // Sanity cap on TOTAL winding memory: banking makes each allocation
        // individually legal, so an absurd aa request would otherwise
        // overcommit VRAM and wedge the device instead of failing cleanly.
        // Default 8 GiB (4xAA at 16K = 6 GB, validated); override with
        // DF_GPU_MAX_WINDING_GB.
        let winding_total_bytes = bytes_per_super_row * (super_h as u64);
        let max_winding_bytes = std::env::var("DF_GPU_MAX_WINDING_GB")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .map(|gb| (gb * 1e9) as u64)
            .unwrap_or(8_000_000_000);
        if winding_total_bytes > max_winding_bytes {
            return Err(format!(
                "winding memory {:.1} GB (aa={aa}, {super_w}x{super_h}) exceeds the \
                 {:.1} GB cap; lower the AA level (or raise DF_GPU_MAX_WINDING_GB \
                 if your GPU has the VRAM)",
                winding_total_bytes as f64 / 1e9,
                max_winding_bytes as f64 / 1e9,
            ));
        }
        let max_native_rows_per_bank =
            (((bind_cap / bytes_per_super_row) as u32) / aa).max(1);
        let num_banks = (height + max_native_rows_per_bank - 1) / max_native_rows_per_bank;
        let rows_per_bank = (height + num_banks - 1) / num_banks;
        if bytes_per_super_row * (aa as u64) > bind_cap {
            return Err(format!(
                "one native row of winding ({} B at aa={aa}) exceeds the storage binding \
                 cap {bind_cap} B; reduce resolution or AA",
                bytes_per_super_row * (aa as u64)
            ));
        }

        // ── Mesh vertex buffer (mesh-space mm; XY→NDC done in shader) ────────
        let mut verts: Vec<[f32; 3]> = Vec::with_capacity(triangles.len() * 3);
        let mut z_min = f32::INFINITY;
        let mut z_top = f32::NEG_INFINITY;
        let mut mesh_min_x = f32::INFINITY;
        let mut mesh_max_x = f32::NEG_INFINITY;
        let mut mesh_min_y = f32::INFINITY;
        let mut mesh_max_y = f32::NEG_INFINITY;
        for t in triangles {
            for v in [t.a, t.b, t.c] {
                verts.push([v.x, v.y, v.z]);
                z_min = z_min.min(v.z);
                z_top = z_top.max(v.z);
                mesh_min_x = mesh_min_x.min(v.x);
                mesh_max_x = mesh_max_x.max(v.x);
                mesh_min_y = mesh_min_y.min(v.y);
                mesh_max_y = mesh_max_y.max(v.y);
            }
        }
        if verts.is_empty() || !z_min.is_finite() {
            return Err("empty / non-finite mesh".into());
        }
        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-verts"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let vertex_count = verts.len() as u32;

        // NDC transform: mesh is plate-centered (origin = plate center), so
        // [-build/2, build/2] → [-1, 1]. Mirror flips the sign.
        let ax = if job.mirror_x { -2.0 } else { 2.0 } / job.build_width_mm;
        let bx = 0.0f32;
        let ay = if job.mirror_y { -2.0 } else { 2.0 } / job.build_depth_mm;
        let by = 0.0f32;

        // ── Buffers ─────────────────────────────────────────────────────────
        let mut banks: Vec<BankUniform> = Vec::new();
        let mut winding_bufs: Vec<wgpu::Buffer> = Vec::new();
        for b in 0..num_banks {
            let ny0 = b * rows_per_bank;
            if ny0 >= height {
                break;
            }
            let ny1 = ((b + 1) * rows_per_bank).min(height) - 1;
            let srows = (ny1 - ny0 + 1) * aa;
            banks.push(BankUniform {
                ny0,
                ny1,
                sy0: ny0 * aa,
                srows,
            });
            winding_bufs.push(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("winding-{b}")),
                size: (srows as u64) * bytes_per_super_row,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let bank_bufs: Vec<wgpu::Buffer> = banks
            .iter()
            .enumerate()
            .map(|(b, bank)| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("bank-{b}")),
                    contents: bytemuck::bytes_of(bank),
                    usage: wgpu::BufferUsages::UNIFORM,
                })
            })
            .collect();
        // Static subpixel-jitter uniforms: pass (i, j) samples super-res
        // centre (x·aa+i+0.5)/aa, i.e. geometry shifted by 0.5-(i+0.5)/aa
        // native px (Y sign flipped: +row = -NDC.y).
        let subpass_bufs: Vec<wgpu::Buffer> = (0..aa * aa)
            .map(|s| {
                let i = s % aa;
                let j = s / aa;
                let sp = SubPassUniform {
                    dx_ndc: (0.5 - (i as f32 + 0.5) / aa as f32) * 2.0 / width as f32,
                    dy_ndc: -(0.5 - (j as f32 + 0.5) / aa as f32) * 2.0 / height as f32,
                    i,
                    j,
                    aa,
                    _p0: 0,
                    _p1: 0,
                    _p2: 0,
                };
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("subpass-{s}")),
                    contents: bytemuck::bytes_of(&sp),
                    usage: wgpu::BufferUsages::UNIFORM,
                })
            })
            .collect();
        // Native-resolution grayscale coverage (0..255) produced by downsample.
        let coverage_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("coverage"),
            size: (width as u64) * (height as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Run capacity: budget ~ height * avg-runs/row. 8M run-slots (64 MB
        // buffer) comfortably covers real prints; overflow guarded in shader.
        let runs_cap: u32 = runs_cap_override
            .or_else(|| {
                std::env::var("DF_GPU_RUNS_CAP_M")
                    .ok()
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .map(|m| m.saturating_mul(1 << 20))
            })
            .unwrap_or(8 << 20)
            .max(1 << 20);
        let row_run_count_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("row_run_count"),
            size: (height as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let row_offset_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("row_offset"),
            size: (height as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let total_runs_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("total_runs"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params"),
            contents: bytemuck::bytes_of(&Params {
                native_w: width,
                native_h: height,
                super_w,
                aa,
                threshold: 1,
                runs_cap,
                _pad0: 0,
                _pad1: 0,
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // Static layer bbox in native pixels: the mesh's XY extent projected
        // through the same mm→NDC→pixel transform the rasterizer uses, padded
        // ±2 px and clamped. Rows/columns outside are background by
        // construction, so the RLE passes handle them in O(1).
        let ndc_of = |a: f32, v: f32| a * v; // b terms are 0
        let px_of_x = |ndc: f32| (ndc * 0.5 + 0.5) * width as f32;
        let px_of_y = |ndc: f32| (0.5 - ndc * 0.5) * height as f32; // NDC +y = row 0
        let (nx0, nx1) = {
            let a = ndc_of(ax, mesh_min_x);
            let b = ndc_of(ax, mesh_max_x);
            (a.min(b), a.max(b))
        };
        let (ny0, ny1) = {
            let a = ndc_of(ay, mesh_min_y);
            let b = ndc_of(ay, mesh_max_y);
            (a.min(b), a.max(b))
        };
        let clamp_px = |v: f32, hi: u32| (v as i64).clamp(0, hi as i64 - 1) as u32;
        let mesh_bbox = [
            clamp_px(px_of_x(nx0).floor() - 2.0, width),
            clamp_px(px_of_x(nx1).ceil() + 2.0, width),
            clamp_px(px_of_y(ny1).floor() - 2.0, height), // y flips: max NDC = min row
            clamp_px(px_of_y(ny0).ceil() + 2.0, height),
        ];
        let bbox_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bbox"),
            contents: bytemuck::cast_slice(&mesh_bbox),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Slab CSR for incremental winding ─────────────────────────────────
        // Layer L's slab is ((L-0.5)h, (L+0.5)h] in mesh Z (plane_{L-1} .. plane_L].
        // A triangle is a candidate for every slab its (padded) Z range
        // overlaps. Over-inclusion is harmless (the fragment predicate drops
        // it); exclusion would corrupt the winding, so pad by ±h/2.
        let total_layers = job.total_layers as usize;
        let h = job.layer_height_mm;
        let slab_range = |t: &Triangle| -> Option<(usize, usize)> {
            let tz0 = t.a.z.min(t.b.z).min(t.c.z) - 0.5 * h;
            let tz1 = t.a.z.max(t.b.z).max(t.c.z) + 0.5 * h;
            let lo = ((tz0 / h) - 0.5).floor() as i64;
            let hi = ((tz1 / h) + 0.5).ceil() as i64;
            let lo = lo.clamp(0, total_layers as i64 - 1) as usize;
            let hi = hi.clamp(0, total_layers as i64 - 1) as usize;
            if lo <= hi { Some((lo, hi)) } else { None }
        };
        let mut slab_counts = vec![0usize; total_layers];
        for t in triangles {
            if let Some((lo, hi)) = slab_range(t) {
                for c in &mut slab_counts[lo..=hi] {
                    *c += 1;
                }
            }
        }
        let mut slab_offsets = vec![0usize; total_layers + 1];
        for l in 0..total_layers {
            slab_offsets[l + 1] = slab_offsets[l] + slab_counts[l];
        }
        let max_slab_tris = slab_counts.iter().copied().max().unwrap_or(0);
        let mut slab_indices = vec![0u32; slab_offsets[total_layers]];
        let mut slab_bboxes: Vec<Option<[u32; 4]>> = vec![None; total_layers];
        let mut mesh_fill_px = 0.0f64;
        let mut slab_fill_px = vec![0.0f64; total_layers];
        let mut cursor = slab_offsets.clone();
        for (ti, t) in triangles.iter().enumerate() {
            let Some((lo, hi)) = slab_range(t) else { continue };
            // Triangle XY bbox in native px (through the same transform).
            let txa = ndc_of(ax, t.a.x.min(t.b.x).min(t.c.x));
            let txb = ndc_of(ax, t.a.x.max(t.b.x).max(t.c.x));
            let tya = ndc_of(ay, t.a.y.min(t.b.y).min(t.c.y));
            let tyb = ndc_of(ay, t.a.y.max(t.b.y).max(t.c.y));
            let tb = [
                clamp_px(px_of_x(txa.min(txb)).floor() - 2.0, width),
                clamp_px(px_of_x(txa.max(txb)).ceil() + 2.0, width),
                clamp_px(px_of_y(tya.max(tyb)).floor() - 2.0, height),
                clamp_px(px_of_y(tya.min(tyb)).ceil() + 2.0, height),
            ];
            let tri_fill =
                ((tb[1] - tb[0] + 1) as f64) * ((tb[3] - tb[2] + 1) as f64);
            mesh_fill_px += tri_fill;
            for l in lo..=hi {
                slab_indices[cursor[l]] = ti as u32;
                cursor[l] += 1;
                slab_bboxes[l] = bbox_union(slab_bboxes[l], Some(tb));
                slab_fill_px[l] += tri_fill;
            }
        }

        // Run-start positions scratch for write_runs: one slot per run plus
        // one leading slot per row.
        let pos_scratch_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pos-scratch"),
            size: ((runs_cap as u64) + (height as u64) + 8) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        // Per-(row, thread) boundary counts bridging count_runs → write_runs.
        let thread_counts_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("thread-counts"),
            size: (height as u64) * 256 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        // ── Dummy R8 color target (drives rasterization; contents unused) ────
        // NATIVE resolution: supersampling happens via jittered subpasses,
        // since texture dimensions cap at 32768 (< 60480 at 16K/4×).
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        // ── Shaders ──────────────────────────────────────────────────────────
        let slice_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("slice.wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("slice.wgsl"))),
        });
        let rle_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rle.wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("rle.wgsl"))),
        });
        let downsample_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("downsample.wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("downsample.wgsl"))),
        });

        // ── Render pipeline (winding accumulation) ───────────────────────────
        let render_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2, // bank uniform
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3, // subpass jitter uniform
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        // One bind group per (bank, subpass): index b·aa² + s.
        let mut render_bgs: Vec<wgpu::BindGroup> =
            Vec::with_capacity(winding_bufs.len() * subpass_bufs.len());
        for (wb, bb) in winding_bufs.iter().zip(bank_bufs.iter()) {
            for sp in &subpass_bufs {
                render_bgs.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("render-bg"),
                    layout: &render_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wb.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: bb.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: sp.as_entire_binding(),
                        },
                    ],
                }));
            }
        }
        let render_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render-layout"),
            bind_group_layouts: &[&render_bgl],
            push_constant_ranges: &[],
        });
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("winding-pipeline"),
            layout: Some(&render_layout),
            vertex: wgpu::VertexState {
                module: &slice_mod,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &slice_mod,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // ── Downsample pipeline (only pass reading banked winding) ───────────
        let downsample_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("downsample-bgl"),
            entries: &[
                storage_entry(0, true),  // winding (read, this bank)
                storage_entry(1, false), // coverage (rw, native full frame)
                uniform_entry(2),        // params
                uniform_entry(3),        // layer bbox
                uniform_entry(4),        // bank
            ],
        });
        let downsample_bgs: Vec<wgpu::BindGroup> = winding_bufs
            .iter()
            .zip(bank_bufs.iter())
            .map(|(wb, bb)| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("downsample-bg"),
                    layout: &downsample_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wb.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: coverage_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: params_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: bbox_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 4, resource: bb.as_entire_binding() },
                    ],
                })
            })
            .collect();
        let downsample_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("downsample-layout"),
            bind_group_layouts: &[&downsample_bgl],
            push_constant_ranges: &[],
        });
        let downsample_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("downsample"),
                layout: Some(&downsample_layout),
                module: &downsample_mod,
                entry_point: "downsample",
                compilation_options: Default::default(),
            });

        // ── RLE pipelines (count / prefix / write — never touch winding) ─────
        let compute_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rle-bgl"),
            entries: &[
                storage_entry(0, false), // coverage (rw, native)
                storage_entry(1, false), // row_run_count
                storage_entry(2, false), // row_offset
                storage_entry(3, false), // runs
                storage_entry(4, false), // total_runs
                uniform_entry(5),        // params
                uniform_entry(6),        // layer bbox
                storage_entry(7, false), // pos_scratch
                storage_entry(8, false), // thread_counts
            ],
        });
        // Per-slot resources: each in-flight layer gets its own runs buffer,
        // bind group, and readback pair, so pipelined layers never alias.
        let slots: Vec<LayerSlot> = (0..PIPELINE_DEPTH)
            .map(|i| {
                let runs_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("runs"),
                    size: (runs_cap as u64) * 2 * 4,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let compute_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("rle-bg"),
                    layout: &compute_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: coverage_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: row_run_count_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: row_offset_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: runs_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 4, resource: total_runs_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 5, resource: params_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 6, resource: bbox_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 7, resource: pos_scratch_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 8, resource: thread_counts_buf.as_entire_binding() },
                    ],
                });
                let total_readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("total-readback-{i}")),
                    size: 4,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let runs_readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("runs-readback-{i}")),
                    size: (runs_cap as u64) * 2 * 4,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let index_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("slab-indices-{i}")),
                    size: (max_slab_tris.max(1) as u64) * 3 * 4,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                LayerSlot {
                    runs_buf,
                    compute_bg,
                    total_readback,
                    runs_readback,
                    index_buf,
                    state: SlotState::Idle,
                }
            })
            .collect();
        let compute_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compute-layout"),
            bind_group_layouts: &[&compute_bgl],
            push_constant_ranges: &[],
        });
        let mk = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&compute_layout),
                module: &rle_mod,
                entry_point: entry,
                compilation_options: Default::default(),
            })
        };
        let count_pipeline = mk("count_runs");
        let prefix_pipeline = mk("prefix_sum");
        let write_pipeline = mk("write_runs");

        // VRAM budget tally — the first thing to check on a device loss.
        let winding_total: u64 = banks
            .iter()
            .map(|b| (b.srows as u64) * bytes_per_super_row)
            .sum();
        let vertex_bytes = (vertex_count as u64) * 12;
        let per_slot = (runs_cap as u64) * 2 * 4 + (max_slab_tris.max(1) as u64) * 3 * 4;
        let coverage_bytes = (width as u64) * (height as u64) * 4;
        let est_total = winding_total
            + vertex_bytes
            + coverage_bytes
            + per_slot * PIPELINE_DEPTH as u64
            + ((runs_cap as u64) + (height as u64) + 8) * 4
            + (height as u64) * 256 * 4;
        eprintln!(
            "[gpu] VRAM estimate: winding {:.0} MB x{} banks, vertices {:.0} MB, \
             coverage {:.0} MB, slots {:.0} MB x{} -> total ~{:.2} GB (aa={aa}, {}x{})",
            winding_total as f64 / 1e6 / banks.len().max(1) as f64,
            banks.len(),
            vertex_bytes as f64 / 1e6,
            coverage_bytes as f64 / 1e6,
            per_slot as f64 / 1e6,
            PIPELINE_DEPTH,
            est_total as f64 / 1e9,
            width,
            height,
        );

        Ok(Self {
            device,
            queue,
            total_layers: job.total_layers,
            width,
            height,
            super_w,
            super_h,
            layer_height_mm: job.layer_height_mm,
            z_min,
            z_top,
            ax,
            bx,
            ay,
            by,
            vertex_buf,
            vertex_count,
            winding_bufs,
            banks,
            target_view,
            render_pipeline,
            render_bgs,
            uniform_buf,
            downsample_pipeline,
            downsample_bgs,
            count_pipeline,
            prefix_pipeline,
            write_pipeline,
            total_runs_buf,
            runs_cap,
            mesh_bbox,
            current_bbox: None,
            bbox_buf,
            initialized: false,
            last_plane_z: 0.0,
            vertical: is_vertical,
            slab_offsets,
            slab_indices,
            slab_bboxes,
            mesh_fill_px,
            slab_fill_px,
            last_runs: Vec::new(),
            slots,
            next_submit: 0,
            est_runs: 65_536,
        })
    }

    /// All-background layer as row-major runs (one full-width bg run per row).
    fn empty_layer(&self) -> Vec<RleRun> {
        (0..self.height)
            .map(|_| RleRun {
                length: self.width,
                value: 0,
            })
            .collect()
    }

    /// Encode + submit one layer's GPU work and request its readback maps.
    /// Returns immediately; the wait happens in [`Self::collect_layer`].
    fn submit_layer(&mut self, layer_index: u32) {
        let slot_idx = (layer_index as usize) % PIPELINE_DEPTH;
        debug_assert!(matches!(self.slots[slot_idx].state, SlotState::Idle));

        let slice_z = (layer_index as f32 + 0.5) * self.layer_height_mm;
        if slice_z <= self.z_min || slice_z >= self.z_top {
            self.slots[slot_idx].state = SlotState::Empty;
            return;
        }
        // The Z the winding advances TO. 3DAA advances to the layer BASE (L·h);
        // the shader then offsets each subrow by (j+0.5)/aa·h so the aa subrows
        // straddle the whole layer. Non-3DAA advances to the centre plane.
        let advance_z = if self.vertical {
            layer_index as f32 * self.layer_height_mm
        } else {
            slice_z
        };

        let li = layer_index as usize;
        let (s0, s1) = (self.slab_offsets[li], self.slab_offsets[li + 1]);

        // Empty slab after init → winding (and runs) identical to the
        // previous layer; skip the GPU entirely. last_plane_z is NOT advanced:
        // the winding still represents the older plane, and the next real
        // slab's (z_lo, z_hi] range covers the skipped span (whose candidate
        // set is empty by construction).
        if self.initialized && s0 == s1 {
            self.slots[slot_idx].state = SlotState::ReusePrev;
            return;
        }

        // Per-layer bbox: superset of the solid extent. Initial layer uses
        // the whole-mesh bbox; afterwards actual-runs bbox ∪ pending slabs.
        let bbox_opt = if !self.initialized {
            Some(self.mesh_bbox)
        } else {
            bbox_union(self.current_bbox, self.slab_bboxes[li])
        };
        let Some(bbox) = bbox_opt else {
            // Nothing solid and nothing arriving: empty layer.
            self.slots[slot_idx].state = SlotState::Empty;
            return;
        };
        self.current_bbox = Some(bbox);
        self.queue
            .write_buffer(&self.bbox_buf, 0, bytemuck::cast_slice(&bbox));

        // Uniform + counter-reset writes are staged in submission order, so a
        // single shared uniform/counter buffer is safe with submit-ahead.
        let mode: u32 = if self.initialized { 1 } else { 0 };
        let u = Uniforms {
            ax: self.ax,
            bx: self.bx,
            ay: self.ay,
            by: self.by,
            z_lo: self.last_plane_z,
            z_hi: advance_z,
            width: self.width,
            height: self.height,
            mode,
            vaa: if self.vertical { 1 } else { 0 },
            layer_h: if self.vertical { self.layer_height_mm } else { 0.0 },
            _p2: 0,
        };
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&u));
        self.queue
            .write_buffer(&self.total_runs_buf, 0, &0u32.to_le_bytes());

        // Slab draw: upload this layer's candidate triangle indices.
        let index_count = if mode == 1 {
            let slab = &self.slab_indices[s0..s1];
            let mut idx: Vec<u32> = Vec::with_capacity(slab.len() * 3);
            for &t in slab {
                idx.push(t * 3);
                idx.push(t * 3 + 1);
                idx.push(t * 3 + 2);
            }
            self.queue
                .write_buffer(&self.slots[slot_idx].index_buf, 0, bytemuck::cast_slice(&idx));
            idx.len() as u32
        } else {
            0
        };

        let copied_runs = self.est_runs.clamp(1, self.runs_cap);
        let slot = &self.slots[slot_idx];

        let aa = self.super_w / self.width.max(1);
        let aa2 = (aa * aa) as usize;

        // Render: initial accumulate (full mesh) or slab subtract — one pass
        // per bank whose rows intersect the layer bbox, scissored to
        // (bbox ∩ bank) in NATIVE coordinates, with aa² jittered draws per
        // pass (one per subpixel; aa=1 → a single draw).
        //
        // Chunked over multiple submissions so a huge full-mesh accumulate (a
        // filled plate of high-poly parts) never exceeds the OS GPU watchdog
        // (TDR, ~2s) and loses the device. Winding banks accumulate additively,
        // so splitting the draw range is exact.
        let draw_total = if mode == 0 { self.vertex_count } else { index_count };
        // Per-SUBMISSION element budget: the chunk range is re-drawn once per
        // (intersecting bank × jitter subpass), so the divisor keeps a
        // submission's total rasterization work bounded regardless of AA/banks.
        // 48M tiny-triangle elements ≈ well under the ~2s OS watchdog on entry
        // GPUs; slab draws almost always fit one submission (stays pipelined).
        const MAX_ELEMS_PER_SUBMIT: u32 = 48_000_000;
        let active_banks = self
            .banks
            .iter()
            .filter(|bank| bbox[2].max(bank.ny0) <= bbox[3].min(bank.ny1))
            .count()
            .max(1) as u32;
        // CRITICAL: chunk boundaries MUST be triangle-aligned (multiple of 3)
        // — a misaligned `draw(start..end)` on a TriangleList reassembles
        // every triangle in the chunk from the wrong vertices.
        let chunk = ((MAX_ELEMS_PER_SUBMIT / (active_banks * aa2 as u32)) / 3 * 3).max(3);
        // The winding banks are persistent; clear them exactly once before the
        // initial full-mesh accumulate — in their OWN submission so the clear
        // never shares a watchdog window with rasterization work.
        if mode == 0 {
            let mut enc = self.device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor { label: Some("winding-clear") },
            );
            for wb in &self.winding_bufs {
                enc.clear_buffer(wb, 0, None);
            }
            let idx = self.queue.submit(Some(enc.finish()));
            self.device
                .poll(wgpu::Maintain::WaitForSubmissionIndex(idx));
        }
        // FILL budget per submission: element chunking alone cannot bound
        // rasterization work — a few thousand plate-covering triangles carry
        // hundreds of gigafragments in one tiny-vertex-count draw and blow
        // the OS watchdog (verified with a stacked-plates torture mesh).
        // Estimated fill = Σ triangle-bbox areas (from the CSR pass) × aa²
        // subpass draws; row-strips split a bank pass so each submission's
        // estimated fragments stay within budget.
        const FILL_BUDGET: f64 = 2.0e9;
        let fill_total = if mode == 0 {
            self.mesh_fill_px
        } else {
            self.slab_fill_px[li]
        } * (aa2 as f64);
        let bbox_rows = (bbox[3] - bbox[2] + 1) as f64;

        let mut start: u32 = 0;
        while start < draw_total {
            let end = (start + chunk).min(draw_total);
            let chunk_fill = fill_total * ((end - start) as f64 / draw_total.max(1) as f64);
            // (bank, row-strip) submission list for this chunk.
            let mut jobs: Vec<(usize, u32, u32)> = Vec::new();
            for (b, bank) in self.banks.iter().enumerate() {
                let ny0 = bbox[2].max(bank.ny0);
                let ny1 = bbox[3].min(bank.ny1);
                if ny0 > ny1 {
                    continue;
                }
                let rows = ny1 - ny0 + 1;
                let bank_fill = chunk_fill * (rows as f64 / bbox_rows.max(1.0));
                let strips = (bank_fill / FILL_BUDGET).ceil().max(1.0) as u32;
                let step = ((rows + strips - 1) / strips).max(1);
                let mut s0 = ny0;
                while s0 <= ny1 {
                    let s1 = (s0 + step - 1).min(ny1);
                    jobs.push((b, s0, s1));
                    s0 = s1 + 1;
                }
            }
            let n_jobs = jobs.len();
            let split = n_jobs > active_banks as usize || end < draw_total;
            for (ji, (b, sy0, sy1)) in jobs.into_iter().enumerate() {
                let mut enc = self.device.create_command_encoder(
                    &wgpu::CommandEncoderDescriptor { label: Some("slice-render") },
                );
                {
                    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("winding-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.target_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Discard,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    rp.set_pipeline(&self.render_pipeline);
                    rp.set_scissor_rect(bbox[0], sy0, bbox[1] - bbox[0] + 1, sy1 - sy0 + 1);
                    rp.set_vertex_buffer(0, self.vertex_buf.slice(..));
                    if mode == 1 {
                        rp.set_index_buffer(
                            slot.index_buf.slice(..(index_count as u64) * 4),
                            wgpu::IndexFormat::Uint32,
                        );
                    }
                    for s in 0..aa2 {
                        rp.set_bind_group(0, &self.render_bgs[b * aa2 + s], &[]);
                        if mode == 0 {
                            rp.draw(start..end, 0..1);
                        } else {
                            rp.draw_indexed(start..end, 0, 0..1);
                        }
                    }
                }
                let render_idx = self.queue.submit(Some(enc.finish()));
                // Keep the queue shallow only when actually splitting — the
                // common path (one chunk, one strip per bank) never polls
                // mid-layer and stays pipelined.
                if split && ji + 1 < n_jobs {
                    self.device
                        .poll(wgpu::Maintain::WaitForSubmissionIndex(render_idx));
                }
            }
            start = end;
        }

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("slice-rle") });

        // Compute passes: per-bank downsample (cropped to bbox ∩ bank), then
        // count → prefix → write over all rows (out-of-bbox rows exit in O(1)
        // with a single background run).
        {
            let bw = bbox[1] - bbox[0] + 1;
            let ds_x = (bw + 7) / 8;
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rle"),
                timestamp_writes: None,
            });
            cp.set_pipeline(&self.downsample_pipeline);
            for (b, bank) in self.banks.iter().enumerate() {
                let ny0 = bbox[2].max(bank.ny0);
                let ny1 = bbox[3].min(bank.ny1);
                if ny0 > ny1 {
                    continue;
                }
                let ds_y = ((ny1 - ny0 + 1) + 7) / 8;
                cp.set_bind_group(0, &self.downsample_bgs[b], &[]);
                cp.dispatch_workgroups(ds_x, ds_y, 1);
            }
            cp.set_bind_group(0, &slot.compute_bg, &[]);
            cp.set_pipeline(&self.count_pipeline);
            cp.dispatch_workgroups(self.height, 1, 1);
            cp.set_pipeline(&self.prefix_pipeline);
            cp.dispatch_workgroups(1, 1, 1);
            cp.set_pipeline(&self.write_pipeline);
            cp.dispatch_workgroups(self.height, 1, 1);
        }

        enc.copy_buffer_to_buffer(&self.total_runs_buf, 0, &slot.total_readback, 0, 4);
        // Copy only the estimated prefix of the runs buffer — not the 64 MB
        // cap. Underestimates are topped up in collect_layer via a remainder
        // copy from this slot's private runs_buf.
        enc.copy_buffer_to_buffer(
            &slot.runs_buf,
            0,
            &slot.runs_readback,
            0,
            (copied_runs as u64) * 2 * 4,
        );
        let submission = self.queue.submit(Some(enc.finish()));

        // Request the maps now; the callbacks fire once the submission
        // completes (guaranteed invoked by WaitForSubmissionIndex).
        slot.total_readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("total map failed"));
        slot.runs_readback
            .slice(..(copied_runs as u64) * 2 * 4)
            .map_async(wgpu::MapMode::Read, |r| r.expect("runs map failed"));

        self.slots[slot_idx].state = SlotState::InFlight {
            submission,
            copied_runs,
        };
        // The winding buffer now represents this layer's plane(s). For 3DAA
        // last_plane_z holds the layer BASE (L·h); the next real layer's slab
        // subtract advances each subrow from base+off to next_base+off.
        self.initialized = true;
        self.last_plane_z = advance_z;
    }

    /// Block until `layer_index`'s submission completes and convert its runs.
    fn collect_layer(&mut self, layer_index: u32) -> Result<Vec<RleRun>, String> {
        let slot_idx = (layer_index as usize) % PIPELINE_DEPTH;
        let state = std::mem::replace(&mut self.slots[slot_idx].state, SlotState::Idle);
        let (submission, copied_runs) = match state {
            SlotState::Empty => {
                self.last_runs = Vec::new();
                return Ok(self.empty_layer());
            }
            SlotState::ReusePrev => {
                if self.last_runs.is_empty() {
                    return Ok(self.empty_layer());
                }
                return Ok(self.last_runs.clone());
            }
            SlotState::InFlight {
                submission,
                copied_runs,
            } => (submission, copied_runs),
            SlotState::Idle => unreachable!("collect_layer on idle slot"),
        };

        // Wait for THIS submission only — later submitted layers keep running.
        self.device
            .poll(wgpu::Maintain::WaitForSubmissionIndex(submission));

        let slot = &self.slots[slot_idx];
        let total = {
            let data = slot.total_readback.slice(..).get_mapped_range();
            let v = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            drop(data);
            slot.total_readback.unmap();
            v
        };
        // Overflowing the runs buffer silently DROPS every row past the cap —
        // a print-corrupting failure (verified: a 2px-comb layer producing
        // 38M runs kept only the first ~1.2k of 6.2k rows). Return a flowing
        // error (NOT a panic: the desktop app builds panic=abort); the
        // machine-parseable `needed=` lets the fallback helper grow the run
        // buffers to the measured size and retry on-GPU.
        if total > self.runs_cap {
            slot.runs_readback.unmap();
            return Err(format!(
                "runs-cap-exceeded needed={total} cap={} — layer {layer_index} produced more \
                 RLE runs than the buffer holds; output would be truncated/corrupt",
                self.runs_cap,
            ));
        }

        if total == 0 {
            slot.runs_readback.unmap();
            self.last_runs = Vec::new();
            self.current_bbox = self.pending_slab_bbox(layer_index);
            return Ok(self.empty_layer());
        }

        // Rare: the estimate was too small. Top up from this slot's private
        // runs_buf (later layers write their own slots, so it's still intact).
        if total > copied_runs {
            slot.runs_readback.unmap();
            let mut enc = self.device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor { label: Some("runs-remainder") },
            );
            let from = (copied_runs as u64) * 2 * 4;
            let to = (total as u64) * 2 * 4;
            enc.copy_buffer_to_buffer(&slot.runs_buf, from, &slot.runs_readback, from, to - from);
            let idx = self.queue.submit(Some(enc.finish()));
            slot.runs_readback
                .slice(..to)
                .map_async(wgpu::MapMode::Read, |r| r.expect("remainder map failed"));
            self.device
                .poll(wgpu::Maintain::WaitForSubmissionIndex(idx));
        }

        let runs = {
            let data = slot
                .runs_readback
                .slice(..(total as u64) * 2 * 4)
                .get_mapped_range();
            let pairs: &[u32] = bytemuck::cast_slice(&data);
            let mut runs = Vec::with_capacity(total as usize);
            for i in 0..total as usize {
                runs.push(RleRun {
                    length: pairs[i * 2],
                    value: pairs[i * 2 + 1] as u8,
                });
            }
            runs
        };
        slot.runs_readback.unmap();

        // Adapt the estimate: 2× the latest layer's runs, floored generously.
        self.est_runs = (total.saturating_mul(2)).clamp(65_536, self.runs_cap);

        if runs.is_empty() {
            self.last_runs = Vec::new();
            self.current_bbox = self.pending_slab_bbox(layer_index);
            return Ok(self.empty_layer());
        }

        // Tighten the running bbox: this layer's ACTUAL solid extent ∪ slab
        // bboxes of everything already submitted past it. Any superset of the
        // true solid is correct; tighter = less RLE work.
        let actual = self.runs_bbox(&runs);
        self.current_bbox = bbox_union(actual, self.pending_slab_bbox(layer_index));
        self.last_runs = runs.clone();
        Ok(runs)
    }

    /// Union of the slab bboxes for layers submitted after `layer_index`
    /// (still in flight): solid can only appear where those slabs touch.
    fn pending_slab_bbox(&self, layer_index: u32) -> Option<[u32; 4]> {
        let mut acc = None;
        for l in (layer_index + 1)..self.next_submit {
            acc = bbox_union(acc, self.slab_bboxes[l as usize]);
        }
        acc
    }

    /// Inclusive native-px bbox of the non-background runs, padded ±2 px.
    fn runs_bbox(&self, runs: &[RleRun]) -> Option<[u32; 4]> {
        let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0u32, u32::MAX, 0u32);
        let mut x = 0u32;
        let mut row = 0u32;
        for r in runs {
            if r.value != 0 && r.length > 0 {
                x0 = x0.min(x);
                x1 = x1.max(x + r.length - 1);
                y0 = y0.min(row);
                y1 = y1.max(row);
            }
            x += r.length;
            if x >= self.width {
                x = 0;
                row += 1;
            }
        }
        if x0 == u32::MAX {
            return None;
        }
        Some([
            x0.saturating_sub(2),
            (x1 + 2).min(self.width - 1),
            y0.saturating_sub(2),
            (y1 + 2).min(self.height - 1),
        ])
    }
}

impl SliceBackend for GpuSliceBackend {
    fn total_layers(&self) -> u32 {
        self.total_layers
    }

    fn name(&self) -> &'static str {
        "gpu-wgpu"
    }

    fn slice_layer(
        &mut self,
        layer_index: u32,
        _compute_stats: bool,
    ) -> Result<(Vec<RleRun>, LayerAreaStatsV3), String> {
        let stats = LayerAreaStatsV3::default();

        // Keep the queue primed PIPELINE_DEPTH layers ahead: the driver calls
        // layers strictly in order, so slot (layer % DEPTH) is always Idle by
        // the time it is resubmitted.
        debug_assert!(layer_index < self.next_submit || layer_index == self.next_submit);
        while self.next_submit < self.total_layers
            && self.next_submit <= layer_index + (PIPELINE_DEPTH as u32 - 1)
        {
            let next = self.next_submit;
            self.submit_layer(next);
            self.next_submit += 1;
        }

        Ok((self.collect_layer(layer_index)?, stats))
    }
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
