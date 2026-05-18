import type { IslandMarker } from '@/volumeAnalysis/IslandScan/islandOverlayLogic';
import { MAX_ISLAND_MARKERS } from './softClay';
import type { IslandMarkerData } from './softClay';

/**
 * Packs IslandMarker[] into the Float32Array layout SoftClayMaterial's
 * uIslandMarkers uniform expects (xyzw = centerX, centerY, baseZ, weight).
 *
 * Skips negative-id markers (debug / seed markers from computeIslandMarkers),
 * weight<=0 markers, and any marker with non-finite weight or coordinates.
 * The NaN guard uses !(weight > 0) — direct `<= 0` lets NaN through because
 * every comparison with NaN is false. A NaN slipping into the uniform would
 * propagate to the shader as undefined GPU behaviour. Caps at MAX_ISLAND_MARKERS.
 */
export function packIslandMarkers(markers: IslandMarker[] | undefined): IslandMarkerData {
  const out = new Float32Array(MAX_ISLAND_MARKERS * 4);
  if (!markers || markers.length === 0) return { markers: out, count: 0 };
  let count = 0;
  for (const m of markers) {
    if (count >= MAX_ISLAND_MARKERS) break;
    if (m.id < 0) continue;
    if (!(m.weight > 0)) continue;
    if (!Number.isFinite(m.centerX) || !Number.isFinite(m.centerY) || !Number.isFinite(m.baseZ)) continue;
    out[count * 4 + 0] = m.centerX;
    out[count * 4 + 1] = m.centerY;
    out[count * 4 + 2] = m.baseZ;
    out[count * 4 + 3] = m.weight;
    count += 1;
  }
  return { markers: out, count };
}
