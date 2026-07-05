/**
 * Pre-flight Check 1 (resin escape) — Tauri bridge.
 *
 * Mirrors nativeIslandScan.ts: stage the (world-transformed) mesh, then invoke
 * the Rust `preflight_escape_native` command. Returns the per-layer worst
 * lateral escape distance + the worst layer's heatmap as packed RGBA.
 *
 * Fields are snake_case: the Rust result struct is serialized as-is.
 */
import * as THREE from 'three';
import * as core from '@tauri-apps/api/core';

export interface PreflightLayerEscape {
  layer: number;
  max_escape_um: number;
  argmax: [number, number];
  flagged: boolean;
  drain_candidates: [number, number, number][]; // (x, y, um)
}

export interface PreflightEscapeResult {
  grid_width: number;
  grid_height: number;
  pitch_um: number;
  layers_checked: number;
  worst_layer: number;
  worst_escape_um: number;
  flagged_layers: number;
  heatmap_scale_um: number;
  per_layer: PreflightLayerEscape[];
  heatmap_width: number;
  heatmap_height: number;
  heatmap_rgba: number[]; // tightly packed RGBA of the worst layer
}

export interface PreflightEscapeOpts {
  pxMm: number;
  layers: number;
  warnUm: number;
  /**
   * 'full' (default): one EDT per bottom layer. 'quick': union the band into
   * one occupancy grid and run a single transform — the conservative flow
   * check, meant for a coarse pixel (~1 mm).
   */
  mode?: 'full' | 'quick';
}

/**
 * Single-shot staging is fine for one part, but bed-scope merges (every
 * visible model's soup) can run to hundreds of MB — stage those through the
 * same start/chunk path the slicer uses so no single IPC body carries the
 * whole buffer.
 */
const STAGE_CHUNK_BYTES = 32 * 1024 * 1024;

async function stageMeshBytes(meshBytes: Uint8Array): Promise<void> {
  if (meshBytes.byteLength <= STAGE_CHUNK_BYTES) {
    await core.invoke('stage_mesh_binary_set', meshBytes, {
      headers: { 'Content-Type': 'application/octet-stream' },
    });
    return;
  }
  await core.invoke('stage_mesh_binary_start', { totalBytes: meshBytes.byteLength });
  for (let offset = 0; offset < meshBytes.byteLength; offset += STAGE_CHUNK_BYTES) {
    const chunk = meshBytes.subarray(offset, Math.min(offset + STAGE_CHUNK_BYTES, meshBytes.byteLength));
    await core.invoke('stage_mesh_binary_chunk', chunk, {
      headers: { 'Content-Type': 'application/octet-stream' },
    });
  }
}

export async function runPreflightEscapeNative(
  geom: { geometry: THREE.BufferGeometry; bbox: THREE.Box3 },
  layerHeightMm: number,
  opts: PreflightEscapeOpts,
): Promise<PreflightEscapeResult> {
  const positions = geom.geometry.getAttribute('position').array as Float32Array;
  const meshBytes = new Uint8Array(positions.buffer, positions.byteOffset, positions.byteLength);

  await stageMeshBytes(meshBytes);

  const bb = geom.bbox;
  const paramsJson = JSON.stringify({
    px_mm: opts.pxMm,
    layer_height_mm: layerHeightMm,
    layers: opts.layers,
    warn_um: opts.warnUm,
    mode: opts.mode ?? 'full',
    bbox_min_x: bb.min.x,
    bbox_max_x: bb.max.x,
    bbox_min_y: bb.min.y,
    bbox_max_y: bb.max.y,
    // Geometry below the plate never prints (the slicer clips at z=0), so a
    // sunk model's band starts at the plate — where its printed bottom
    // cross-sections actually are. Floating models keep their mesh bottom:
    // each of their layers still cures against the FEP at one layer-height
    // gap, so height above the plate does not reduce squeeze-flow risk.
    bbox_min_z: Math.max(0, bb.min.z),
    bbox_max_z: bb.max.z,
  });

  return await core.invoke<PreflightEscapeResult>('preflight_escape_native', { paramsJson });
}
