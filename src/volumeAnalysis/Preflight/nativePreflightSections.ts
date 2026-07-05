/**
 * Pre-flight Check 2 (geometry mode) — Tauri bridge.
 *
 * Stages the (world-transformed) part mesh and runs the native full-section
 * peel analyzer. Works on ANY geometry — parts, imported pre-supported meshes,
 * native supports rendered to mesh — because it analyses cross-sections, not
 * support primitives. Same staging pattern as nativePreflightEscape.ts.
 */
import * as THREE from 'three';
import * as core from '@tauri-apps/api/core';

export interface SectionNeck {
  layer: number;
  x_mm: number;
  y_mm: number;
  sf: number;
  band: 'fail' | 'marginal' | 'ok';
  area_mm2: number;
  peel_above_mm2: number;
}

export interface PreflightSectionsResult {
  component_count: number;
  region_count: number;
  layers_analyzed: number;
  worst_sf: number;
  fail_count: number;
  marginal_count: number;
  necks: SectionNeck[];
}

export interface PreflightSectionsOpts {
  pxMm: number;
  greenMpa?: number;
  peelMpa?: number;
  maxNecks?: number;
}

export async function runPreflightSectionsNative(
  geom: { geometry: THREE.BufferGeometry; bbox: THREE.Box3 },
  layerHeightMm: number,
  opts: PreflightSectionsOpts,
): Promise<PreflightSectionsResult> {
  const positions = geom.geometry.getAttribute('position').array as Float32Array;
  const meshBytes = new Uint8Array(positions.buffer, positions.byteOffset, positions.byteLength);

  await core.invoke('stage_mesh_binary_set', meshBytes, {
    headers: { 'Content-Type': 'application/octet-stream' },
  });

  const bb = geom.bbox;
  const paramsJson = JSON.stringify({
    px_mm: opts.pxMm,
    layer_height_mm: layerHeightMm,
    green_mpa: opts.greenMpa ?? 18.0,
    peel_mpa: opts.peelMpa ?? 0.012,
    max_necks: opts.maxNecks ?? 50,
    bbox_min_x: bb.min.x,
    bbox_max_x: bb.max.x,
    bbox_min_y: bb.min.y,
    bbox_max_y: bb.max.y,
    bbox_min_z: bb.min.z,
    bbox_max_z: bb.max.z,
  });

  return await core.invoke<PreflightSectionsResult>('preflight_sections_native', { paramsJson });
}
