use std::sync::atomic::Ordering;
use tauri::ipc::{InvokeBody, Response};

#[tauri::command]
pub(crate) async fn slice_solid_native(
    window: crate::DragonFruitWindow,
    job_json: String,
) -> Result<Response, String> {
    let flag = crate::cancel_flag().clone();
    flag.store(false, Ordering::SeqCst);

    let win = window.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let job: dragonfruit_slicing_engine::types::SliceJobV3 = serde_json::from_str(&job_json)
            .map_err(|err| format!("Invalid SliceJobV3 JSON: {err}"))?;

        let progress_cb = crate::make_throttled_progress_cb(win);

        crate::slicer_pool().install(|| -> Result<Vec<u8>, String> {
            let artifact = dragonfruit_slicing_engine::slice_with_progress_v3(
                &job,
                Some(progress_cb),
                Some(flag.as_ref()),
            )
            .map_err(|err| format!("V3 slicing failed: {err}"))?;
            Ok(artifact.bytes)
        })
    })
    .await
    .map_err(|err| format!("Native slicer task failed to join: {err}"))??;

    Ok(Response::new(bytes))
}

/// Receive raw mesh bytes from the frontend via efficient binary IPC in chunks.
/// The bytes are stored in a pre-allocated memory vector and consumed by the next `slice_solid_native_to_temp_path` call.
#[tauri::command]
pub(crate) async fn stage_mesh_binary_start(total_bytes: usize) -> Result<(), String> {
    let reserve_bytes = crate::normalize_staged_mesh_prealloc_bytes(total_bytes);
    *crate::staged_mesh()
        .lock()
        .map_err(|e| format!("staged mesh lock poisoned: {e}"))? =
        Some(Vec::with_capacity(reserve_bytes));

    *crate::staged_mesh_stats()
        .lock()
        .map_err(|e| format!("staged mesh stats lock poisoned: {e}"))? =
        crate::StageMeshStats::default();

    *crate::staged_mesh_file_path()
        .lock()
        .map_err(|e| format!("staged mesh file-path lock poisoned: {e}"))? = None;

    *crate::staged_mesh_file_appender()
        .lock()
        .map_err(|e| format!("staged mesh file appender lock poisoned: {e}"))? = None;

    Ok(())
}

#[tauri::command]
pub(crate) async fn allocate_mesh_stage_path() -> Result<String, String> {
    let path = crate::allocate_mesh_stage_file_path()?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub(crate) async fn append_mesh_stage_chunk(
    request: tauri::ipc::Request<'_>,
) -> Result<u64, String> {
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes,
        InvokeBody::Json(_) => {
            return Err("append_mesh_stage_chunk expects raw binary body, got JSON".into())
        }
    };

    let path_header = request
        .headers()
        .get("x-mesh-stage-path")
        .ok_or("append_mesh_stage_chunk missing x-mesh-stage-path header")?;

    let path_text = path_header
        .to_str()
        .map_err(|e| format!("Invalid x-mesh-stage-path header value: {e}"))?
        .trim();

    if path_text.is_empty() {
        return Err("append_mesh_stage_chunk received empty x-mesh-stage-path header".into());
    }

    let path = std::path::PathBuf::from(path_text);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed creating mesh stage directory: {err}"))?;
    }

    let offset_header = request.headers().get("x-mesh-stage-offset");
    let is_first_chunk = match offset_header {
        Some(value) => {
            let raw = value
                .to_str()
                .map_err(|e| format!("Invalid x-mesh-stage-offset header value: {e}"))?
                .trim();
            if raw.is_empty() {
                false
            } else {
                raw.parse::<u64>()
                    .map_err(|e| format!("Invalid x-mesh-stage-offset header value: {e}"))?
                    == 0
            }
        }
        None => false,
    };

    let mut appender_lock = crate::staged_mesh_file_appender()
        .lock()
        .map_err(|e| format!("staged mesh file appender lock poisoned: {e}"))?;

    let needs_new_appender = match appender_lock.as_ref() {
        Some(existing) => is_first_chunk || existing.path != path_text,
        None => true,
    };

    if needs_new_appender {
        if let Some(existing) = appender_lock.as_mut() {
            use std::io::Write;
            let _ = existing.writer.flush();
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|err| format!("Failed opening mesh stage file for write: {err}"))?;

        *appender_lock = Some(crate::StageFileAppender {
            path: path_text.to_string(),
            writer: std::io::BufWriter::with_capacity(4 * 1024 * 1024, file),
            len: 0,
        });
    }

    let appender = appender_lock
        .as_mut()
        .ok_or("Failed initializing mesh stage file appender")?;

    use std::io::Write;
    appender
        .writer
        .write_all(bytes)
        .map_err(|err| format!("Failed appending mesh stage bytes: {err}"))?;
    appender
        .writer
        .flush()
        .map_err(|err| format!("Failed flushing mesh stage bytes: {err}"))?;
    appender.len = appender.len.saturating_add(bytes.len() as u64);

    Ok(appender.len)
}

#[tauri::command]
pub(crate) async fn finish_mesh_stage_write(path: String) -> Result<u64, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("finish_mesh_stage_write requires a non-empty path".into());
    }

    let mut appender_lock = crate::staged_mesh_file_appender()
        .lock()
        .map_err(|e| format!("staged mesh file appender lock poisoned: {e}"))?;

    if let Some(appender) = appender_lock.as_mut() {
        if appender.path == trimmed {
            use std::io::Write;
            appender
                .writer
                .flush()
                .map_err(|e| format!("Failed flushing mesh stage file writer: {e}"))?;
            let len = appender.len;
            *appender_lock = None; // release OS file handle immediately
            return Ok(len);
        }
    }

    // No open appender for this path (already closed or never opened).
    Ok(0)
}

#[tauri::command]
pub(crate) async fn stage_mesh_file_path(mesh_file_path: String) -> Result<u64, String> {
    let trimmed = mesh_file_path.trim();
    if trimmed.is_empty() {
        return Err("stage_mesh_file_path requires a non-empty path".into());
    }

    let path = std::path::PathBuf::from(trimmed);
    if !path.exists() {
        return Err(format!(
            "Mesh stage file does not exist: {}",
            path.display()
        ));
    }

    {
        let mut appender_lock = crate::staged_mesh_file_appender()
            .lock()
            .map_err(|e| format!("staged mesh file appender lock poisoned: {e}"))?;
        if let Some(appender) = appender_lock.as_mut() {
            if appender.path == trimmed {
                use std::io::Write;
                appender
                    .writer
                    .flush()
                    .map_err(|e| format!("Failed flushing mesh stage file writer: {e}"))?;
            }
        }
    }

    let len = std::fs::metadata(&path)
        .map_err(|e| format!("Failed reading mesh stage file metadata: {e}"))?
        .len();

    *crate::staged_mesh()
        .lock()
        .map_err(|e| format!("staged mesh lock poisoned: {e}"))? = None;

    *crate::staged_mesh_file_path()
        .lock()
        .map_err(|e| format!("staged mesh file-path lock poisoned: {e}"))? =
        Some(path.to_string_lossy().to_string());

    *crate::staged_mesh_stats()
        .lock()
        .map_err(|e| format!("staged mesh stats lock poisoned: {e}"))? = crate::StageMeshStats {
        chunks_received: 1,
        append_ns_total: 0,
    };

    Ok(len)
}

#[tauri::command]
pub(crate) async fn stage_mesh_binary_set(
    request: tauri::ipc::Request<'_>,
) -> Result<crate::StageMeshChunkAck, String> {
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes,
        InvokeBody::Json(_) => {
            return Err("stage_mesh_binary_set expects raw binary body, got JSON".into())
        }
    };

    let reserve_bytes = crate::normalize_staged_mesh_prealloc_bytes(bytes.len());
    let append_start = std::time::Instant::now();
    let mut staged = Vec::with_capacity(reserve_bytes);
    staged.extend_from_slice(bytes);
    let append_ns = append_start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    let capacity_bytes = staged.capacity() as u64;
    let total_bytes = staged.len() as u64;

    *crate::staged_mesh()
        .lock()
        .map_err(|e| format!("staged mesh lock poisoned: {e}"))? = Some(staged);

    *crate::staged_mesh_stats()
        .lock()
        .map_err(|e| format!("staged mesh stats lock poisoned: {e}"))? = crate::StageMeshStats {
        chunks_received: 1,
        append_ns_total: append_ns,
    };

    *crate::staged_mesh_file_path()
        .lock()
        .map_err(|e| format!("staged mesh file-path lock poisoned: {e}"))? = None;

    *crate::staged_mesh_file_appender()
        .lock()
        .map_err(|e| format!("staged mesh file appender lock poisoned: {e}"))? = None;

    Ok(crate::StageMeshChunkAck {
        chunk_bytes: total_bytes,
        total_bytes,
        capacity_bytes,
        reserve_grew: capacity_bytes > reserve_bytes as u64,
        chunks_received: 1,
        append_ns,
        append_ns_total: append_ns,
    })
}

#[tauri::command]
pub(crate) async fn stage_mesh_binary_chunk(
    request: tauri::ipc::Request<'_>,
) -> Result<crate::StageMeshChunkAck, String> {
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes,
        InvokeBody::Json(_) => {
            return Err("stage_mesh_binary_chunk expects raw binary body, got JSON".into())
        }
    };

    let mut lock = crate::staged_mesh()
        .lock()
        .map_err(|e| format!("staged mesh lock poisoned: {e}"))?;

    let vec = lock
        .as_mut()
        .ok_or("Staged mesh not started. Call stage_mesh_binary_start first")?;

    let chunk_bytes = bytes.len() as u64;
    let capacity_before = vec.capacity();
    let append_start = std::time::Instant::now();
    vec.extend_from_slice(bytes);
    let append_ns = append_start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    let capacity_after = vec.capacity();
    let total_bytes = vec.len() as u64;

    drop(lock);

    let mut stats = crate::staged_mesh_stats()
        .lock()
        .map_err(|e| format!("staged mesh stats lock poisoned: {e}"))?;
    stats.chunks_received = stats.chunks_received.saturating_add(1);
    stats.append_ns_total = stats.append_ns_total.saturating_add(append_ns);

    Ok(crate::StageMeshChunkAck {
        chunk_bytes,
        total_bytes,
        capacity_bytes: capacity_after as u64,
        reserve_grew: capacity_after > capacity_before,
        chunks_received: stats.chunks_received,
        append_ns,
        append_ns_total: stats.append_ns_total,
    })
}

#[tauri::command]
pub(crate) async fn slice_solid_native_to_temp_path(
    window: crate::DragonFruitWindow,
    job_json: String,
) -> Result<crate::NativeSliceTempPathResult, String> {
    // Take the pre-staged mesh bytes (set by stage_mesh_binary)
    let staged_mesh_bytes = crate::staged_mesh()
        .lock()
        .map_err(|e| format!("staged mesh lock poisoned: {e}"))?
        .take();

    let mesh_bytes = if let Some(bytes) = staged_mesh_bytes {
        bytes
    } else {
        {
            let mut appender_lock = crate::staged_mesh_file_appender()
                .lock()
                .map_err(|e| format!("staged mesh file appender lock poisoned: {e}"))?;
            if let Some(appender) = appender_lock.as_mut() {
                use std::io::Write;
                appender
                    .writer
                    .flush()
                    .map_err(|e| format!("Failed flushing mesh stage file writer: {e}"))?;
            }
        }

        let staged_path = crate::staged_mesh_file_path()
            .lock()
            .map_err(|e| format!("staged mesh file-path lock poisoned: {e}"))?
            .take()
            .ok_or("No staged mesh binary or file path — call stage_mesh_binary_* or stage_mesh_file_path first")?;

        let path = std::path::PathBuf::from(staged_path.trim());
        if !path.exists() {
            return Err(format!(
                "Staged mesh file no longer exists: {}",
                path.display()
            ));
        }

        let bytes = std::fs::read(&path)
            .map_err(|e| format!("Failed reading staged mesh file '{}': {e}", path.display()))?;

        let _ = std::fs::remove_file(&path);
        *crate::staged_mesh_file_appender()
            .lock()
            .map_err(|e| format!("staged mesh file appender lock poisoned: {e}"))? = None;
        bytes
    };

    let flag = crate::cancel_flag().clone();
    flag.store(false, Ordering::SeqCst);

    let win = window.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let wrapper_start = std::time::Instant::now();

        let metadata_parse_start = std::time::Instant::now();
        let meta: crate::SliceJobMetadata = serde_json::from_str(&job_json)
            .map_err(|err| format!("Invalid slice job metadata JSON: {err}"))?;
        let metadata_parse_ns = crate::duration_ns_u64(metadata_parse_start.elapsed());
        let requested_output_path = meta
            .output_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let mesh_decode_start = std::time::Instant::now();
        let triangles_xyz = crate::decode_mesh_bytes(mesh_bytes, &meta)?;
        let mesh_decode_ns = crate::duration_ns_u64(mesh_decode_start.elapsed());

        let job = dragonfruit_slicing_engine::SliceJobV3 {
            output_format: meta.output_format,
            format_version: meta.format_version,
            source_width_px: meta.source_width_px,
            source_height_px: meta.source_height_px,
            width_px: meta.width_px,
            height_px: meta.height_px,
            x_packing_mode: meta.x_packing_mode.unwrap_or_else(|| "none".to_string()),
            png_compression_strategy: meta.png_compression_strategy,
            anti_aliasing_level: meta.anti_aliasing_level,
            aa_on_supports: meta.aa_on_supports,
            minimum_aa_alpha_percent: meta.minimum_aa_alpha_percent,
            mirror_x: meta.mirror_x,
            mirror_y: meta.mirror_y,
            container_compression_level: meta.container_compression_level,
            build_width_mm: meta.build_width_mm,
            build_depth_mm: meta.build_depth_mm,
            layer_height_mm: meta.layer_height_mm,
            total_layers: meta.total_layers,
            export_thumbnail_png_base64: meta.export_thumbnail_png_base64,
            triangles_xyz,
            metadata_json: meta.metadata_json,
        };

        let progress_cb = crate::make_throttled_progress_cb(win);
        let requested_output_path = requested_output_path.clone();

        crate::slicer_pool().install(
            || -> Result<(String, u64, crate::NativeSlicerPerfMetrics, crate::NativeSlicerRuntimeMetrics), String> {
            let ext = if job.output_format.trim().is_empty() {
                let format_provider = crate::plugin_registry::get_format_provider()
                    .unwrap_or_else(|_| crate::plugin_registry::get_default_format_provider());
                format_provider.default_export_format()
            } else {
                job.output_format.trim_start_matches('.')
            };
            let path = if let Some(requested_path) = requested_output_path.as_deref() {
                let requested = std::path::PathBuf::from(requested_path);
                if let Some(parent) = requested.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|err| format!("Failed creating output folder: {err}"))?;
                }
                requested
            } else {
                crate::temp_artifact_path(ext)
            };

            let perf_raw = dragonfruit_slicing_engine::engine::slice_with_progress_v3_to_path(
                &job,
                &path,
                Some(progress_cb),
                Some(flag.as_ref()),
            )
            .map_err(|err| format!("V3 slicing failed: {err}"))?;

            let perf = crate::NativeSlicerPerfMetrics {
                total_ns: perf_raw.total_ns,
                index_build_ns: perf_raw.index_build_ns,
                render_wall_ns: perf_raw.render_wall_ns,
                render_ns: perf_raw.render_ns,
                png_encode_ns: perf_raw.png_encode_ns,
                archive_encode_ns: perf_raw.archive_encode_ns,
                layers: perf_raw.layers,
            };

            let artifact_metadata_start = std::time::Instant::now();
            let byte_len = std::fs::metadata(&path)
                .map_err(|err| format!("Failed reading temp artifact metadata: {err}"))?
                .len();
            let artifact_metadata_ns = crate::duration_ns_u64(artifact_metadata_start.elapsed());

            let wrapper_total_ns = crate::duration_ns_u64(wrapper_start.elapsed());
            let wrapper_overhead_ns = wrapper_total_ns.saturating_sub(perf_raw.total_ns);
            let runtime = crate::v3_runtime_metrics(
                &path,
                metadata_parse_ns,
                mesh_decode_ns,
                artifact_metadata_ns,
                wrapper_total_ns,
                wrapper_overhead_ns,
            );

            Ok((
                path.to_string_lossy().to_string(),
                byte_len,
                perf,
                runtime,
            ))
        },
        )
    })
    .await
    .map_err(|err| format!("Native slicer task failed to join: {err}"))??;

    Ok(crate::NativeSliceTempPathResult {
        temp_path: result.0,
        byte_len: result.1,
        perf: Some(result.2),
        runtime: result.3,
    })
}

#[tauri::command]
pub(crate) async fn cancel_slicing() -> Result<(), String> {
    crate::cancel_flag().store(true, Ordering::SeqCst);
    Ok(())
}
