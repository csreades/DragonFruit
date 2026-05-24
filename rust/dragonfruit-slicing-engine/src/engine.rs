//! V3 engine orchestration and validation layer.

use crate::encoders::registry::{
    find_encoder, find_encoder_by_hint_or_source, supported_output_formats,
};
use crate::geometry::{parse_triangles, project_triangles_inplace};
use crate::index::build_layer_index;
use crate::metrics::SlicingPerfV3;
use crate::pipeline::{render_layers_bounded, render_layers_rle, render_layers_rle_encoded};
use crate::types::{
    LayerAreaStatsV3, ProgressCallbackV3, RenderedLayersV3, SliceArtifactV3, SliceJobV3,
    SliceProgressPhaseV3, SliceProgressUpdateV3,
};
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SlicerV3Error {
    #[error("cancelled")]
    Cancelled,
    #[error("unsupported output format: {0}")]
    UnsupportedOutput(String),
    #[error("invalid dimensions {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error(
        "invalid layer settings: layer_height_mm={layer_height_mm}, total_layers={total_layers}"
    )]
    InvalidLayerSettings {
        layer_height_mm: f32,
        total_layers: u32,
    },
    #[error("invalid build volume dimensions: build_width_mm={build_width_mm}, build_depth_mm={build_depth_mm}")]
    InvalidBuildVolume {
        build_width_mm: f32,
        build_depth_mm: f32,
    },
    #[error("invalid triangle buffer length: expected multiple of 9, got {0}")]
    InvalidTriangleBuffer(usize),
    #[error("png encode failed: {0}")]
    Png(String),
    #[error("zip encode failed: {0}")]
    Zip(String),
    #[error("json encode failed: {0}")]
    Json(String),
    #[error("missing rendered layer payload: {0}")]
    MissingRenderedLayerPayload(String),
    #[error("layer preview read failed: {0}")]
    LayerPreview(String),
}

/// RAII wrapper that ensures the background encode thread is always joined before the
/// enclosing function returns, even on early-exit `?` paths.
pub struct EncodeThreadHandle<T> {
    pub handle: Option<std::thread::JoinHandle<Result<T, SlicerV3Error>>>,
}

impl<T> EncodeThreadHandle<T> {
    /// Consume the handle on the success path: joins the thread and returns the result.
    /// Must only be called after the sender has been dropped (otherwise the thread will never exit).
    pub fn finish(mut self) -> Result<T, SlicerV3Error> {
        self.handle
            .take()
            .expect("EncodeThreadHandle::finish called twice")
            .join()
            .map_err(|_| SlicerV3Error::LayerPreview("Encode thread panicked".to_string()))?
    }
}

impl<T> Drop for EncodeThreadHandle<T> {
    fn drop(&mut self) {
        // On error paths finish() was not called; join the thread here so
        // that we don't leave zombie threads or have data races.
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn validate_job(job: &SliceJobV3) -> Result<(), SlicerV3Error> {
    if job.width_px == 0
        || job.height_px == 0
        || job.source_width_px == 0
        || job.source_height_px == 0
    {
        return Err(SlicerV3Error::InvalidDimensions {
            width: job.width_px,
            height: job.height_px,
        });
    }
    if !(job.layer_height_mm.is_finite() && job.layer_height_mm > 0.0) || job.total_layers == 0 {
        return Err(SlicerV3Error::InvalidLayerSettings {
            layer_height_mm: job.layer_height_mm,
            total_layers: job.total_layers,
        });
    }
    if !(job.build_width_mm.is_finite() && job.build_width_mm > 0.0)
        || !(job.build_depth_mm.is_finite() && job.build_depth_mm > 0.0)
    {
        return Err(SlicerV3Error::InvalidBuildVolume {
            build_width_mm: job.build_width_mm,
            build_depth_mm: job.build_depth_mm,
        });
    }
    if job.triangles_xyz.len() % 9 != 0 {
        return Err(SlicerV3Error::InvalidTriangleBuffer(
            job.triangles_xyz.len(),
        ));
    }
    Ok(())
}

/// Clean-room V3 entry point with full pipeline:
/// parse triangles -> build layer index -> bounded parallel render -> zip archive encode.
pub fn slice_with_progress_v3(
    job: &SliceJobV3,
    on_progress: Option<ProgressCallbackV3>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<SliceArtifactV3, SlicerV3Error> {
    let mut job_padded = job.clone();
    if job.enable_z_perturbation && job.blur_mode_z != "None" && job.blur_radius_z > 0 {
        job_padded.total_layers = job.total_layers + job.blur_radius_z;
    }
    let job = &job_padded;

    let Some(encoder) = find_encoder(&job.output_format) else {
        return Err(SlicerV3Error::UnsupportedOutput(format!(
            "{} (supported: {})",
            job.output_format,
            supported_output_formats().join(", ")
        )));
    };
    let requires_area_stats = encoder.requires_area_stats();
    let requires_png_layers = encoder.requires_png_layers();
    let requires_raw_mask_layers = encoder.requires_raw_mask_layers();
    let aa_enabled = job.anti_aliasing_level.trim() != "Off";
    let has_blurs = aa_enabled && (job.blur_mode_xy != "None" || (job.enable_z_perturbation && job.blur_mode_z != "None"));

    // RLE path: no full-image pixel buffer — fastest for formats like CTBv5.
    if !requires_png_layers && !has_blurs {
        if let Some(mut rle_enc) = encoder.create_rle_stream_encoder(job)? {
            let total_start = std::time::Instant::now();
            let job_total_layers = job.total_layers;
            let progress_total = job_total_layers.saturating_add(1);

            let slicing_progress = on_progress.as_ref().map(|cb| {
                let cb = cb.clone();
                Arc::new(move |update: SliceProgressUpdateV3| {
                    cb(SliceProgressUpdateV3 {
                        done: update.done.min(job_total_layers),
                        total: progress_total,
                        phase: SliceProgressPhaseV3::Slicing,
                    });
                }) as ProgressCallbackV3
            });

            // Parallel-encode path: rasterize + encode PNG in rayon workers.
            let (_rendered_layers, layer_area_stats, mut perf) = if let Some(encode_fn) =
                rle_enc.parallel_encode_fn()
            {
                let mut store_sink = |idx: u32, bytes: Vec<u8>| -> Result<(), SlicerV3Error> {
                    rle_enc.store_encoded_layer(idx, bytes);
                    Ok(())
                };
                slice_and_rasterize_rle_encoded_v3(
                    job,
                    requires_area_stats,
                    encode_fn,
                    &mut store_sink,
                    slicing_progress,
                    cancel_flag,
                )?
            } else {
                let mut rle_sink =
                    |idx: u32, runs: Vec<crate::rle::RleRun>| rle_enc.consume_rle_layer(idx, runs);
                slice_and_rasterize_rle_v3(
                    job,
                    requires_area_stats,
                    &mut rle_sink,
                    slicing_progress,
                    cancel_flag,
                )?
            };

            rle_enc.set_area_stats(layer_area_stats);

            if let Some(cb) = on_progress.as_ref() {
                cb(SliceProgressUpdateV3 {
                    done: job_total_layers,
                    total: progress_total,
                    phase: SliceProgressPhaseV3::Finalizing,
                });
            }

            let encode_start = std::time::Instant::now();
            let bytes = rle_enc.finalize_to_bytes()?;

            if let Some(cb) = on_progress.as_ref() {
                cb(SliceProgressUpdateV3 {
                    done: progress_total,
                    total: progress_total,
                    phase: SliceProgressPhaseV3::Finalizing,
                });
            }

            perf.archive_encode_ns = encode_start.elapsed().as_nanos() as u64;
            perf.total_ns = total_start.elapsed().as_nanos() as u64;
            perf.layers = job.total_layers;

            return Ok(SliceArtifactV3 { bytes, perf });
        }
    }

    if !requires_png_layers && requires_raw_mask_layers && !has_blurs {
        if let Some(mut stream_encoder) = encoder.create_raw_mask_stream_encoder(job)? {
            let total_start = std::time::Instant::now();
            let job_total_layers = job.total_layers;
            let progress_total = job_total_layers.saturating_add(1);
            let mut raw_mask_sink = |layer_index: u32, raw_mask: Vec<u8>| {
                stream_encoder.consume_raw_mask_layer(layer_index, raw_mask)
            };

            let slicing_progress = on_progress.as_ref().map(|cb| {
                let cb = cb.clone();
                Arc::new(move |update: SliceProgressUpdateV3| {
                    cb(SliceProgressUpdateV3 {
                        done: update.done.min(job_total_layers),
                        total: progress_total,
                        phase: SliceProgressPhaseV3::Slicing,
                    });
                }) as ProgressCallbackV3
            });

            let (_rendered_layers, _layer_area_stats, mut perf) = slice_and_rasterize_v3(
                job,
                false,
                requires_png_layers,
                false,
                Some(&mut raw_mask_sink),
                slicing_progress,
                cancel_flag,
            )?;

            if let Some(cb) = on_progress.as_ref() {
                cb(SliceProgressUpdateV3 {
                    done: job_total_layers,
                    total: progress_total,
                    phase: SliceProgressPhaseV3::Finalizing,
                });
            }

            let encode_start = std::time::Instant::now();
            let bytes = stream_encoder.finalize_to_bytes()?;

            if let Some(cb) = on_progress.as_ref() {
                cb(SliceProgressUpdateV3 {
                    done: progress_total,
                    total: progress_total,
                    phase: SliceProgressPhaseV3::Finalizing,
                });
            }

            perf.archive_encode_ns = encode_start.elapsed().as_nanos() as u64;
            perf.total_ns = total_start.elapsed().as_nanos() as u64;
            perf.layers = job.total_layers;

            return Ok(SliceArtifactV3 { bytes, perf });
        }
    }

    let total_start = std::time::Instant::now();

    let (rendered_layers, layer_area_stats, mut perf) = slice_and_rasterize_v3(
        job,
        requires_area_stats,
        requires_png_layers,
        requires_raw_mask_layers,
        None,
        on_progress.clone(),
        cancel_flag,
    )?;

    let encode_units = encoder
        .estimate_encode_progress_units(&rendered_layers)
        .max(1);
    let progress_total = job.total_layers.saturating_add(encode_units);

    let encode_progress = on_progress.as_ref().map(|cb| {
        let cb = cb.clone();
        move |done: u32, total: u32| {
            let safe_total = total.max(1);
            let clamped_done = done.min(safe_total);
            let normalized =
                ((clamped_done as u64) * (encode_units as u64) / (safe_total as u64)) as u32;
            cb(SliceProgressUpdateV3 {
                done: job.total_layers.saturating_add(normalized),
                total: progress_total,
                phase: SliceProgressPhaseV3::Encoding,
            });
        }
    });

    let encode_start = std::time::Instant::now();
    let bytes = dispatch_encode_by_format(
        job,
        &rendered_layers,
        &layer_area_stats,
        encode_progress.as_ref().map(|cb| cb as &dyn Fn(u32, u32)),
    )?;

    if let Some(cb) = on_progress.as_ref() {
        cb(SliceProgressUpdateV3 {
            done: progress_total,
            total: progress_total,
            phase: SliceProgressPhaseV3::Finalizing,
        });
    }

    perf.archive_encode_ns = encode_start.elapsed().as_nanos() as u64;
    perf.total_ns = total_start.elapsed().as_nanos() as u64;
    perf.layers = job.total_layers;

    Ok(SliceArtifactV3 { bytes, perf })
}

/// Fast raster stage that produces RLE runs per layer — no pixel buffers.
pub fn slice_and_rasterize_rle_v3(
    job: &SliceJobV3,
    compute_area_stats: bool,
    on_rle_layer: impl FnMut(u32, Vec<crate::rle::RleRun>) -> Result<(), SlicerV3Error>,
    on_progress: Option<ProgressCallbackV3>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(RenderedLayersV3, Vec<LayerAreaStatsV3>, SlicingPerfV3), SlicerV3Error> {
    validate_job(job)?;

    let mut triangles = parse_triangles(&job.triangles_xyz);
    project_triangles_inplace(&mut triangles, job);
    let index_start = std::time::Instant::now();
    let layer_index = build_layer_index(&triangles, job.total_layers, job.layer_height_mm);
    let index_ns = index_start.elapsed().as_nanos() as u64;

    let (rendered_layers, layer_area_stats, mut perf) = render_layers_rle(
        job,
        &triangles,
        &layer_index,
        compute_area_stats,
        on_rle_layer,
        on_progress,
        cancel_flag,
    )?;
    perf.index_build_ns = index_ns;

    Ok((rendered_layers, layer_area_stats, perf))
}

/// Fast raster stage that encodes RLE runs into layer bytes in parallel rayon workers.
pub fn slice_and_rasterize_rle_encoded_v3(
    job: &SliceJobV3,
    compute_area_stats: bool,
    encode_fn: Arc<
        dyn Fn(u32, &[crate::rle::RleRun]) -> Result<Vec<u8>, SlicerV3Error> + Send + Sync,
    >,
    on_encoded_layer: impl FnMut(u32, Vec<u8>) -> Result<(), SlicerV3Error>,
    on_progress: Option<ProgressCallbackV3>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(RenderedLayersV3, Vec<LayerAreaStatsV3>, SlicingPerfV3), SlicerV3Error> {
    validate_job(job)?;

    let mut triangles = parse_triangles(&job.triangles_xyz);
    project_triangles_inplace(&mut triangles, job);
    let index_start = std::time::Instant::now();
    let layer_index = build_layer_index(&triangles, job.total_layers, job.layer_height_mm);
    let index_ns = index_start.elapsed().as_nanos() as u64;

    let (rendered_layers, layer_area_stats, mut perf) = render_layers_rle_encoded(
        job,
        &triangles,
        &layer_index,
        compute_area_stats,
        encode_fn,
        on_encoded_layer,
        on_progress,
        cancel_flag,
    )?;
    perf.index_build_ns = index_ns;

    Ok((rendered_layers, layer_area_stats, perf))
}

/// Format-agnostic geometry/index/raster stage that outputs layer PNG bytes.
pub fn slice_and_rasterize_v3(
    job: &SliceJobV3,
    requires_area_stats: bool,
    emit_png_layers: bool,
    emit_raw_mask_layers: bool,
    on_raw_mask_layer: Option<&mut dyn FnMut(u32, Vec<u8>) -> Result<(), SlicerV3Error>>,
    on_progress: Option<ProgressCallbackV3>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(RenderedLayersV3, Vec<LayerAreaStatsV3>, SlicingPerfV3), SlicerV3Error> {
    validate_job(job)?;

    let mut triangles = parse_triangles(&job.triangles_xyz);
    project_triangles_inplace(&mut triangles, job);
    let index_start = std::time::Instant::now();
    let layer_index = build_layer_index(&triangles, job.total_layers, job.layer_height_mm);
    let index_ns = index_start.elapsed().as_nanos() as u64;

    let (rendered_layers, layer_area_stats, mut perf) = render_layers_bounded(
        job,
        &triangles,
        &layer_index,
        requires_area_stats,
        emit_png_layers,
        emit_raw_mask_layers,
        on_raw_mask_layer,
        on_progress,
        cancel_flag,
    )?;
    perf.index_build_ns = index_ns;

    Ok((rendered_layers, layer_area_stats, perf))
}

/// Decode a single 1-based layer preview PNG from an encoded print artifact,
/// dispatching through the registered format plugin by hint or file extension.
pub fn read_layer_preview_png_by_format_hint(
    source_path: &Path,
    layer_number: u32,
    format_hint: &str,
) -> Result<Vec<u8>, SlicerV3Error> {
    let Some(encoder) = find_encoder_by_hint_or_source(format_hint, source_path) else {
        let requested = format_hint.trim();
        let requested_display = if requested.is_empty() {
            source_path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| format!(".{}", ext.trim_start_matches('.')))
                .unwrap_or_else(|| "(unknown)".to_string())
        } else {
            requested.to_string()
        };

        return Err(SlicerV3Error::UnsupportedOutput(format!(
            "{} (supported: {})",
            requested_display,
            supported_output_formats().join(", ")
        )));
    };

    encoder.read_layer_preview_png(source_path, layer_number)
}

/// Encode rendered layers through a registered format encoder.
pub fn dispatch_encode_by_format(
    job: &SliceJobV3,
    rendered_layers: &RenderedLayersV3,
    layer_area_stats: &[LayerAreaStatsV3],
    on_encode_progress: Option<&dyn Fn(u32, u32)>,
) -> Result<Vec<u8>, SlicerV3Error> {
    let Some(encoder) = find_encoder(&job.output_format) else {
        return Err(SlicerV3Error::UnsupportedOutput(format!(
            "{} (supported: {})",
            job.output_format,
            supported_output_formats().join(", ")
        )));
    };
    encoder.encode_container_from_rendered_layers_with_progress(
        job,
        rendered_layers,
        layer_area_stats,
        on_encode_progress,
    )
}

/// Encode rendered layers through a registered format encoder directly to disk.
pub fn dispatch_encode_by_format_to_path(
    job: &SliceJobV3,
    rendered_layers: &RenderedLayersV3,
    layer_area_stats: &[LayerAreaStatsV3],
    output_path: &Path,
    on_encode_progress: Option<&dyn Fn(u32, u32)>,
) -> Result<(), SlicerV3Error> {
    let Some(encoder) = find_encoder(&job.output_format) else {
        return Err(SlicerV3Error::UnsupportedOutput(format!(
            "{} (supported: {})",
            job.output_format,
            supported_output_formats().join(", ")
        )));
    };
    encoder.encode_container_to_path_with_progress(
        job,
        rendered_layers,
        layer_area_stats,
        output_path,
        on_encode_progress,
    )
}

/// Path-oriented entrypoint that writes final archive bytes directly to disk.
///
/// This avoids materializing the final encoded artifact in-memory at the call
/// boundary, reducing bridge and copy overhead in desktop/Tauri flows.
pub fn slice_with_progress_v3_to_path(
    job: &SliceJobV3,
    output_path: &Path,
    on_progress: Option<ProgressCallbackV3>,
    cancel_flag: Option<&AtomicBool>,
) -> Result<SlicingPerfV3, SlicerV3Error> {
    let mut job_padded = job.clone();
    if job.enable_z_perturbation && job.blur_mode_z != "None" && job.blur_radius_z > 0 {
        job_padded.total_layers = job.total_layers + job.blur_radius_z;
    }
    let job = &job_padded;

    let Some(encoder) = find_encoder(&job.output_format) else {
        return Err(SlicerV3Error::UnsupportedOutput(format!(
            "{} (supported: {})",
            job.output_format,
            supported_output_formats().join(", ")
        )));
    };
    let requires_area_stats = encoder.requires_area_stats();
    let requires_png_layers = encoder.requires_png_layers();
    let requires_raw_mask_layers = encoder.requires_raw_mask_layers();
    let aa_enabled = job.anti_aliasing_level.trim() != "Off";
    let has_blurs = aa_enabled && (job.blur_mode_xy != "None" || (job.enable_z_perturbation && job.blur_mode_z != "None"));

    // RLE path: no full-image pixel buffer — fastest for formats like CTBv5.
    if !requires_png_layers && !has_blurs {
        if let Some(mut rle_enc) = encoder.create_rle_stream_encoder(job)? {
            let total_start = std::time::Instant::now();
            let job_total_layers = job.total_layers;
            let progress_total = job_total_layers.saturating_add(1);

            let slicing_progress = on_progress.as_ref().map(|cb| {
                let cb = cb.clone();
                Arc::new(move |update: SliceProgressUpdateV3| {
                    cb(SliceProgressUpdateV3 {
                        done: update.done.min(job_total_layers),
                        total: progress_total,
                        phase: SliceProgressPhaseV3::Slicing,
                    });
                }) as ProgressCallbackV3
            });

            let (_rendered_layers, layer_area_stats, mut perf) = if let Some(encode_fn) =
                rle_enc.parallel_encode_fn()
            {
                let mut store_sink = |idx: u32, bytes: Vec<u8>| -> Result<(), SlicerV3Error> {
                    rle_enc.store_encoded_layer(idx, bytes);
                    Ok(())
                };
                slice_and_rasterize_rle_encoded_v3(
                    job,
                    requires_area_stats,
                    encode_fn,
                    &mut store_sink,
                    slicing_progress,
                    cancel_flag,
                )?
            } else {
                let mut rle_sink =
                    |idx: u32, runs: Vec<crate::rle::RleRun>| rle_enc.consume_rle_layer(idx, runs);
                slice_and_rasterize_rle_v3(
                    job,
                    requires_area_stats,
                    &mut rle_sink,
                    slicing_progress,
                    cancel_flag,
                )?
            };

            rle_enc.set_area_stats(layer_area_stats);

            if let Some(cb) = on_progress.as_ref() {
                cb(SliceProgressUpdateV3 {
                    done: job_total_layers,
                    total: progress_total,
                    phase: SliceProgressPhaseV3::Finalizing,
                });
            }

            let encode_start = std::time::Instant::now();
            rle_enc.finalize_to_path(output_path)?;

            if let Some(cb) = on_progress.as_ref() {
                cb(SliceProgressUpdateV3 {
                    done: progress_total,
                    total: progress_total,
                    phase: SliceProgressPhaseV3::Finalizing,
                });
            }

            perf.archive_encode_ns = encode_start.elapsed().as_nanos() as u64;
            perf.total_ns = total_start.elapsed().as_nanos() as u64;
            perf.layers = job.total_layers;

            return Ok(perf);
        }
    }

    if !requires_png_layers && requires_raw_mask_layers && !has_blurs {
        if let Some(mut stream_encoder) = encoder.create_raw_mask_stream_encoder(job)? {
            let total_start = std::time::Instant::now();
            let job_total_layers = job.total_layers;
            let progress_total = job_total_layers.saturating_add(1);
            let mut raw_mask_sink = |layer_index: u32, raw_mask: Vec<u8>| {
                stream_encoder.consume_raw_mask_layer(layer_index, raw_mask)
            };

            let slicing_progress = on_progress.as_ref().map(|cb| {
                let cb = cb.clone();
                Arc::new(move |update: SliceProgressUpdateV3| {
                    cb(SliceProgressUpdateV3 {
                        done: update.done.min(job_total_layers),
                        total: progress_total,
                        phase: SliceProgressPhaseV3::Slicing,
                    });
                }) as ProgressCallbackV3
            });

            let (_rendered_layers, _layer_area_stats, mut perf) = slice_and_rasterize_v3(
                job,
                false,
                requires_png_layers,
                false,
                Some(&mut raw_mask_sink),
                slicing_progress,
                cancel_flag,
            )?;

            if let Some(cb) = on_progress.as_ref() {
                cb(SliceProgressUpdateV3 {
                    done: job_total_layers,
                    total: progress_total,
                    phase: SliceProgressPhaseV3::Finalizing,
                });
            }

            let encode_start = std::time::Instant::now();
            stream_encoder.finalize_to_path(output_path)?;

            if let Some(cb) = on_progress.as_ref() {
                cb(SliceProgressUpdateV3 {
                    done: progress_total,
                    total: progress_total,
                    phase: SliceProgressPhaseV3::Finalizing,
                });
            }

            perf.archive_encode_ns = encode_start.elapsed().as_nanos() as u64;
            perf.total_ns = total_start.elapsed().as_nanos() as u64;
            perf.layers = job.total_layers;

            return Ok(perf);
        }
    }

    let total_start = std::time::Instant::now();

    let (rendered_layers, layer_area_stats, mut perf) = slice_and_rasterize_v3(
        job,
        requires_area_stats,
        requires_png_layers,
        requires_raw_mask_layers,
        None,
        on_progress.clone(),
        cancel_flag,
    )?;

    let encode_units = encoder
        .estimate_encode_progress_units(&rendered_layers)
        .max(1);
    let progress_total = job.total_layers.saturating_add(encode_units);

    let encode_progress = on_progress.as_ref().map(|cb| {
        let cb = cb.clone();
        move |done: u32, total: u32| {
            let safe_total = total.max(1);
            let clamped_done = done.min(safe_total);
            let normalized =
                ((clamped_done as u64) * (encode_units as u64) / (safe_total as u64)) as u32;
            cb(SliceProgressUpdateV3 {
                done: job.total_layers.saturating_add(normalized),
                total: progress_total,
                phase: SliceProgressPhaseV3::Encoding,
            });
        }
    });

    let encode_start = std::time::Instant::now();
    dispatch_encode_by_format_to_path(
        job,
        &rendered_layers,
        &layer_area_stats,
        output_path,
        encode_progress.as_ref().map(|cb| cb as &dyn Fn(u32, u32)),
    )?;

    if let Some(cb) = on_progress.as_ref() {
        cb(SliceProgressUpdateV3 {
            done: progress_total,
            total: progress_total,
            phase: SliceProgressPhaseV3::Finalizing,
        });
    }

    perf.archive_encode_ns = encode_start.elapsed().as_nanos() as u64;
    perf.total_ns = total_start.elapsed().as_nanos() as u64;
    perf.layers = job.total_layers;

    Ok(perf)
}


pub(crate) fn apply_lut_or_floor_to_masks_attenuated(
    job: &SliceJobV3,
    masks: &mut [Vec<u8>],
) {
    let aa_enabled = job.anti_aliasing_level.trim() != "Off";
    if !aa_enabled {
        return;
    }

    let custom_lut = match job.normalized_custom_cure_lut() {
        Some(lut) => lut,
        None => return, // "No LUT" mode: skips the LUT step entirely, passing raw blurred coverage
    };

    use rayon::prelude::*;
    masks.par_iter_mut().for_each(|mask| {
        for pixel in mask.iter_mut() {
            if *pixel == 0 {
                continue;
            }
            *pixel = custom_lut[*pixel as usize];
        }
    });
}

pub(crate) fn composite_supports_in_parallel_offset(
    job: &SliceJobV3,
    support_triangles: &[crate::geometry::Triangle],
    support_layer_index: &crate::index::LayerIndex,
    masks: &mut [Vec<u8>],
    start_global_idx: usize,
) {
    let mut support_job = job.clone();
    support_job.anti_aliasing_level = "Off".to_string();
    support_job.enable_z_perturbation = false;
    let support_job_ref = &support_job;

    masks.par_iter_mut().enumerate().for_each(|(layer_idx, mask)| {
        let global_layer = (start_global_idx + layer_idx) as u32;
        let support_candidates = support_layer_index.candidates_for_layer(global_layer);
        if !support_candidates.is_empty() {
            let (s_mask, _s_stats) = crate::raster::rasterize_layer_with_stats(
                support_job_ref,
                support_triangles,
                support_candidates,
                global_layer,
                false,
                false,
            );
            
            for (m, &s) in mask.iter_mut().zip(s_mask.iter()) {
                if s > 0 {
                    *m = s;
                }
            }
            crate::pipeline::return_mask_to_pool(s_mask);
        }
    });
}

pub(crate) fn composite_supports_in_parallel(
    job: &SliceJobV3,
    support_triangles: &[crate::geometry::Triangle],
    support_layer_index: &crate::index::LayerIndex,
    masks: &mut [Vec<u8>],
) {
    composite_supports_in_parallel_offset(job, support_triangles, support_layer_index, masks, 0);
}

pub(crate) fn recalculate_area_stats_parallel(
    masks: &[Vec<u8>],
    width: usize,
    height: usize,
    pixel_area_mm2: f64,
) -> Vec<LayerAreaStatsV3> {
    masks
        .par_iter()
        .map(|mask| {
            let mut stats = LayerAreaStatsV3::default();
            if mask.is_empty() {
                return stats;
            }
            // Find bounding box
            let mut min_x = i32::MAX;
            let mut max_x = i32::MIN;
            let mut min_y = i32::MAX;
            let mut max_y = i32::MIN;
            let mut total_pixels = 0u32;

            for y in 0..height {
                let offset = y * width;
                let row = &mask[offset..offset + width];
                for x in 0..width {
                    if row[x] > 0 {
                        total_pixels += 1;
                        let xi = x as i32;
                        let yi = y as i32;
                        if xi < min_x { min_x = xi; }
                        if xi > max_x { max_x = xi; }
                        if yi < min_y { min_y = yi; }
                        if yi > max_y { max_y = yi; }
                    }
                }
            }

            if total_pixels > 0 {
                stats.total_solid_pixels = total_pixels;
                stats.min_x = min_x;
                stats.max_x = max_x;
                stats.min_y = min_y;
                stats.max_y = max_y;

                let roi_w = (max_x - min_x + 1) as usize;
                let roi_h = (max_y - min_y + 1) as usize;
                let min_x = min_x as usize;
                let min_y = min_y as usize;
                let max_x = max_x as usize;
                let max_y = max_y as usize;

                let mut visited = vec![false; roi_w * roi_h];
                let mut stack = Vec::<usize>::new();
                let mut largest_pixels = 0u32;
                let mut smallest_pixels = u32::MAX;
                let mut area_count = 0u32;

                for y in min_y..=max_y {
                    for x in min_x..=max_x {
                        let local_idx = (y - min_y) * roi_w + (x - min_x);
                        let idx = y * width + x;
                        if mask[idx] == 0 || visited[local_idx] {
                            continue;
                        }

                        area_count = area_count.saturating_add(1);
                        let mut component_pixels = 0u32;

                        visited[local_idx] = true;
                        stack.push(local_idx);

                        while let Some(cur_local) = stack.pop() {
                            component_pixels = component_pixels.saturating_add(1);

                            let ly = cur_local / roi_w;
                            let lx = cur_local % roi_w;
                            let gy = min_y + ly;
                            let gx = min_x + lx;

                            let y0 = gy.saturating_sub(1).max(min_y);
                            let y1 = (gy + 1).min(max_y);
                            let x0 = gx.saturating_sub(1).max(min_x);
                            let x1 = (gx + 1).min(max_x);

                            for ny in y0..=y1 {
                                for nx in x0..=x1 {
                                    if nx == gx && ny == gy {
                                        continue;
                                    }
                                    let nidx = ny * width + nx;
                                    let nlocal = (ny - min_y) * roi_w + (nx - min_x);
                                    if mask[nidx] == 0 || visited[nlocal] {
                                        continue;
                                    }
                                    visited[nlocal] = true;
                                    stack.push(nlocal);
                                }
                            }
                        }

                        if component_pixels > largest_pixels {
                            largest_pixels = component_pixels;
                        }
                        if component_pixels < smallest_pixels {
                            smallest_pixels = component_pixels;
                        }
                    }
                }

                stats.total_solid_area_mm2 = (total_pixels as f64) * pixel_area_mm2;
                stats.largest_area_mm2 = (largest_pixels as f64) * pixel_area_mm2;
                stats.smallest_area_mm2 = if area_count > 0 {
                    (smallest_pixels as f64) * pixel_area_mm2
                } else {
                    0.0
                };
                stats.area_count = area_count;
            }
            stats
        })
        .collect()
}

impl From<zip::result::ZipError> for SlicerV3Error {
    fn from(value: zip::result::ZipError) -> Self {
        Self::Zip(value.to_string())
    }
}

impl From<std::io::Error> for SlicerV3Error {
    fn from(value: std::io::Error) -> Self {
        Self::Zip(value.to_string())
    }
}

impl From<serde_json::Error> for SlicerV3Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}

#[allow(dead_code)]
fn _empty_perf() -> SlicingPerfV3 {
    SlicingPerfV3::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_lut_or_floor_to_masks_attenuated() {
        let mut job = SliceJobV3 {
            output_format: ".placeholder".to_string(),
            format_version: None,
            width_px: 3,
            height_px: 1,
            source_width_px: 3,
            source_height_px: 1,
            x_packing_mode: "none".to_string(),
            layer_height_mm: 0.05,
            build_width_mm: 100.0,
            build_depth_mm: 100.0,
            total_layers: 1,
            export_thumbnail_png_base64: None,
            png_compression_strategy: "fastest".to_string(),
            container_compression_level: 0,
            anti_aliasing_level: "8x".to_string(),
            aa_on_supports: false,
            minimum_aa_alpha_percent: 35.0,
            mirror_x: false,
            mirror_y: false,
            enable_z_perturbation: false,
            z_perturbation_mode: "Uniform".to_string(),
            duplicate_z_height: false,
            blur_mode_xy: "None".to_string(),
            blur_mode_z: "None".to_string(),
            blur_radius_xy: 0,
            blur_radius_z: 0,
            sigma_x: 1.0,
            sigma_y: 1.0,
            sigma_z: 1.0,
            triangles_xyz: Vec::new(),
            triangles_supports_xyz: None,
            z_blend_custom_lut: None,
            metadata_json: "{}".to_string(),
        };

        // 1. Test case: Custom LUT
        // Let's define a custom LUT mapping 120 -> 200, 255 -> 255.
        let mut custom_lut = vec![0u8; 256];
        custom_lut[120] = 200;
        custom_lut[255] = 255;
        job.z_blend_custom_lut = Some(custom_lut);

        // A single-row mask of 3 pixels:
        // idx 0: value 0 (background)
        // idx 1: value 120
        // idx 2: value 120
        let mut masks = vec![vec![0u8, 120, 120]];

        apply_lut_or_floor_to_masks_attenuated(&job, &mut masks);

        // Assertions:
        // idx 0 remains 0
        assert_eq!(masks[0][0], 0);
        // Both idx 1 and idx 2 (value 120) are mapped to 200 directly (no attenuation!)
        assert_eq!(masks[0][1], 200);
        assert_eq!(masks[0][2], 200);

        // 2. Test case: No LUT Bypass Mode
        job.z_blend_custom_lut = None;
        let mut masks_floor = vec![vec![0u8, 40, 40]];

        apply_lut_or_floor_to_masks_attenuated(&job, &mut masks_floor);

        // Assertions:
        // Raw blurred mask values are completely unaffected and passed directly because LUT remapping was bypassed:
        assert_eq!(masks_floor[0][0], 0);
        assert_eq!(masks_floor[0][1], 40);
        assert_eq!(masks_floor[0][2], 40);
    }
}
