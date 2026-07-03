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
    slice_z: f32,
    z_top: f32,
    width: u32,
    height: u32,
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

    winding_buf: wgpu::Buffer,
    target_view: wgpu::TextureView,

    render_pipeline: wgpu::RenderPipeline,
    render_bg: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,

    downsample_pipeline: wgpu::ComputePipeline,
    count_pipeline: wgpu::ComputePipeline,
    prefix_pipeline: wgpu::ComputePipeline,
    write_pipeline: wgpu::ComputePipeline,
    compute_bg: wgpu::BindGroup,

    runs_buf: wgpu::Buffer,
    total_runs_buf: wgpu::Buffer,
    runs_readback: wgpu::Buffer,
    total_readback: wgpu::Buffer,
    runs_cap: u32,
}

impl GpuSliceBackend {
    pub fn new(job: &SliceJobV3, triangles: &[Triangle]) -> Result<Self, String> {
        pollster::block_on(Self::new_async(job, triangles))
    }

    async fn new_async(job: &SliceJobV3, triangles: &[Triangle]) -> Result<Self, String> {
        let width = job.effective_render_width_px();
        let height = job.source_height_px;
        if width == 0 || height == 0 {
            return Err("zero-size render target".into());
        }
        // Supersample factor from the AA level ("4x" -> 4). Winding is rendered
        // at super resolution and box-downsampled to native grayscale coverage.
        let aa = (job.effective_xy_aa_steps() as u32).max(1);
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

        let winding_bytes = (super_w as u64) * (super_h as u64) * 4;
        let max_binding = device.limits().max_storage_buffer_binding_size as u64;
        if winding_bytes > max_binding {
            return Err(format!(
                "winding buffer {winding_bytes} B (aa={aa}, {super_w}x{super_h}) exceeds \
                 max_storage_buffer_binding_size {max_binding} B; lower --anti-aliasing, tile \
                 the frame, or reduce resolution (v0 does not tile yet)"
            ));
        }

        // ── Mesh vertex buffer (mesh-space mm; XY→NDC done in shader) ────────
        let mut verts: Vec<[f32; 3]> = Vec::with_capacity(triangles.len() * 3);
        let mut z_min = f32::INFINITY;
        let mut z_top = f32::NEG_INFINITY;
        for t in triangles {
            for v in [t.a, t.b, t.c] {
                verts.push([v.x, v.y, v.z]);
                z_min = z_min.min(v.z);
                z_top = z_top.max(v.z);
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
        let winding_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("winding"),
            size: winding_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
        let runs_cap: u32 = 8 << 20;
        let runs_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("runs"),
            size: (runs_cap as u64) * 2 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
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
        let runs_readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("runs-readback"),
            size: (runs_cap as u64) * 2 * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let total_readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("total-readback"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Dummy R8 color target (drives rasterization; contents unused) ────
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy-target"),
            size: wgpu::Extent3d {
                width: super_w,
                height: super_h,
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
            ],
        });
        let render_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render-bg"),
            layout: &render_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: winding_buf.as_entire_binding(),
                },
            ],
        });
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

        // ── Compute pipelines (count / prefix / write) ───────────────────────
        let compute_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compute-bgl"),
            entries: &[
                storage_entry(0, true),  // winding (read, super res)
                storage_entry(1, false), // coverage (rw, native)
                storage_entry(2, false), // row_run_count
                storage_entry(3, false), // row_offset
                storage_entry(4, false), // runs
                storage_entry(5, false), // total_runs
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let compute_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compute-bg"),
            layout: &compute_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: winding_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: coverage_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: row_run_count_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: row_offset_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: runs_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: total_runs_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: params_buf.as_entire_binding() },
            ],
        });
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
        let downsample_pipeline = mk("downsample");
        let count_pipeline = mk("count_runs");
        let prefix_pipeline = mk("prefix_sum");
        let write_pipeline = mk("write_runs");

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
            winding_buf,
            target_view,
            render_pipeline,
            render_bg,
            uniform_buf,
            downsample_pipeline,
            count_pipeline,
            prefix_pipeline,
            write_pipeline,
            compute_bg,
            runs_buf,
            total_runs_buf,
            runs_readback,
            total_readback,
            runs_cap,
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

    fn read_u32(buf: &wgpu::Buffer, device: &wgpu::Device) -> u32 {
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::Maintain::Wait);
        let data = slice.get_mapped_range();
        let v = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        drop(data);
        buf.unmap();
        v
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
    ) -> (Vec<RleRun>, LayerAreaStatsV3) {
        let stats = LayerAreaStatsV3::default();
        let slice_z = (layer_index as f32 + 0.5) * self.layer_height_mm;
        if slice_z <= self.z_min || slice_z >= self.z_top {
            return (self.empty_layer(), stats);
        }

        // Upload this layer's uniform.
        let u = Uniforms {
            ax: self.ax,
            bx: self.bx,
            ay: self.ay,
            by: self.by,
            slice_z,
            z_top: self.z_top,
            width: self.super_w,
            height: self.super_h,
        };
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&u));
        // Reset the total-runs counter.
        self.queue
            .write_buffer(&self.total_runs_buf, 0, &0u32.to_le_bytes());

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("slice") });

        // Clear winding to 0 for this layer.
        enc.clear_buffer(&self.winding_buf, 0, None);

        // Render pass: accumulate winding.
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
            rp.set_bind_group(0, &self.render_bg, &[]);
            rp.set_vertex_buffer(0, self.vertex_buf.slice(..));
            rp.draw(0..self.vertex_count, 0..1);
        }

        // Compute passes: count → prefix → write.
        {
            let row_groups = (self.height + 63) / 64;
            let ds_x = (self.width + 7) / 8;
            let ds_y = (self.height + 7) / 8;
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rle"),
                timestamp_writes: None,
            });
            cp.set_bind_group(0, &self.compute_bg, &[]);
            // AA downsample: super-res winding -> native grayscale coverage.
            cp.set_pipeline(&self.downsample_pipeline);
            cp.dispatch_workgroups(ds_x, ds_y, 1);
            cp.set_pipeline(&self.count_pipeline);
            cp.dispatch_workgroups(row_groups, 1, 1);
            cp.set_pipeline(&self.prefix_pipeline);
            cp.dispatch_workgroups(1, 1, 1);
            cp.set_pipeline(&self.write_pipeline);
            cp.dispatch_workgroups(row_groups, 1, 1);
        }

        enc.copy_buffer_to_buffer(&self.total_runs_buf, 0, &self.total_readback, 0, 4);
        enc.copy_buffer_to_buffer(
            &self.runs_buf,
            0,
            &self.runs_readback,
            0,
            (self.runs_cap as u64) * 2 * 4,
        );
        self.queue.submit(Some(enc.finish()));

        let total = Self::read_u32(&self.total_readback, &self.device).min(self.runs_cap);

        // Read back the compact runs.
        let slice = self.runs_readback.slice(..(total as u64) * 2 * 4);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let data = slice.get_mapped_range();
        let pairs: &[u32] = bytemuck::cast_slice(&data);
        let mut runs = Vec::with_capacity(total as usize);
        for i in 0..total as usize {
            runs.push(RleRun {
                length: pairs[i * 2],
                value: pairs[i * 2 + 1] as u8,
            });
        }
        drop(data);
        self.runs_readback.unmap();

        if runs.is_empty() {
            return (self.empty_layer(), stats);
        }
        (runs, stats)
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
