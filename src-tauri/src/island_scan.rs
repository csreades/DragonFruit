use serde::{Deserialize, Serialize};
use tauri::Emitter;

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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IslandScanProgressPayload {
    done: u32,
    total: u32,
    phase: String,
}

#[tauri::command]
pub(crate) async fn run_island_scan_native(
    window: crate::DragonFruitWindow,
    params_json: String,
) -> Result<NativeIslandScanResult, String> {
    // Take staged mesh bytes
    let mesh_bytes = crate::staged_mesh()
        .lock()
        .map_err(|e| format!("staged mesh lock poisoned: {e}"))?
        .take()
        .ok_or("No staged mesh binary — call stage_mesh_binary first")?;

    let win = window.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let params: IslandScanParams = serde_json::from_str(&params_json)
            .map_err(|e| format!("Invalid island scan params JSON: {e}"))?;

        let triangles_xyz = crate::bytes_to_f32_vec(&mesh_bytes)?;
        let triangles = dragonfruit_slicing_engine::geometry::parse_triangles(&triangles_xyz);

        // Debug dump: write positions + params to temp dir for offline reproduction
        let dump_dir = std::env::temp_dir().join("dragonfruit-island-debug");
        let _ = std::fs::create_dir_all(&dump_dir);
        let _ = std::fs::write(dump_dir.join("params.json"), &params_json);
        // Write positions as raw f32 binary (same format as stage_mesh_binary)
        let _ = std::fs::write(dump_dir.join("positions.bin"), &mesh_bytes);
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
        let (masks, gw, gh, num_layers, ox, oz) = crate::slicer_pool().install(|| {
            dragonfruit_islands::rasterize::rasterize_for_island_scan(
                &triangles,
                params.bbox_min_x,
                params.bbox_max_x,
                params.bbox_min_y,
                params.bbox_max_y,
                params.bbox_min_z,
                params.bbox_max_z,
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
        let scan_result = crate::slicer_pool().install(|| {
            dragonfruit_islands::pipeline::run_island_scan(
                &job,
                &masks,
                Some(&move |done: u32, total: u32| {
                    // Map pipeline progress (0..total) to layer count (0..total_layers)
                    // — same convention as TS ScanOrchestrator onProgress(done, numLayers)
                    let layer = (done as u64 * total_layers as u64 / total.max(1) as u64) as u32;
                    let _ = win_scan.emit(
                        "islandscan://progress",
                        IslandScanProgressPayload {
                            done: layer.min(total_layers),
                            total: total_layers,
                            phase: "Scanning".to_string(),
                        },
                    );
                }),
            )
        });
        let scan_ms = t_scan.elapsed().as_secs_f64() * 1000.0;
        let total_ms = rasterize_ms + scan_ms;

        let total_solid_px: u64 = masks.iter().map(|m| m.pixel_count()).sum();
        log::info!(
            "[island-scan-native] grid={}x{} layers={} solid_px={} islands={} raster={:.0}ms scan={:.0}ms",
            grid_width,
            grid_height,
            num_layers,
            total_solid_px,
            scan_result.islands.len(),
            rasterize_ms,
            scan_ms,
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
                                if run.id > 0 {
                                    base_labels[idx] = run.id;
                                }
                            }
                            last_hit[idx] = l as i16;
                        }
                    }
                }
            }
        }

        let base_footprint: Vec<u8> = first_hit
            .iter()
            .map(|&h| if h != -1 { 1 } else { 0 })
            .collect();

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
        let islands: Vec<NativeIsland> = scan_result
            .islands
            .iter()
            .map(|i| NativeIsland {
                id: i.id.0,
                first_layer: i.first_layer,
                last_layer: i.last_layer,
                status: match i.status {
                    dragonfruit_islands::model::IslandStatus::Active => "active".into(),
                    dragonfruit_islands::model::IslandStatus::Complete => "complete".into(),
                },
                total_area_mm2: i.total_area_mm2,
                per_layer_area_mm2: i
                    .per_layer_area_mm2
                    .iter()
                    .map(|(k, v)| (k.to_string(), *v))
                    .collect(),
                parent_id: i.parent_id.map(|p| p.0),
                child_ids: i.child_ids.iter().map(|c| c.0).collect(),
                volume_mm3: i.volume_mm3,
                max_area_mm2: i.max_area_mm2,
                max_area_layer: i.max_area_layer,
                is_merged_placeholder: i.is_merged_placeholder,
                centroid: i.centroid.map(|c| NativeCentroid {
                    x: c.x,
                    y: c.y,
                    z: c.z,
                }),
                last_layer_centroid: i.last_layer_centroid.map(|c| NativeCentroid {
                    x: c.x,
                    y: c.y,
                    z: c.z,
                }),
            })
            .collect();

        // Convert RLE labels to frontend shape (flat i32 arrays)
        let island_labels_per_layer: Vec<NativeRleLabels> = scan_result
            .island_labels_per_layer
            .iter()
            .map(|labels| NativeRleLabels {
                rows: labels
                    .rows
                    .iter()
                    .map(|row| {
                        let mut flat = Vec::new();
                        for run in row {
                            flat.push(run.start);
                            flat.push(run.length);
                            flat.push(run.id);
                        }
                        flat
                    })
                    .collect(),
                width: labels.width,
                height: labels.height,
            })
            .collect();

        Ok::<NativeIslandScanResult, String>(NativeIslandScanResult {
            grid: NativeGridRef {
                origin_x,
                origin_z,
                width: grid_width,
                height: grid_height,
                px_mm: params.px_mm,
            },
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
