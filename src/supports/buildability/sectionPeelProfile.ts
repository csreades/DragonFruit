/**
 * Check 2 (geometry mode) — cross-section peel analysis.
 *
 * Peel failure is a GEOMETRY phenomenon, not a support phenomenon: whenever a
 * thin cross-section carries a large peeling mass above it, it can snap — a
 * baked-in support strut, a thin wall, or a delicate neck on the part itself.
 * So the general check analyses the sliced geometry directly and works
 * uniformly for parts, imported pre-supported meshes, and native supports.
 *
 * For one vertical connected mass we have its cross-sectional area at each
 * layer, bottom→top. A "neck" is a local thin section; the load it must carry
 * is the worst single peeling layer at or above it (peel separates one layer at
 * a time, so the largest such layer is the worst instantaneous force):
 *
 *   SF(i) = (sigma_green · A[i]) / (sigma_peel · maxArea(j>=i))
 *
 * The mass's safety factor is the MINIMUM over its layers — the weakest neck.
 *
 * This reproduces the intuition exactly: an UPRIGHT pyramid (area decreasing
 * upward) has SF = green/peel everywhere (safe — each layer carries only what
 * is above it), while an INVERTED pyramid (area increasing upward) collapses to
 * SF≈0 at its point, which bears the entire part above it. No supports needed to
 * see it. Pure arithmetic over an area profile — unit-tested.
 */

export interface SectionMaterial {
  /** Green strength of the cured resin, MPa (= N/mm²). Conservative (low). */
  greenStrengthMPa: number;
  /** Effective peel/separation stress, MPa. Conservative (high). */
  sigmaPeelMPa: number;
}

// Shares Check 2's calibrated defaults (see supportSafety / loadAttribution).
export const DEFAULT_SECTION_MATERIAL: SectionMaterial = {
  greenStrengthMPa: 18.0,
  sigmaPeelMPa: 0.012,
};

export interface NeckResult {
  /** Layer index of the governing (weakest) neck within the mass. */
  layerIndex: number;
  sf: number;
  areaMm2: number;
  peelAreaAboveMm2: number;
}

export interface ProfileVerdict {
  worst: NeckResult;
  /** Per-layer safety factor (min over modes is just tension here). */
  sfByLayer: number[];
}

/**
 * Analyse one connected mass's cross-section area profile (bottom→top, mm²).
 * Returns the weakest neck and the per-layer SF. An empty/zero profile yields
 * SF 0 (degenerate → always flagged).
 */
export function analyzeSectionProfile(
  areaByLayerMm2: number[],
  mat: SectionMaterial = DEFAULT_SECTION_MATERIAL,
): ProfileVerdict {
  const n = areaByLayerMm2.length;
  const green = Math.max(0, mat.greenStrengthMPa);
  const peel = Math.max(1e-12, mat.sigmaPeelMPa);

  if (n === 0) {
    return { worst: { layerIndex: 0, sf: 0, areaMm2: 0, peelAreaAboveMm2: 0 }, sfByLayer: [] };
  }

  // Suffix-max of area (largest peeling layer at or above each index).
  const maxAbove = new Array<number>(n);
  let running = 0;
  for (let i = n - 1; i >= 0; i--) {
    running = Math.max(running, Math.max(0, areaByLayerMm2[i]));
    maxAbove[i] = running;
  }

  const sfByLayer = new Array<number>(n);
  let worst: NeckResult = { layerIndex: 0, sf: Infinity, areaMm2: 0, peelAreaAboveMm2: 0 };
  for (let i = 0; i < n; i++) {
    const a = Math.max(0, areaByLayerMm2[i]);
    const demand = peel * maxAbove[i];
    const sf = a <= 0 ? 0 : demand <= 0 ? Infinity : (green * a) / demand;
    sfByLayer[i] = sf;
    if (sf < worst.sf) {
      worst = { layerIndex: i, sf, areaMm2: a, peelAreaAboveMm2: maxAbove[i] };
    }
  }
  return { worst, sfByLayer };
}

export function sectionBand(sf: number): 'fail' | 'marginal' | 'ok' {
  if (sf < 1.0) return 'fail';
  if (sf < 2.0) return 'marginal';
  return 'ok';
}
