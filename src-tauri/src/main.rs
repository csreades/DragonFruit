#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod local_backup;
mod logging;
mod mesh_repair;
mod network;
mod print_io;
mod window_commands;
fn default_minimum_aa_alpha_percent() -> f32 {
    35.0
}
mod plugin_registry;
mod scene_autosave;
mod scene_files;
mod slicing_staging;

use rayon::{ThreadPool, ThreadPoolBuilder};
use serde::Deserialize;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::ipc::{InvokeBody, Response};
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

fn temp_artifact_path(extension: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let clean_ext = extension.trim_start_matches('.');
    path.push(format!("dragonfruit-slice-{stamp}.{clean_ext}"));
    path
}

pub(crate) fn is_dragonfruit_temp_artifact(path: &std::path::Path) -> bool {
    let file_name_ok = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("dragonfruit-slice-"))
        .unwrap_or(false);
    let in_temp_dir = path.starts_with(std::env::temp_dir());
    file_name_ok && in_temp_dir
}

pub(crate) fn sweep_stale_temp_artifacts(max_age_seconds: u64) -> u32 {
    let mut removed = 0u32;
    let temp_dir = std::env::temp_dir();
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(max_age_seconds))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let entries = match std::fs::read_dir(&temp_dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_dragonfruit_temp_artifact(&path) {
            continue;
        }

        let stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|modified| modified <= cutoff)
            .unwrap_or(false);

        if stale && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    removed
}

pub(crate) fn sweep_all_temp_artifacts() -> u32 {
    let mut removed = 0u32;
    let temp_dir = std::env::temp_dir();
    let entries = match std::fs::read_dir(&temp_dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_dragonfruit_temp_artifact(&path) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    removed
}

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

// ---------------------------------------------------------------------------
// Native mesh export (STL / 3MF)
// ---------------------------------------------------------------------------

/// Read a little-endian `f32` from a byte slice at the given byte offset.
#[inline(always)]
fn read_f32_le(data: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

/// Write binary STL directly to `dest_path` from raw triangle staging data.
///
/// Staging format: 9 × f32 per triangle (v0x v0y v0z  v1x v1y v1z  v2x v2y v2z), LE.
fn write_binary_stl(raw: &[u8], tri_count: usize, dest_path: &str) -> Result<(), String> {
    use std::io::{BufWriter, Write};

    let file =
        std::fs::File::create(dest_path).map_err(|e| format!("Failed creating STL file: {e}"))?;
    let mut w = BufWriter::with_capacity(256 * 1024, file);

    // 80-byte header (all zeros)
    w.write_all(&[0u8; 80])
        .map_err(|e| format!("STL write: {e}"))?;
    w.write_all(&(tri_count as u32).to_le_bytes())
        .map_err(|e| format!("STL write: {e}"))?;

    for i in 0..tri_count {
        let base = i * 36; // 9 floats × 4 bytes
        let v0x = read_f32_le(raw, base);
        let v0y = read_f32_le(raw, base + 4);
        let v0z = read_f32_le(raw, base + 8);
        let v1x = read_f32_le(raw, base + 12);
        let v1y = read_f32_le(raw, base + 16);
        let v1z = read_f32_le(raw, base + 20);
        let v2x = read_f32_le(raw, base + 24);
        let v2y = read_f32_le(raw, base + 28);
        let v2z = read_f32_le(raw, base + 32);

        // Face normal = cross(v1 - v0, v2 - v0), normalized
        let e1x = v1x - v0x;
        let e1y = v1y - v0y;
        let e1z = v1z - v0z;
        let e2x = v2x - v0x;
        let e2y = v2y - v0y;
        let e2z = v2z - v0z;
        let mut nx = e1y * e2z - e1z * e2y;
        let mut ny = e1z * e2x - e1x * e2z;
        let mut nz = e1x * e2y - e1y * e2x;
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        if len > 1e-30 {
            let inv = 1.0 / len;
            nx *= inv;
            ny *= inv;
            nz *= inv;
        }

        // Normal
        w.write_all(&nx.to_le_bytes())
            .map_err(|e| format!("STL write: {e}"))?;
        w.write_all(&ny.to_le_bytes())
            .map_err(|e| format!("STL write: {e}"))?;
        w.write_all(&nz.to_le_bytes())
            .map_err(|e| format!("STL write: {e}"))?;
        // 3 vertices
        w.write_all(&raw[base..base + 36])
            .map_err(|e| format!("STL write: {e}"))?;
        // Attribute byte count
        w.write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("STL write: {e}"))?;
    }

    w.flush().map_err(|e| format!("STL flush: {e}"))?;
    Ok(())
}

/// Write a 3MF file (DEFLATE-compressed ZIP) directly to `dest_path` from raw
/// triangle staging data.
///
/// The `zip` crate handles DEFLATE compression, CRC32, and central directory
/// bookkeeping.  XML text for vertex/triangle tags compresses ~10–20× with
/// DEFLATE, so the resulting 3MF is typically smaller than the equivalent
/// binary STL.
fn write_3mf(raw: &[u8], tri_count: usize, dest_path: &str) -> Result<(), String> {
    use std::io::{BufWriter, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    let file =
        std::fs::File::create(dest_path).map_err(|e| format!("Failed creating 3MF file: {e}"))?;
    // Large outer buffer so 4 MB compressed chunks land on disk efficiently.
    let buf_writer = BufWriter::with_capacity(4 * 1024 * 1024, file);
    let mut zip = ZipWriter::new(buf_writer);

    // Level 1 = fastest deflate. Repetitive XML (vertex/triangle tags) still
    // compresses 5–8× even at level 1, so output size stays reasonable while
    // the compressor runs ~5× faster than the default level 6.
    let deflate_opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));

    // ── [Content_Types].xml ──
    zip.start_file("[Content_Types].xml", deflate_opts)
        .map_err(|e| format!("3MF zip: {e}"))?;
    zip.write_all(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
          <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
          <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
          <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\
          </Types>",
    )
    .map_err(|e| format!("3MF zip: {e}"))?;

    // ── _rels/.rels ──
    zip.start_file("_rels/.rels", deflate_opts)
        .map_err(|e| format!("3MF zip: {e}"))?;
    zip.write_all(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
          <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
          <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" \
          Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\
          </Relationships>",
    )
    .map_err(|e| format!("3MF zip: {e}"))?;

    // ── 3D/3dmodel.model ──
    // The old approach called zip.write_all once per vertex/triangle — for a
    // 2 M-triangle mesh that is ~8 M individual DEFLATE feed calls, each with
    // full compressor overhead. Instead we accumulate XML into a 4 MB in-memory
    // chunk and flush to DEFLATE in bulk, reducing compressor calls by >1000×.
    zip.start_file("3D/3dmodel.model", deflate_opts)
        .map_err(|e| format!("3MF zip: {e}"))?;

    const CHUNK: usize = 4 * 1024 * 1024;
    let mut buf: Vec<u8> = Vec::with_capacity(CHUNK + 512);

    buf.extend_from_slice(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
          <model unit=\"millimeter\" xml:lang=\"en-US\" \
          xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\
          <resources><object id=\"1\" type=\"model\"><mesh><vertices>",
    );

    // ── Vertices ──
    for i in 0..tri_count {
        let base = i * 36;
        for j in 0..3usize {
            let fbase = base + j * 12;
            let x = read_f32_le(raw, fbase);
            let y = read_f32_le(raw, fbase + 4);
            let z = read_f32_le(raw, fbase + 8);
            write!(buf, "<vertex x=\"{x:.4}\" y=\"{y:.4}\" z=\"{z:.4}\"/>").unwrap();
        }
        // Flush every ~4 MB so we never hold more than one chunk in memory.
        if buf.len() >= CHUNK {
            zip.write_all(&buf).map_err(|e| format!("3MF zip: {e}"))?;
            buf.clear();
        }
    }

    buf.extend_from_slice(b"</vertices><triangles>");

    // ── Triangles (sequential indices: tri i → 3i, 3i+1, 3i+2) ──
    for i in 0..tri_count {
        let v0 = i * 3;
        write!(
            buf,
            "<triangle v1=\"{v0}\" v2=\"{}\" v3=\"{}\"/>",
            v0 + 1,
            v0 + 2
        )
        .unwrap();
        if buf.len() >= CHUNK {
            zip.write_all(&buf).map_err(|e| format!("3MF zip: {e}"))?;
            buf.clear();
        }
    }

    buf.extend_from_slice(
        b"</triangles></mesh></object></resources>\
          <build><item objectid=\"1\"/></build></model>",
    );
    zip.write_all(&buf).map_err(|e| format!("3MF zip: {e}"))?;

    zip.finish().map_err(|e| format!("3MF zip finish: {e}"))?;
    Ok(())
}

/// Exports raw staged geometry to a properly formatted mesh file.
///
/// JS sends raw triangle vertex data (9 × f32 LE per triangle) to a staging
/// file via `append_mesh_stage_chunk`, then calls this command to convert
/// the staging file into a valid STL or 3MF at the user-chosen destination.
#[tauri::command]
async fn export_mesh_file(
    staging_path: String,
    dest_path: String,
    format: String,
) -> Result<String, String> {
    // Flush and release the staged file appender if it was writing to our staging file,
    // so all buffered bytes are written before we read.
    {
        let mut lock = staged_mesh_file_appender()
            .lock()
            .map_err(|e| format!("Appender lock poisoned: {e}"))?;
        let matches = lock.as_ref().map_or(false, |a| a.path == staging_path);
        if matches {
            if let Some(appender) = lock.as_mut() {
                use std::io::Write;
                appender
                    .writer
                    .flush()
                    .map_err(|e| format!("Failed flushing staging appender: {e}"))?;
            }
            *lock = None; // release file handle
        }
    }

    let raw = std::fs::read(&staging_path)
        .map_err(|e| format!("Failed reading staging file '{}': {e}", staging_path))?;

    if raw.len() % 36 != 0 {
        return Err(format!(
            "Invalid staging data: {} bytes is not a multiple of 36 (9 × f32 per triangle)",
            raw.len()
        ));
    }
    let tri_count = raw.len() / 36;
    if tri_count == 0 {
        return Err("Cannot export: no triangles in staged geometry.".into());
    }

    log::info!(
        "[export_mesh_file] {} triangles → {} format → {}",
        tri_count,
        format,
        dest_path
    );

    match format.as_str() {
        "stl" => write_binary_stl(&raw, tri_count, &dest_path)?,
        "3mf" => write_3mf(&raw, tri_count, &dest_path)?,
        _ => return Err(format!("Unsupported export format: {format}")),
    }

    // Clean up staging file
    let _ = std::fs::remove_file(&staging_path);

    Ok(dest_path)
}

// ---------------------------------------------------------------------------
// Native Island Scan
// ---------------------------------------------------------------------------

/// Parameters for native island scan. Field names match the JSON keys sent by
/// nativeIslandScan.ts (snake_case).
#[derive(Deserialize)]
struct IslandScanParams {
    px_mm: f64,
    support_buffer_mm: f64,
    #[serde(default = "default_connectivity")]
    connectivity: u8,
    #[serde(default = "default_min_island_area")]
    min_island_area_mm2: f64,
    #[serde(default = "default_overlap_px")]
    min_overlap_px: i32,
    #[serde(default = "default_neighborhood_px")]
    overlap_neighborhood_px: i32,
    layer_height_mm: f64,
    // Bounding box from frontend (world coords after transform)
    bbox_min_x: f64,
    bbox_max_x: f64,
    bbox_min_y: f64,
    bbox_max_y: f64,
    bbox_min_z: f64,
    bbox_max_z: f64,
}

fn default_connectivity() -> u8 {
    4
}
fn default_min_island_area() -> f64 {
    0.01
}
fn default_overlap_px() -> i32 {
    1
}
fn default_neighborhood_px() -> i32 {
    1
}

/// IPC result matching the TS `ScanResults` shape so the frontend overlay/voxel
/// rendering works unchanged.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeIslandScanResult {
    grid: NativeGridRef,
    islands: Vec<NativeIsland>,
    island_labels_per_layer: Vec<NativeRleLabels>,
    // Derived arrays the frontend overlay needs
    first_hit: Vec<i16>,
    last_hit: Vec<i16>,
    base_footprint: Vec<u8>,
    base_labels: Vec<i32>,
    comp_base: Vec<i16>,
    comp_top: Vec<i16>,
    // Perf
    rasterize_ms: f64,
    scan_ms: f64,
    total_ms: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeGridRef {
    origin_x: f64,
    origin_z: f64,
    width: i32,
    height: i32,
    px_mm: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeIsland {
    id: u32,
    first_layer: u32,
    last_layer: u32,
    status: String,
    total_area_mm2: f64,
    per_layer_area_mm2: std::collections::HashMap<String, f64>,
    parent_id: Option<u32>,
    child_ids: Vec<u32>,
    volume_mm3: Option<f64>,
    max_area_mm2: Option<f64>,
    max_area_layer: Option<u32>,
    is_merged_placeholder: bool,
    centroid: Option<NativeCentroid>,
    last_layer_centroid: Option<NativeCentroid>,
}

#[derive(Serialize)]
struct NativeCentroid {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Serialize)]
struct NativeRleLabels {
    rows: Vec<Vec<i32>>,
    width: i32,
    height: i32,
}

#[tauri::command]
async fn run_island_scan_native(
    window: DragonFruitWindow,
    params_json: String,
) -> Result<NativeIslandScanResult, String> {
    // Take staged mesh bytes
    let mesh_bytes = staged_mesh()
        .lock()
        .map_err(|e| format!("staged mesh lock poisoned: {e}"))?
        .take()
        .ok_or("No staged mesh binary — call stage_mesh_binary first")?;

    let win = window.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let params: IslandScanParams = serde_json::from_str(&params_json)
            .map_err(|e| format!("Invalid island scan params JSON: {e}"))?;

        let triangles_xyz = bytes_to_f32_vec(&mesh_bytes)?;
        let triangles = dragonfruit_slicing_engine::geometry::parse_triangles(&triangles_xyz);

        // Debug dump: write positions + params to temp dir for offline reproduction
        let dump_dir = std::env::temp_dir().join("dragonfruit-island-debug");
        let _ = std::fs::create_dir_all(&dump_dir);
        let _ = std::fs::write(
            dump_dir.join("params.json"),
            &params_json,
        );
        // Write positions as raw f32 binary (same format as stage_mesh_binary)
        let _ = std::fs::write(
            dump_dir.join("positions.bin"),
            &mesh_bytes,
        );
        log::debug!(
            "[island-scan-native] triangles={} bbox=({:.4},{:.4},{:.4})-({:.4},{:.4},{:.4}) px_mm={} layer_h={} buf={} conn={} min_area={} overlap_px={} neighborhood={}",
            triangles.len(),
            params.bbox_min_x, params.bbox_min_y, params.bbox_min_z,
            params.bbox_max_x, params.bbox_max_y, params.bbox_max_z,
            params.px_mm, params.layer_height_mm, params.support_buffer_mm,
            params.connectivity, params.min_island_area_mm2,
            params.min_overlap_px, params.overlap_neighborhood_px,
        );
        log::debug!("[island-scan-native] debug dump: {}", dump_dir.display());

        // Phase A: Rasterize all layers using shared module (same code as bench)
        let total_layers;
        let grid_width;
        let grid_height;
        let origin_x;
        let origin_z;
        let w;
        let h;

        let t_raster = std::time::Instant::now();
        let (masks, gw, gh, num_layers, ox, oz) = slicer_pool().install(|| {
            dragonfruit_islands::rasterize::rasterize_for_island_scan(
                &triangles,
                params.bbox_min_x, params.bbox_max_x,
                params.bbox_min_y, params.bbox_max_y,
                params.bbox_min_z, params.bbox_max_z,
                params.px_mm,
                params.layer_height_mm,
            )
        });
        grid_width = gw;
        grid_height = gh;
        origin_x = ox;
        origin_z = oz;
        w = grid_width as usize;
        h = grid_height as usize;
        total_layers = num_layers as u32;
        let rasterize_ms = t_raster.elapsed().as_secs_f64() * 1000.0;

        // Phase B: Island scan pipeline (sequential tracking — progress per layer)
        let t_scan = std::time::Instant::now();
        let connectivity = if params.connectivity == 8 {
            dragonfruit_islands::model::Connectivity::Eight
        } else {
            dragonfruit_islands::model::Connectivity::Four
        };

        let job = dragonfruit_islands::model::IslandScanJob {
            px_mm: params.px_mm,
            support_buffer_mm: params.support_buffer_mm,
            connectivity,
            min_island_area_mm2: params.min_island_area_mm2,
            layer_height_mm: params.layer_height_mm,
            grid: dragonfruit_islands::model::GridRef {
                origin_x,
                origin_z,
                width: grid_width,
                height: grid_height,
                px_mm: params.px_mm,
            },
            num_layers: num_layers as u32,
            min_overlap_px: params.min_overlap_px,
            overlap_neighborhood_px: params.overlap_neighborhood_px,
        };

        let win_scan = win.clone();
        let scan_result = slicer_pool().install(|| {
            dragonfruit_islands::pipeline::run_island_scan(
                &job,
                &masks,
                Some(&move |done: u32, total: u32| {
                    // Map pipeline progress (0..total) to layer count (0..total_layers)
                    // — same convention as TS ScanOrchestrator onProgress(done, numLayers)
                    let layer = (done as u64 * total_layers as u64 / total.max(1) as u64) as u32;
                    let _ = win_scan.emit("islandscan://progress", SliceProgressPayload {
                        done: layer.min(total_layers),
                        total: total_layers,
                        phase: "Scanning".to_string(),
                    });
                }),
            )
        });
        let scan_ms = t_scan.elapsed().as_secs_f64() * 1000.0;
        let total_ms = rasterize_ms + scan_ms;

        let total_solid_px: u64 = masks.iter().map(|m| m.pixel_count()).sum();
        log::info!(
            "[island-scan-native] grid={}x{} layers={} solid_px={} islands={} raster={:.0}ms scan={:.0}ms",
            grid_width, grid_height, num_layers, total_solid_px,
            scan_result.islands.len(), rasterize_ms, scan_ms,
        );

        // Phase C: Build frontend-compatible result
        // Compute firstHit, lastHit, baseLabels, compBase, compTop
        let pixel_count = w * h;
        let mut first_hit = vec![-1i16; pixel_count];
        let mut last_hit = vec![-1i16; pixel_count];
        let mut base_labels = vec![0i32; pixel_count];

        for (l, labels) in scan_result.island_labels_per_layer.iter().enumerate() {
            for (y, row) in labels.rows.iter().enumerate() {
                let row_off = y * w;
                for run in row {
                    for j in 0..run.length {
                        let idx = row_off + (run.start + j) as usize;
                        if idx < pixel_count {
                            if first_hit[idx] == -1 {
                                first_hit[idx] = l as i16;
                                if run.id > 0 { base_labels[idx] = run.id; }
                            }
                            last_hit[idx] = l as i16;
                        }
                    }
                }
            }
        }

        let base_footprint: Vec<u8> = first_hit.iter().map(|&h| if h != -1 { 1 } else { 0 }).collect();

        // compBase/compTop from islands
        let max_id = scan_result.islands.iter().map(|i| i.id.0).max().unwrap_or(0) as usize;
        let mut comp_base = vec![-1i16; max_id + 1];
        let mut comp_top = vec![-1i16; max_id + 1];
        for island in &scan_result.islands {
            let id = island.id.0 as usize;
            if id <= max_id {
                comp_base[id] = island.first_layer as i16;
                comp_top[id] = island.last_layer as i16;
            }
        }

        // Convert islands to frontend shape
        let islands: Vec<NativeIsland> = scan_result.islands.iter().map(|i| {
            NativeIsland {
                id: i.id.0,
                first_layer: i.first_layer,
                last_layer: i.last_layer,
                status: match i.status {
                    dragonfruit_islands::model::IslandStatus::Active => "active".into(),
                    dragonfruit_islands::model::IslandStatus::Complete => "complete".into(),
                },
                total_area_mm2: i.total_area_mm2,
                per_layer_area_mm2: i.per_layer_area_mm2.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
                parent_id: i.parent_id.map(|p| p.0),
                child_ids: i.child_ids.iter().map(|c| c.0).collect(),
                volume_mm3: i.volume_mm3,
                max_area_mm2: i.max_area_mm2,
                max_area_layer: i.max_area_layer,
                is_merged_placeholder: i.is_merged_placeholder,
                centroid: i.centroid.map(|c| NativeCentroid { x: c.x, y: c.y, z: c.z }),
                last_layer_centroid: i.last_layer_centroid.map(|c| NativeCentroid { x: c.x, y: c.y, z: c.z }),
            }
        }).collect();

        // Convert RLE labels to frontend shape (flat i32 arrays)
        let island_labels_per_layer: Vec<NativeRleLabels> = scan_result.island_labels_per_layer.iter().map(|labels| {
            NativeRleLabels {
                rows: labels.rows.iter().map(|row| {
                    let mut flat = Vec::new();
                    for run in row { flat.push(run.start); flat.push(run.length); flat.push(run.id); }
                    flat
                }).collect(),
                width: labels.width,
                height: labels.height,
            }
        }).collect();

        Ok::<NativeIslandScanResult, String>(NativeIslandScanResult {
            grid: NativeGridRef { origin_x, origin_z, width: grid_width, height: grid_height, px_mm: params.px_mm },
            islands,
            island_labels_per_layer,
            first_hit,
            last_hit,
            base_footprint,
            base_labels,
            comp_base,
            comp_top,
            rasterize_ms,
            scan_ms,
            total_ms,
        })
    })
    .await
    .map_err(|e| format!("Island scan task failed to join: {e}"))??;

    Ok(result)
}

fn main() {
    // Issue #83: on Linux with Nvidia+Wayland, WebKitGTK's DMA-BUF renderer
    // can crash inside gbm_bo_create_with_modifiers. Setting this env var
    // before any GTK/WebKit init forces the SHM fallback path, which is
    // slightly slower but stable. Only applies to the wry (WebKitGTK) path;
    // CEF uses its own compositor and is unaffected.
    #[cfg(all(target_os = "linux", not(feature = "tauri-cef")))]
    {
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            // NOTE: called at the very start of main, single-threaded, before
            // any other threads or libraries are initialized. On Rust edition
            // 2024+ this will require an unsafe block (set_var became unsafe).
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    // Install a panic hook that writes the panic location and message to the
    // DragonFruit log before the default handler runs.  This ensures that even
    // hard crashes leave a human-readable trace in the log file rather than
    // only a terse Windows Event 1005 entry.
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let message: String = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "(non-string panic payload)".to_string()
        };
        log::error!("[panic] {message} at {location}");
        default_panic_hook(info);
    }));

    // Sweep week-old stale temp artifacts on app startup.
    let _ = sweep_stale_temp_artifacts(7 * 24 * 60 * 60);

    // Initialize plugin registry and register built-in plugins
    if let Err(error) = plugin_registry::initialize_plugins() {
        if cfg!(debug_assertions) {
            log::warn!("[plugin-registry] WARNING: {error}");
        } else {
            panic!("Failed to initialize plugin registry: {error}");
        }
    }

    log::info!(
        "DragonFruit {} starting (debug={})",
        env!("CARGO_PKG_VERSION"),
        cfg!(debug_assertions)
    );

    let _log_level = logging::read_log_level_pref();
    // Log plugin disabled on CEF — it pulls tauri/wry transitively, causing
    // E0252 collision. See Cargo.toml comment.
    #[cfg(not(feature = "tauri-cef"))]
    let log_plugin = {
        use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};
        LogBuilder::new()
            .targets([
                Target::new(TargetKind::Stdout),
                Target::new(TargetKind::LogDir {
                    file_name: Some("dragonfruit".to_string()),
                }),
                Target::new(TargetKind::Webview),
            ])
            .level(_log_level)
            // Suppress chatty low-level transport crates — these flood the log
            // at DEBUG/TRACE and contain no actionable application information.
            .level_for("tungstenite", log::LevelFilter::Warn)
            .level_for("reqwest", log::LevelFilter::Warn)
            .level_for("hyper", log::LevelFilter::Warn)
            .level_for("hyper_util", log::LevelFilter::Warn)
            .level_for("rustls", log::LevelFilter::Warn)
            .level_for("h2", log::LevelFilter::Warn)
            .level_for("tokio_tungstenite", log::LevelFilter::Warn)
            .max_file_size(5_000_000)
            .rotation_strategy(RotationStrategy::KeepOne)
            .build()
    };

    let builder = tauri::Builder::default();
    #[cfg(not(feature = "tauri-cef"))]
    let builder = builder.plugin(log_plugin);
    let builder = builder.setup(|app| {
        let app_handle = app.handle().clone();

        // Defer main window creation to an async task so the splashscreen's
        // WebView2 instance fully initialises before the main window's does.
        // On Windows, simultaneous WebView2 init produces a brief window flash
        // even when the main window is created with visible=false.
        tauri::async_runtime::spawn(async move {
            use tauri::WebviewWindowBuilder;

            // Give the splash WebView2 a head-start on Windows only.
            #[cfg(target_os = "windows")]
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;

            let config = app_handle.config();
            let Some(window_config) = config.app.windows.iter().find(|w| w.label == "main") else {
                log::error!("Missing 'main' window config in tauri.conf.json");
                return;
            };

            let builder = match WebviewWindowBuilder::from_config(&app_handle, window_config) {
                Ok(b) => b,
                Err(e) => {
                    log::error!("Failed to create main window builder: {e}");
                    return;
                }
            };

            // Keep custom titlebar behavior on non-macOS.
            #[cfg(not(target_os = "macos"))]
            let builder = builder.decorations(false);

            // Belt-and-suspenders: enforce hidden + no taskbar entry while
            // the window is invisible, regardless of config values. On Windows
            // a taskbar button flash is the tell-tale sign the window was
            // briefly visible during WebView2 initialisation.
            let builder = builder.visible(false);
            #[cfg(target_os = "windows")]
            let builder = builder.skip_taskbar(true);

            match builder.build() {
                Ok(_window) => {
                    // On macOS, reveal immediately so a frontend startup hiccup
                    // can't leave the app invisible when created as hidden.
                    #[cfg(target_os = "macos")]
                    {
                        if let Err(error) = _window.show() {
                            log::warn!("Failed to show main window during setup: {error}");
                        }
                    }
                    log::info!("Main window created successfully");
                }
                Err(e) => log::error!("Failed to build main window: {e}"),
            }
        });

        Ok(())
    });

    // Single-instance plugin also disabled on CEF (same wry collision).
    #[cfg(not(feature = "tauri-cef"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        log::info!("Single-instance activated: second launch detected (args={argv:?})");
        let has_scene_files = !scene_files::collect_scene_file_paths_from_args(&argv).is_empty();
        window_commands::emit_scene_file_handoff(app, &argv, "single-instance");

        // Only force foreground when this second launch is handing off a scene file.
        // Avoiding unconditional focus here reduces Windows "error" chimes when users
        // launch the app again while it's already running.
        if has_scene_files {
            window_commands::focus_main_window(app);
        }
    }));

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_plugin_macos_fps::init());

    builder
        .invoke_handler(tauri::generate_handler![
            slicing_staging::slice_solid_native,
            slicing_staging::stage_mesh_binary_start,
            slicing_staging::allocate_mesh_stage_path,
            slicing_staging::append_mesh_stage_chunk,
            slicing_staging::finish_mesh_stage_write,
            slicing_staging::stage_mesh_file_path,
            slicing_staging::stage_mesh_binary_set,
            slicing_staging::stage_mesh_binary_chunk,
            slicing_staging::slice_solid_native_to_temp_path,
            slicing_staging::cancel_slicing,
            run_island_scan_native,
            export_mesh_file,
            print_io::save_print_file,
            print_io::save_print_file_from_path,
            print_io::pick_save_path,
            print_io::pick_open_files,
            print_io::get_launch_scene_files,
            window_commands::get_slicer_engine_version,
            window_commands::notify_launch_scene_handoff,
            window_commands::focus_main_window_command,
            window_commands::reveal_main_window_command,
            print_io::write_bytes_to_path,
            print_io::read_print_file_bytes,
            print_io::read_print_file_size,
            print_io::read_print_file_chunk,
            print_io::read_print_layer_png,
            print_io::delete_print_temp_file,
            print_io::cleanup_stale_print_temp_files,
            print_io::cleanup_all_print_temp_files,
            local_backup::local_backup_default_directory,
            local_backup::local_backup_pick_directory,
            local_backup::local_backup_read_state,
            local_backup::local_backup_sync,
            local_backup::local_backup_list_history,
            local_backup::local_backup_read_history_item,
            local_backup::local_backup_delete_history_item,
            local_backup::local_backup_restore_history_item,
            scene_autosave::scene_autosave_get_paths,
            scene_autosave::scene_autosave_write_manifest,
            scene_autosave::scene_autosave_read_manifest,
            scene_autosave::scene_autosave_read_voxl_bytes,
            print_io::reveal_in_file_manager,
            logging::set_log_level_pref,
            logging::read_log_tail,
            logging::open_log_file,
            logging::delete_log_file,
            network::plugin_network_request,
            network::ensure_rtsp_relay,
            mesh_repair::mesh_analyze_from_path,
            mesh_repair::mesh_analyze_staged,
            mesh_repair::mesh_repair_from_path,
            mesh_repair::mesh_repair_staged,
            mesh_repair::mesh_classify_staged,
            mesh_repair::mesh_repair_read_positions
        ])
        .run(tauri::generate_context!())
        .expect("error while running DragonFruit desktop app");
}
