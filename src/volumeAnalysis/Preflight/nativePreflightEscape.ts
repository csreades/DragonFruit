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
}

export async function runPreflightEscapeNative(
  geom: { geometry: THREE.BufferGeometry; bbox: THREE.Box3 },
  layerHeightMm: number,
  opts: PreflightEscapeOpts,
): Promise<PreflightEscapeResult> {
  const positions = geom.geometry.getAttribute('position').array as Float32Array;
  const meshBytes = new Uint8Array(positions.buffer, positions.byteOffset, positions.byteLength);

  await core.invoke('stage_mesh_binary_set', meshBytes, {
    headers: { 'Content-Type': 'application/octet-stream' },
  });

  const bb = geom.bbox;
  const paramsJson = JSON.stringify({
    px_mm: opts.pxMm,
    layer_height_mm: layerHeightMm,
    layers: opts.layers,
    warn_um: opts.warnUm,
    bbox_min_x: bb.min.x,
    bbox_max_x: bb.max.x,
    bbox_min_y: bb.min.y,
    bbox_max_y: bb.max.y,
    bbox_min_z: bb.min.z,
    bbox_max_z: bb.max.z,
  });

  return await core.invoke<PreflightEscapeResult>('preflight_escape_native', { paramsJson });
}
