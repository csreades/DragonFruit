import * as THREE from 'three';
import { type DetectedIsland } from './types';
// PORTABILITY: ALL analysis-domain dependencies are confined to this file —
// the analysis slicer + the IslandScan per-layer primitives. If that infra is
// removed (e.g. Analysis tab retired), this is the one Islands/ module to
// re-home or vendor; every other Islands/ file is independent of it.
import { BucketedSlicer } from '@/components/analysis/Slice2D';
import { scanLayer } from '@/volumeAnalysis/IslandScan/island';
import { rleEncode } from '@/volumeAnalysis/IslandScan/rle';
import { rasterizeLoopsToExistingGrid } from '@/volumeAnalysis/IslandScan/raster';
import type { Mask } from '@/volumeAnalysis/IslandScan/types';

// Pixel-centre offsets — mirror ScanOrchestrator's VOXEL_OFFSET_{X,Y} so contact
// points land in the same world frame as the legacy overlay. Hardcoded (rather
// than imported) to avoid pulling the whole ScanOrchestrator pipeline for two
// constants.
const VOXEL_OFFSET_X = 0.5;
const VOXEL_OFFSET_Y = 0;

export interface VoxelDetectParams {
  pxMm: number;
  supportBufferMm: number;
  /** Per-layer 2D candidate connectivity (passed to `scanLayer`). */
  connectivity?: 4 | 8;
  /** 3D cluster connectivity across layers: true = 26-conn (default), false = 6-conn. */
  diagonal3D?: boolean;
}

export interface VoxelDetectInput {
  /** World-space (build-plate Z-up) non-indexed positions, 9 floats per triangle. */
  positions: Float32Array;
  /** World-space bbox of the same positions. */
  bbox: THREE.Box3;
}

/**
 * Rebuilt voxel island detection.
 *
 * THE FIX: the legacy pipeline ran connected-components on the whole solid mask
 * and propagated island IDs up through every solid pixel above a seed (and seeded
 * the grounded body at layer 0), so every island column reached a top surface.
 * Here an island is a 3D-connected cluster of the *unsupported* `scanLayer`
 * candidates only (`current − dilate(prev_below)`). Candidates never include the
 * supported bulk, so a cluster terminates as soon as the region becomes supported
 * — it can never climb to a top surface.
 *
 * Cross-layer linking is voxel adjacency (26-conn by default, robust to slanted
 * overhangs). Overlap-based linking is the refinement if overhangs fragment/merge.
 *
 * Runs on the main thread for PoC simplicity (reusing the same slicer + scanLayer
 * the worker uses), yielding periodically for progress. Worker-ization is a
 * straightforward perf follow-up for very large models.
 */
export async function detectVoxelIslands(
  input: VoxelDetectInput,
  layerHeightMm: number,
  params: VoxelDetectParams,
  onProgress?: (done: number, total: number) => void,
): Promise<DetectedIsland[]> {
  const px = params.pxMm;
  const bb = input.bbox;
  const minZ = bb.min.z;

  // Grid (mirrors ScanOrchestrator): mask Y stores -worldY.
  const originX = bb.min.x;
  const originZ = -bb.max.y; // mask row 0
  const width = Math.max(1, Math.ceil((bb.max.x - bb.min.x) / px));
  const height = Math.max(1, Math.ceil((bb.max.y - bb.min.y) / px));
  const numLayers = Math.max(0, Math.ceil((bb.max.z - minZ) / layerHeightMm));
  if (numLayers === 0) return [];

  const codec = gridCodec(width, height);
  const slicer = new BucketedSlicer(input.positions, 5.0);
  const opts = {
    px_mm: px,
    support_buffer_mm: params.supportBufferMm,
    connectivity: params.connectivity ?? 4,
  };
  const gridRef: Mask = {
    data: new Uint8Array(width * height),
    width,
    height,
    originX,
    originZ,
    px_mm: px,
  };

  // Collect the unsupported candidate voxels across all layers.
  const candidates = new Set<number>();

  for (let L = 0; L < numLayers; L++) {
    const zTop = minZ + (L + 1) * layerHeightMm + 1e-6;
    const zBot = zTop - layerHeightMm;

    const loopsNow = slicer.slice(zTop).map((loop) => loop.map((v) => ({ x: v.x, y: v.y })));
    const loopsPrev = slicer.slice(zBot).map((loop) => loop.map((v) => ({ x: v.x, y: v.y })));

    const currentRle = rleEncode(rasterizeLoopsToExistingGrid(loopsNow, gridRef).data, width, height);
    const prevRle =
      loopsPrev.length > 0
        ? rleEncode(rasterizeLoopsToExistingGrid(loopsPrev, gridRef).data, width, height)
        : null;

    const { labels } = scanLayer(currentRle, prevRle, opts);

    // Mark candidate pixels (label id > 0 == unsupported).
    for (let y = 0; y < labels.height; y++) {
      const row = labels.rows[y];
      for (let i = 0; i < row.length; i += 3) {
        const start = row[i];
        const len = row[i + 1];
        const id = row[i + 2];
        if (id > 0) {
          for (let c = 0; c < len; c++) candidates.add(codec.pack(start + c, y, L));
        }
      }
    }

    onProgress?.(L + 1, numLayers);
    if ((L & 15) === 0) await Promise.resolve(); // keep the UI breathing
  }

  return buildIslands(candidates, codec, { originX, originZ, px, minZ, layerHeightMm }, params.diagonal3D !== false);
}

interface GridGeom {
  originX: number;
  originZ: number;
  px: number;
  minZ: number;
  layerHeightMm: number;
}

/** 3D connected components over the candidate voxel set → contact-region islands. */
function buildIslands(
  candidates: Set<number>,
  codec: GridCodec,
  geom: GridGeom,
  diagonal: boolean,
): DetectedIsland[] {
  const offsets = neighbourOffsets(diagonal);
  const visited = new Set<number>();
  const islands: DetectedIsland[] = [];
  let idx = 0;

  for (const startKey of candidates) {
    if (visited.has(startKey)) continue;

    // Flood this component.
    const comp: number[] = [];
    const stack = [startKey];
    visited.add(startKey);
    while (stack.length) {
      const k = stack.pop()!;
      comp.push(k);
      const { col, row, layer } = codec.unpack(k);
      for (const [dc, dr, dl] of offsets) {
        const nc = col + dc;
        const nr = row + dr;
        const nl = layer + dl;
        if (nc < 0 || nc >= codec.width || nr < 0 || nr >= codec.height || nl < 0) continue;
        const nk = codec.pack(nc, nr, nl);
        if (candidates.has(nk) && !visited.has(nk)) {
          visited.add(nk);
          stack.push(nk);
        }
      }
    }

    // Lowest-layer footprint defines the contact point (where support attaches).
    let minLayer = Infinity;
    let maxLayer = -Infinity;
    for (const k of comp) {
      const { layer } = codec.unpack(k);
      if (layer < minLayer) minLayer = layer;
      if (layer > maxLayer) maxLayer = layer;
    }

    let sumX = 0;
    let sumY = 0;
    let baseCount = 0;
    for (const k of comp) {
      const { col, row, layer } = codec.unpack(k);
      if (layer !== minLayer) continue;
      sumX += geom.originX + col * geom.px + geom.px * VOXEL_OFFSET_X;
      sumY += -(geom.originZ + row * geom.px - geom.px * VOXEL_OFFSET_Y);
      baseCount++;
    }

    const contactX = sumX / baseCount;
    const contactY = sumY / baseCount;
    const baseZ = geom.minZ + minLayer * geom.layerHeightMm;

    islands.push({
      id: `v${idx++}`,
      source: 'voxel',
      contact: new THREE.Vector3(contactX, contactY, baseZ),
      baseZ,
      areaMm2: baseCount * geom.px * geom.px,
      layerSpan: [minLayer, maxLayer],
    });
  }

  return islands;
}

interface GridCodec {
  width: number;
  height: number;
  pack: (col: number, row: number, layer: number) => number;
  unpack: (key: number) => { col: number; row: number; layer: number };
}

function gridCodec(width: number, height: number): GridCodec {
  return {
    width,
    height,
    pack: (col, row, layer) => (layer * height + row) * width + col,
    unpack: (key) => {
      const col = key % width;
      const rest = (key - col) / width;
      const row = rest % height;
      const layer = (rest - row) / height;
      return { col, row, layer };
    },
  };
}

/** 6- or 26-connectivity neighbour offsets (excluding the origin). */
function neighbourOffsets(diagonal: boolean): Array<readonly [number, number, number]> {
  if (!diagonal) {
    return [
      [1, 0, 0], [-1, 0, 0],
      [0, 1, 0], [0, -1, 0],
      [0, 0, 1], [0, 0, -1],
    ];
  }
  const out: Array<readonly [number, number, number]> = [];
  for (let dc = -1; dc <= 1; dc++) {
    for (let dr = -1; dr <= 1; dr++) {
      for (let dl = -1; dl <= 1; dl++) {
        if (dc === 0 && dr === 0 && dl === 0) continue;
        out.push([dc, dr, dl]);
      }
    }
  }
  return out;
}
