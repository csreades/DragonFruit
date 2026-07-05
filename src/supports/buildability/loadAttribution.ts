/**
 * Check 2 — peel-load attribution (the H2 fix).
 *
 * Distributes the part's FEP peel demand across its supports so the safety-factor
 * core (supportSafety.ts) can be evaluated per support. The true reaction is
 * statically indeterminate (a truss with the film as a distributed load); we do
 * NOT pretend to solve it. We use an explicit nearest-support tributary with a
 * NAMED error mode, biased pessimistic.
 *
 * Review Round 1, H2 — the previous design's fatal flaw: a plain nearest-support
 * split *credits load-sharing relief* — a region between two supports is halved,
 * so the strut that actually concentrates the load is under-counted and reads
 * green. That is the forbidden false "you're fine."
 *
 * The fix here is the `concentrationFactor` (≥ 1): each support's attributed
 * demand is its geometric tributary SCALED UP, modelling the reality that load
 * concentrates on stiffer/nearer members and that a neighbour may fail and dump
 * its share. The factor is the calibratable fail-safe knob — larger = more
 * pessimistic. At 1.0 it degenerates to the (unsafe) even split, so the default
 * is well above 1.
 *
 * Pure geometry over a sampled footprint — unit-tested. Producing the footprint
 * sample (projecting the model's FEP-contact silhouette) is the caller's job.
 */

/** A support's contact point projected to the plate (mm). */
export interface SupportContact {
  id: string;
  x: number;
  y: number;
}

/**
 * The model's FEP-contact footprint, discretized into equal-area cells.
 * Typically the projected silhouette of the worst (largest) contact layer.
 */
export interface FootprintSample {
  cells: { x: number; y: number }[];
  cellAreaMm2: number;
}

export interface AttributionParams {
  /** Effective peel/separation stress, MPa (= N/mm²). Conservative default. */
  sigmaPeelMPa: number;
  /**
   * ≥ 1 pessimism multiplier on each support's tributary (the H2 fail-safe
   * knob). Models load concentration + neighbour-failure inheritance so the
   * check never credits perfect load-sharing. 1.0 = unsafe even split.
   */
  concentrationFactor: number;
}

export const DEFAULT_ATTRIBUTION: AttributionParams = {
  sigmaPeelMPa: 0.3, // conservative placeholder; per-film, calibrated from failures
  concentrationFactor: 2.0,
};

export interface AttributionResult {
  /** Peel force each support must resist, newtons, keyed by support id. */
  peelForceN: Record<string, number>;
  /** Geometric tributary area (pre-scaling) per support, mm². */
  tributaryMm2: Record<string, number>;
}

/**
 * Assign each footprint cell to its nearest support (a discrete Voronoi
 * partition), sum the tributary area per support, then convert to a peel force
 * biased pessimistic by `concentrationFactor`.
 *
 *   peelForce_i = sigma_peel · tributary_i · concentrationFactor
 */
export function attributePeelLoad(
  supports: SupportContact[],
  footprint: FootprintSample,
  params: AttributionParams = DEFAULT_ATTRIBUTION,
): AttributionResult {
  const tributaryMm2: Record<string, number> = {};
  const peelForceN: Record<string, number> = {};
  for (const s of supports) {
    tributaryMm2[s.id] = 0;
  }
  if (supports.length === 0) {
    return { tributaryMm2, peelForceN };
  }

  const cellArea = Math.max(0, footprint.cellAreaMm2);
  for (const cell of footprint.cells) {
    let bestId = supports[0].id;
    let bestD2 = Infinity;
    for (const s of supports) {
      const dx = cell.x - s.x;
      const dy = cell.y - s.y;
      const d2 = dx * dx + dy * dy;
      if (d2 < bestD2) {
        bestD2 = d2;
        bestId = s.id;
      }
    }
    tributaryMm2[bestId] += cellArea;
  }

  const k = Math.max(1, params.concentrationFactor);
  const sigma = Math.max(0, params.sigmaPeelMPa);
  for (const s of supports) {
    peelForceN[s.id] = sigma * tributaryMm2[s.id] * k;
  }
  return { tributaryMm2, peelForceN };
}
