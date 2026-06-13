import * as THREE from 'three';
import { type DetectedIsland, SUPPORTED_RADIUS_MM } from './types';

/**
 * Shared, default-ON island filter for BOTH detectors (voxel + minima).
 *
 * Dependency-injected and tab-agnostic: this module never imports `src/supports/*`.
 * The caller (page-level wiring) extracts existing support-tip world positions
 * from the supports store and passes them in via `supportTips`, so the Islands
 * module is equally usable with zero supports or on a branch without a Support tab.
 */

export interface IslandFilterInput {
  /** Existing support-tip world positions for the active model (world mm, Z-up). */
  supportTips: THREE.Vector3[];
  /** Build-plate plane Z (world mm). Contacts at/below this (+eps) are "grounded". */
  plateZ: number;
  /** "Already supported" proximity radius (mm). Defaults to {@link SUPPORTED_RADIUS_MM}. */
  supportedRadiusMm?: number;
  /** Tolerance (mm) above `plateZ` still counted as plate-contact. */
  groundedEpsilonMm?: number;
}

export interface IslandFilterToggles {
  /** Reveal islands already covered by a support tip. Default false (they are hidden). */
  showAlreadySupported: boolean;
  /** Reveal islands sitting on the build-plate plane. Default false (they are hidden). */
  showPlateContact: boolean;
}

export const DEFAULT_FILTER_TOGGLES: IslandFilterToggles = {
  showAlreadySupported: false,
  showPlateContact: false,
};

/**
 * Annotate each island with `supported` / `grounded` flags. Mutates and returns
 * the same array. n(islands)·m(tips) are both small; a spatial index can be
 * added later if needed.
 */
export function annotateFilterFlags(
  islands: DetectedIsland[],
  input: IslandFilterInput,
): DetectedIsland[] {
  const radius = input.supportedRadiusMm ?? SUPPORTED_RADIUS_MM;
  const radiusSq = radius * radius;
  const groundedEps = input.groundedEpsilonMm ?? 1e-3;
  const plateCeil = input.plateZ + groundedEps;

  for (const island of islands) {
    island.grounded = island.contact.z <= plateCeil;
    island.supported = input.supportTips.some(
      (tip) => tip.distanceToSquared(island.contact) <= radiusSq,
    );
  }
  return islands;
}

/**
 * Apply the visibility toggles to produce the subset shown in 3D and in the
 * browser list. Default toggles hide both supported and plate-contact islands,
 * so ←/→ stepping skips them.
 */
export function applyFilter(
  islands: DetectedIsland[],
  toggles: IslandFilterToggles,
): DetectedIsland[] {
  return islands.filter((island) => {
    if (island.supported && !toggles.showAlreadySupported) return false;
    if (island.grounded && !toggles.showPlateContact) return false;
    return true;
  });
}
