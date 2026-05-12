#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_startup;
mod island_scan;
mod local_backup;
mod logging;
mod mesh_export;
mod mesh_repair;
mod network;
mod plugin_registry;
mod print_io;
mod scene_autosave;
mod scene_files;
mod slicing_staging;
mod temp_artifacts;
mod window_commands;

use rayon::{ThreadPool, ThreadPoolBuilder};
use serde::Deserialize;
use serde::Serialize;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Emitter;

// Runtime type aliases — the feat/cef branch requires an explicit runtime
// generic. These select Cef or Wry based on the active cargo feature.
#[cfg(feature = "tauri-cef")]
type DragonFruitAppHandle = tauri::AppHandle<tauri::Cef>;
#[cfg(not(feature = "tauri-cef"))]
type DragonFruitAppHandle = tauri::AppHandle<tauri::Wry>;

#[cfg(feature = "tauri-cef")]
type DragonFruitWindow = tauri::Window<tauri::Cef>;
#[cfg(not(feature = "tauri-cef"))]
type DragonFruitWindow = tauri::Window<tauri::Wry>;

/// Scene file extensions contributed by built-in fileType plugins — auto-generated, do not edit.
use plugin_registry::GENERATED_BUILTIN_PLUGIN_SCENE_FILE_EXTENSIONS as BUILTIN_PLUGIN_SCENE_EXTENSIONS;

pub(crate) fn build_save_dialog_with_filters(suggested_name: &str) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().set_file_name(suggested_name);

    let maybe_ext = std::path::Path::new(suggested_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|ext| !ext.is_empty());

    if let Some(ext) = maybe_ext.as_deref() {
        dialog = match ext {
            "stl" | "obj" | "3mf" => dialog.add_filter("Mesh Files", &["stl", "obj", "3mf"]),
            "voxl" => dialog.add_filter("Scene Files", &["voxl"]),
            x if BUILTIN_PLUGIN_SCENE_EXTENSIONS.contains(&x) => {
                dialog.add_filter("Scene Files", BUILTIN_PLUGIN_SCENE_EXTENSIONS)
            }
            "json" => dialog.add_filter("JSON Files", &["json"]),
            _ => dialog.add_filter("Print Files", &[ext]),
        };
    }

    dialog
}

static SLICER_POOL: OnceLock<ThreadPool> = OnceLock::new();
static CANCEL_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static STAGED_MESH: OnceLock<Mutex<Option<Vec<u8>>>> = OnceLock::new();
static STAGED_MESH_STATS: OnceLock<Mutex<StageMeshStats>> = OnceLock::new();
static STAGED_MESH_FILE_PATH: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static STAGED_MESH_FILE_APPENDER: OnceLock<Mutex<Option<StageFileAppender>>> = OnceLock::new();
const STAGED_MESH_PREALLOC_MIN_BYTES: usize = 16 * 1024 * 1024;
const STAGED_MESH_PREALLOC_MAX_BYTES: usize = 1024 * 1024 * 1024;

pub(crate) struct StageFileAppender {
    pub path: String,
    pub writer: std::io::BufWriter<std::fs::File>,
    #[allow(dead_code)]
    pub len: u64,
}

#[derive(Default)]
pub(crate) struct StageMeshStats {
    pub chunks_received: u64,
    pub append_ns_total: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StageMeshChunkAck {
    chunk_bytes: u64,
    total_bytes: u64,
    capacity_bytes: u64,
    reserve_grew: bool,
    chunks_received: u64,
    append_ns: u64,
    append_ns_total: u64,
}

pub(crate) fn staged_mesh() -> &'static Mutex<Option<Vec<u8>>> {
    STAGED_MESH.get_or_init(|| Mutex::new(None))
}

pub(crate) fn staged_mesh_stats() -> &'static Mutex<StageMeshStats> {
    STAGED_MESH_STATS.get_or_init(|| Mutex::new(StageMeshStats::default()))
}

pub(crate) fn staged_mesh_file_path() -> &'static Mutex<Option<String>> {
    STAGED_MESH_FILE_PATH.get_or_init(|| Mutex::new(None))
}

pub(crate) fn staged_mesh_file_appender() -> &'static Mutex<Option<StageFileAppender>> {
    STAGED_MESH_FILE_APPENDER.get_or_init(|| Mutex::new(None))
}

fn resolve_mesh_stage_directory() -> std::path::PathBuf {
    let configured = std::env::var("DF_MESH_STAGE_DIR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);

    configured.unwrap_or_else(std::env::temp_dir)
}

fn allocate_mesh_stage_file_path() -> Result<std::path::PathBuf, String> {
    let mut dir = resolve_mesh_stage_directory();
    std::fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "Failed creating mesh stage directory '{}': {e}",
            dir.display()
        )
    })?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.push(format!("dragonfruit-mesh-stage-{stamp}.bin"));
    Ok(dir)
}

fn normalize_staged_mesh_prealloc_bytes(total_bytes_hint: usize) -> usize {
    total_bytes_hint
        .max(STAGED_MESH_PREALLOC_MIN_BYTES)
        .min(STAGED_MESH_PREALLOC_MAX_BYTES)
}

fn bytes_to_f32_vec(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() % 4 != 0 {
        return Err(format!(
            "Mesh binary length {} is not a multiple of 4 (f32 size)",
            bytes.len()
        ));
    }

    // Fast path on little-endian targets (Windows/macOS/Linux desktop):
    // memcpy raw bytes into a pre-sized Vec<f32> to avoid per-float conversion overhead.
    #[cfg(target_endian = "little")]
    {
        let count = bytes.len() / 4;
        let mut floats = vec![0.0f32; count];
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                floats.as_mut_ptr() as *mut u8,
                bytes.len(),
            );
        }
        return Ok(floats);
    }

    #[cfg(not(target_endian = "little"))]
    {
        let count = bytes.len() / 4;
        let mut floats = Vec::with_capacity(count);
        for chunk in bytes.chunks_exact(4) {
            floats.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        Ok(floats)
    }
}

fn bytes_into_f32_vec(bytes: Vec<u8>) -> Result<Vec<f32>, String> {
    if bytes.len() % 4 != 0 {
        return Err(format!(
            "Mesh binary length {} is not a multiple of 4 (f32 size)",
            bytes.len()
        ));
    }

    #[cfg(target_endian = "little")]
    {
        let ptr_addr = bytes.as_ptr() as usize;
        let aligned_for_f32 = ptr_addr % std::mem::align_of::<f32>() == 0;
        let capacity_bytes = bytes.capacity();

        // Reinterpret in-place only when alignment and capacity permit exact Vec layout conversion.
        if aligned_for_f32 && (capacity_bytes % 4 == 0) {
            let mut bytes = std::mem::ManuallyDrop::new(bytes);
            let ptr = bytes.as_mut_ptr() as *mut f32;
            let len_f32 = bytes.len() / 4;
            let cap_f32 = bytes.capacity() / 4;
            let floats = unsafe { Vec::from_raw_parts(ptr, len_f32, cap_f32) };
            return Ok(floats);
        }

        return bytes_to_f32_vec(&bytes);
    }

    #[cfg(not(target_endian = "little"))]
    {
        bytes_to_f32_vec(&bytes)
    }
}

/// Metadata-only slice job (no inline triangles — those come from staged binary).
#[derive(Deserialize)]
struct MeshQuantizationMetadata {
    min_x: f32,
    min_y: f32,
    min_z: f32,
    max_x: f32,
    max_y: f32,
    max_z: f32,
}

fn default_minimum_aa_alpha_percent() -> f32 {
    35.0
}

#[derive(Deserialize)]
struct SliceJobMetadata {
    output_format: String,
    #[serde(default)]
    format_version: Option<String>,
    #[serde(default)]
    output_path: Option<String>,
    source_width_px: u32,
    source_height_px: u32,
    width_px: u32,
    height_px: u32,
    #[serde(default)]
    x_packing_mode: Option<String>,
    png_compression_strategy: String,
    anti_aliasing_level: String,
    aa_on_supports: bool,
    #[serde(default = "default_minimum_aa_alpha_percent")]
    minimum_aa_alpha_percent: f32,
    #[serde(default)]
    mirror_x: bool,
    #[serde(default)]
    mirror_y: bool,
    container_compression_level: u8,
    build_width_mm: f32,
    build_depth_mm: f32,
    layer_height_mm: f32,
    total_layers: u32,
    export_thumbnail_png_base64: Option<String>,
    #[serde(default)]
    mesh_encoding: Option<String>,
    #[serde(default)]
    mesh_quantization: Option<MeshQuantizationMetadata>,
    metadata_json: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeSlicerPerfMetrics {
    total_ns: u64,
    index_build_ns: u64,
    render_wall_ns: u64,
    render_ns: u64,
    png_encode_ns: u64,
    archive_encode_ns: u64,
    layers: u32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeSlicerRuntimeMetrics {
    pool_threads: u32,
    max_concurrent: u32,
    queue_buffer: u32,
    build_profile: String,
    artifact_dir: String,
    mesh_stage_dir: String,
    metadata_parse_ns: u64,
    mesh_decode_ns: u64,
    artifact_metadata_ns: u64,
    wrapper_total_ns: u64,
    wrapper_overhead_ns: u64,
}

fn duration_ns_u64(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

fn phase_to_label(phase: dragonfruit_slicing_engine::types::SliceProgressPhaseV3) -> &'static str {
    match phase {
        dragonfruit_slicing_engine::types::SliceProgressPhaseV3::Slicing => "Slicing",
        dragonfruit_slicing_engine::types::SliceProgressPhaseV3::Encoding => "Encoding",
        dragonfruit_slicing_engine::types::SliceProgressPhaseV3::Finalizing => "Finalizing",
    }
}

fn slicer_pool() -> &'static ThreadPool {
    SLICER_POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        ThreadPoolBuilder::new()
            .thread_name(|i| format!("dragonfruit-slicing-engine-{i}"))
            .num_threads(threads)
            .build()
            .expect("failed to create slicer rayon thread pool")
    })
}

fn decode_quantized_u16_mesh(
    bytes: &[u8],
    quant: &MeshQuantizationMetadata,
) -> Result<Vec<f32>, String> {
    if bytes.len() % 2 != 0 {
        return Err(format!(
            "Quantized mesh binary length {} is not a multiple of 2 (u16 size)",
            bytes.len()
        ));
    }

    let spans = [
        (quant.max_x - quant.min_x).max(0.0),
        (quant.max_y - quant.min_y).max(0.0),
        (quant.max_z - quant.min_z).max(0.0),
    ];
    let mins = [quant.min_x, quant.min_y, quant.min_z];
    let values = bytes.len() / 2;
    let mut floats = Vec::with_capacity(values);

    for (index, chunk) in bytes.chunks_exact(2).enumerate() {
        let q = u16::from_le_bytes([chunk[0], chunk[1]]);
        let axis = index % 3;
        let span = spans[axis];
        if span <= 0.0 || !span.is_finite() {
            floats.push(mins[axis]);
            continue;
        }

        let normalized = (q as f32) / 65_535.0;
        floats.push(mins[axis] + normalized * span);
    }

    Ok(floats)
}

fn decode_mesh_bytes(bytes: Vec<u8>, meta: &SliceJobMetadata) -> Result<Vec<f32>, String> {
    match meta.mesh_encoding.as_deref() {
        Some("quantized_u16") => {
            let quant = meta
                .mesh_quantization
                .as_ref()
                .ok_or("Quantized mesh encoding requires mesh_quantization metadata")?;
            decode_quantized_u16_mesh(&bytes, quant)
        }
        Some("raw_f32") | None => bytes_into_f32_vec(bytes),
        Some(other) => Err(format!("Unsupported mesh encoding: {other}")),
    }
}

fn cancel_flag() -> &'static Arc<AtomicBool> {
    CANCEL_FLAG.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

/// Build a progress callback that throttles `window.emit` to at most ~60 fps.
///
/// The first event, any phase change, and the final event (done == total) are
/// always emitted immediately.  Intermediate updates are skipped if fewer than
/// 16 ms have elapsed since the last emit.
fn make_throttled_progress_cb(
    win: DragonFruitWindow,
) -> dragonfruit_slicing_engine::types::ProgressCallbackV3 {
    use std::sync::Mutex;
    use std::time::Instant;

    struct State {
        last_emit: Instant,
        last_phase: String,
    }

    let state = Arc::new(Mutex::new(State {
        last_emit: Instant::now(),
        last_phase: String::new(),
    }));

    Arc::new(
        move |update: dragonfruit_slicing_engine::types::SliceProgressUpdateV3| {
            let phase = phase_to_label(update.phase).to_string();
            let is_final = update.done >= update.total;

            let should_emit = {
                let mut s = state.lock().unwrap();
                let phase_changed = s.last_phase != phase;
                let elapsed = s.last_emit.elapsed().as_millis() >= 8;

                if is_final || phase_changed || elapsed {
                    s.last_emit = Instant::now();
                    s.last_phase = phase.clone();
                    true
                } else {
                    false
                }
            };

            if should_emit {
                let _ = win.emit(
                    "slicer://progress",
                    SliceProgressPayload {
                        done: update.done,
                        total: update.total,
                        phase,
                    },
                );
            }
        },
    )
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SliceProgressPayload {
    done: u32,
    total: u32,
    phase: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeSliceTempPathResult {
    temp_path: String,
    byte_len: u64,
    perf: Option<NativeSlicerPerfMetrics>,
    runtime: NativeSlicerRuntimeMetrics,
}

fn v3_runtime_metrics(
    output_path: &std::path::Path,
    metadata_parse_ns: u64,
    mesh_decode_ns: u64,
    artifact_metadata_ns: u64,
    wrapper_total_ns: u64,
    wrapper_overhead_ns: u64,
) -> NativeSlicerRuntimeMetrics {
    let hw_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let max_concurrent = std::env::var("DF_V3_MAX_CONCURRENT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(hw_threads)
        .clamp(1, hw_threads);
    let queue_buffer = (max_concurrent * 2).clamp(2, 16);

    let artifact_dir = output_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());

    let mesh_stage_dir = resolve_mesh_stage_directory().to_string_lossy().to_string();

    let build_profile = if cfg!(debug_assertions) {
        "debug".to_string()
    } else {
        "release".to_string()
    };

    NativeSlicerRuntimeMetrics {
        pool_threads: hw_threads as u32,
        max_concurrent: max_concurrent as u32,
        queue_buffer: queue_buffer as u32,
        build_profile,
        artifact_dir,
        mesh_stage_dir,
        metadata_parse_ns,
        mesh_decode_ns,
        artifact_metadata_ns,
        wrapper_total_ns,
        wrapper_overhead_ns,
    }
}

fn main() {
    app_startup::run();
}
