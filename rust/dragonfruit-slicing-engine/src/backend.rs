//! Pluggable slice-generation backends.
//!
//! DragonFruit's engine historically calls the CPU scanline rasterizer
//! (`raster::rasterize_layer_rle`) directly.  This module introduces a thin
//! seam so an alternative slice generator — notably a GPU backend — can produce
//! the same per-layer `Vec<RleRun>` and feed the *existing* streaming encoders
//! (goo/ctb/…) unchanged.
//!
//! The hot path is RLE-native (`crate::rle::RleRun`), so a GPU backend that
//! emits `(value, length)` runs plugs in with near-zero impedance: no dense
//! 16K mask ever crosses the PCIe bus.
//!
//! The default full-feature slice path (`engine::slice_with_progress_v3_to_path`,
//! with its 3DAA pump) is untouched; this seam is opt-in via the CLI
//! `--backend` flag and is the mount point for `gpu::GpuSliceBackend`.

use std::path::Path;
use std::time::Instant;

use crate::engine::SlicerV3Error;
use crate::geometry::Triangle;
use crate::index::{build_layer_index, LayerIndex, LayerSamplingSpan};
use crate::rle::RleRun;
use crate::types::{LayerAreaStatsV3, SliceJobV3};

/// A slice generator: given a prepared job + mesh, produce one layer's RLE
/// runs at a time, in display order.  Implementations own whatever state they
/// need (triangle index for CPU, device + uploaded mesh for GPU).
pub trait SliceBackend: Send {
    /// Number of layers this backend will produce (`job.total_layers`).
    fn total_layers(&self) -> u32;

    /// Rasterize one layer to RLE runs.  `layer_index` is 0-based; the slice
    /// plane is `(layer_index as f32 + 0.5) * layer_height_mm` in mesh Z — the
    /// same center-sampling convention used by `index`/`raster`.
    ///
    /// When `compute_stats` is false, implementations may return
    /// `LayerAreaStatsV3::default()` to skip connected-component analysis.
    ///
    /// Errors abort the slice (and, for the GPU backend, trigger the loud CPU
    /// fallback) — they must NOT be silently swallowed: a backend that cannot
    /// represent a layer faithfully (e.g. run-buffer overflow) must say so.
    fn slice_layer(
        &mut self,
        layer_index: u32,
        compute_stats: bool,
    ) -> Result<(Vec<RleRun>, LayerAreaStatsV3), String>;

    /// Human-readable backend name for logs/telemetry.
    fn name(&self) -> &'static str;
}

/// Per-run timing for the backend driver (nanoseconds).
#[derive(Debug, Default, Clone, Copy)]
pub struct BackendPerf {
    pub total_layers: u32,
    /// Wall time spent inside `slice_layer` across all layers.
    pub slice_ns: u128,
    /// Wall time spent feeding + finalizing the encoder.
    pub encode_ns: u128,
    pub total_ns: u128,
}

impl BackendPerf {
    pub fn layers_per_second(&self) -> f64 {
        if self.total_ns == 0 {
            0.0
        } else {
            self.total_layers as f64 / (self.total_ns as f64 / 1e9)
        }
    }
}

/// Drive a [`SliceBackend`] end-to-end: resolve the streaming encoder for
/// `job.output_format`, feed every layer's runs, and finalize to `output_path`.
///
/// This is the shared driver used by both the CPU seam backend and the GPU
/// backend, so the *only* thing that varies between them is `slice_layer`.
pub fn run_backend_to_path(
    job: &SliceJobV3,
    backend: &mut dyn SliceBackend,
    output_path: &Path,
) -> Result<BackendPerf, SlicerV3Error> {
    let encoder = crate::encoders::registry::find_encoder(&job.output_format).ok_or_else(|| {
        SlicerV3Error::UnsupportedOutput(format!(
            "no encoder registered for output format {:?}",
            job.output_format
        ))
    })?;

    let mut sink = encoder.create_rle_stream_encoder(job)?.ok_or_else(|| {
        SlicerV3Error::UnsupportedOutput(format!(
            "{} encoder has no streaming RLE sink (required by the backend driver)",
            job.output_format
        ))
    })?;

    let want_stats = encoder.requires_area_stats();
    let total = backend.total_layers();

    let t0 = Instant::now();
    let mut slice_ns: u128 = 0;
    let mut stats_vec: Vec<LayerAreaStatsV3> = if want_stats {
        Vec::with_capacity(total as usize)
    } else {
        Vec::new()
    };

    for layer in 0..total {
        let ts = Instant::now();
        let (runs, stats) = backend
            .slice_layer(layer, want_stats)
            .map_err(|e| SlicerV3Error::UnsupportedOutput(format!("slice backend: {e}")))?;
        slice_ns += ts.elapsed().as_nanos();
        sink.consume_rle_layer(layer, runs)?;
        if want_stats {
            stats_vec.push(stats);
        }
    }
    if want_stats {
        sink.set_area_stats(stats_vec);
    }

    let enc_start = Instant::now();
    sink.finalize_to_path(output_path)?;
    let encode_ns = enc_start.elapsed().as_nanos() + (t0.elapsed().as_nanos() - slice_ns);

    Ok(BackendPerf {
        total_layers: total,
        slice_ns,
        encode_ns,
        total_ns: t0.elapsed().as_nanos(),
    })
}

/// CPU slice backend: wraps the existing scanline rasterizer behind the seam.
///
/// Mirrors exactly what the streaming engine does per layer
/// (`build_layer_index` once, then `rasterize_layer_rle` per layer), so a
/// `--backend cpu-seam` run is directly comparable to the default path and
/// serves as the correctness oracle for the GPU backend.
pub struct CpuSliceBackend<'a> {
    job: &'a SliceJobV3,
    triangles: &'a [Triangle],
    index: LayerIndex,
}

impl<'a> CpuSliceBackend<'a> {
    pub fn new(job: &'a SliceJobV3, triangles: &'a [Triangle]) -> Self {
        // CenterOnly matches the non-Z-perturbed default sampling span used by
        // `engine::layer_index_sampling_span` for AA-off / blur / coverage.
        let index = build_layer_index(
            triangles,
            job.total_layers,
            job.layer_height_mm,
            LayerSamplingSpan::CenterOnly,
        );
        Self {
            job,
            triangles,
            index,
        }
    }
}

impl<'a> SliceBackend for CpuSliceBackend<'a> {
    fn total_layers(&self) -> u32 {
        self.job.total_layers
    }

    fn slice_layer(
        &mut self,
        layer_index: u32,
        compute_stats: bool,
    ) -> Result<(Vec<RleRun>, LayerAreaStatsV3), String> {
        let candidates = self.index.candidates_for_layer(layer_index);
        Ok(crate::raster::rasterize_layer_rle(
            self.job,
            self.triangles,
            candidates,
            layer_index,
            compute_stats,
        ))
    }

    fn name(&self) -> &'static str {
        "cpu-seam"
    }
}

/// Run the GPU backend with a LOUD automatic fallback to the full CPU engine
/// path on any GPU failure — init errors (VRAM caps, no adapter), mid-slice
/// backend errors (run-buffer overflow), or panics where unwinding is enabled.
/// The GPU's pathological content is the CPU's easy case (fill-bound and
/// run-dense meshes), so falling back is always safe, just slower.
///
/// Returns the perf plus `true` when the CPU fallback produced the output.
#[cfg(feature = "gpu")]
pub fn run_gpu_with_cpu_fallback(
    job: &SliceJobV3,
    triangles: &[Triangle],
    output_path: &Path,
) -> Result<(BackendPerf, bool), SlicerV3Error> {
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<BackendPerf, String> {
            let mut b = crate::gpu::GpuSliceBackend::new(job, triangles)?;
            run_backend_to_path(job, &mut b, output_path).map_err(|e| e.to_string())
        },
    ));
    let reason = match attempt {
        Ok(Ok(perf)) => return Ok((perf, false)),
        Ok(Err(e)) => e,
        Err(payload) => payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "GPU backend panicked (non-string payload)".to_string()),
    };

    eprintln!("[gpu] ==================================================================");
    eprintln!("[gpu] GPU SLICE FAILED — FALLING BACK TO THE CPU ENGINE PATH");
    eprintln!("[gpu] reason: {reason}");
    eprintln!("[gpu] (the output will be produced by the full CPU pipeline instead)");
    eprintln!("[gpu] ==================================================================");

    let t0 = Instant::now();
    let perf = crate::engine::slice_with_progress_v3_to_path(job, output_path, None, None)?;
    Ok((
        BackendPerf {
            total_layers: job.total_layers,
            slice_ns: perf.render_wall_ns as u128,
            encode_ns: (perf.png_encode_ns as u128).saturating_add(perf.archive_encode_ns as u128),
            total_ns: t0.elapsed().as_nanos(),
        },
        true,
    ))
}
