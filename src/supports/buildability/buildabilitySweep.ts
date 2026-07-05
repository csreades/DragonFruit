/**
 * Check 2 — the buildability sweep (assembly).
 *
 * Ties the two validated cores together for an on-demand, per-part support
 * survival check:
 *
 *   support primitives ──adapter──▶ SupportInput[]
 *        └▶ footprint estimate ──▶ attributePeelLoad (H2) ──▶ peelForce_i
 *        └▶ per-support geometry ─▶ computeSupportSafetyFactor (H1) ──▶ SF_i
 *
 * v1 is native + on-demand only (reactive tier and imported-mesh detection are
 * deferred per Review Round 1). It is source-agnostic in shape but here reads
 * DragonFruit's own support primitives via the adapter (supportGeometry.ts).
 *
 * The footprint (which drives peel demand) is estimated self-contained from the
 * support CONTACT points — the bounding region of the contacts, gridded — so v1
 * needs no model raster. This is a documented approximation: a support isolated
 * from its neighbours owns a larger tributary → higher demand → flagged first,
 * which is the physically right, fail-safe bias.
 */
import {
  computeSupportSafetyFactor,
  safetyBand,
  DEFAULT_SUPPORT_MATERIAL,
  type SupportMaterial,
  type FailureMode,
} from './supportSafety';
import {
  attributePeelLoad,
  DEFAULT_ATTRIBUTION,
  type AttributionParams,
  type FootprintSample,
  type SupportContact,
} from './loadAttribution';

/** One support reduced to the quantities the checks need (from the adapter). */
export interface SupportInput {
  id: string;
  /** True perpendicular MINIMUM cross-section diameter along the load path (mm). */
  minDiameterMm: number;
  /** Total path length = bending lever (mm). */
  lengthMm: number;
  /** Inclination from vertical (radians). */
  angleFromVerticalRad: number;
  /** Model-contact point projected to the plate (mm). */
  contactX: number;
  contactY: number;
}

export interface SupportVerdict {
  id: string;
  sf: number;
  band: 'fail' | 'marginal' | 'ok';
  governingMode: FailureMode;
  sfTension: number;
  sfBending: number;
  tributaryMm2: number;
  peelForceN: number;
}

export interface SweepResult {
  perSupport: SupportVerdict[];
  worst: SupportVerdict | null;
  failCount: number;
  marginalCount: number;
  supportCount: number;
}

export interface SweepConfig {
  material: SupportMaterial;
  attribution: AttributionParams;
  /** Target cell count for the footprint estimate (resolution vs cost). */
  footprintCells: number;
  /** Padding around the contact bbox, mm (edge supports need catchment). */
  footprintPadMm: number;
}

export const DEFAULT_SWEEP_CONFIG: SweepConfig = {
  material: DEFAULT_SUPPORT_MATERIAL,
  attribution: DEFAULT_ATTRIBUTION,
  footprintCells: 900,
  footprintPadMm: 2.0,
};

/**
 * Self-contained footprint estimate: the axis-aligned bounding region of the
 * support contacts (padded), sampled into a roughly `targetCells`-cell grid.
 * The tributary partition of this region approximates each support's share of
 * the peel demand without needing the model silhouette.
 */
export function estimateFootprintFromContacts(
  supports: SupportInput[],
  targetCells: number,
  padMm: number,
): FootprintSample {
  if (supports.length === 0) return { cells: [], cellAreaMm2: 0 };
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (const s of supports) {
    minX = Math.min(minX, s.contactX);
    maxX = Math.max(maxX, s.contactX);
    minY = Math.min(minY, s.contactY);
    maxY = Math.max(maxY, s.contactY);
  }
  minX -= padMm; maxX += padMm; minY -= padMm; maxY += padMm;
  const w = Math.max(1e-3, maxX - minX);
  const h = Math.max(1e-3, maxY - minY);
  // Choose a grid with ~targetCells cells preserving aspect ratio.
  const aspect = w / h;
  const ny = Math.max(1, Math.round(Math.sqrt(Math.max(1, targetCells) / Math.max(1e-6, aspect))));
  const nx = Math.max(1, Math.round((targetCells) / ny));
  const cellW = w / nx;
  const cellH = h / ny;
  const cells: { x: number; y: number }[] = [];
  for (let j = 0; j < ny; j++) {
    for (let i = 0; i < nx; i++) {
      cells.push({ x: minX + (i + 0.5) * cellW, y: minY + (j + 0.5) * cellH });
    }
  }
  return { cells, cellAreaMm2: cellW * cellH };
}

/** Run the buildability sweep over a part's supports. */
export function runBuildabilitySweep(
  supports: SupportInput[],
  config: SweepConfig = DEFAULT_SWEEP_CONFIG,
): SweepResult {
  const contacts: SupportContact[] = supports.map((s) => ({ id: s.id, x: s.contactX, y: s.contactY }));
  const footprint = estimateFootprintFromContacts(supports, config.footprintCells, config.footprintPadMm);
  const attribution = attributePeelLoad(contacts, footprint, config.attribution);

  const perSupport: SupportVerdict[] = supports.map((s) => {
    const peelForceN = attribution.peelForceN[s.id] ?? 0;
    const r = computeSupportSafetyFactor(
      { minDiameterMm: s.minDiameterMm, lengthMm: s.lengthMm, angleFromVerticalRad: s.angleFromVerticalRad },
      { peelForceN },
      config.material,
    );
    return {
      id: s.id,
      sf: r.sf,
      band: safetyBand(r.sf),
      governingMode: r.governingMode,
      sfTension: r.sfTension,
      sfBending: r.sfBending,
      tributaryMm2: attribution.tributaryMm2[s.id] ?? 0,
      peelForceN,
    };
  });

  // Worst-first for the risk list.
  perSupport.sort((a, b) => a.sf - b.sf);
  const failCount = perSupport.filter((v) => v.band === 'fail').length;
  const marginalCount = perSupport.filter((v) => v.band === 'marginal').length;
  return {
    perSupport,
    worst: perSupport.length > 0 ? perSupport[0] : null,
    failCount,
    marginalCount,
    supportCount: supports.length,
  };
}
