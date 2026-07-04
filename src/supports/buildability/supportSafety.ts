/**
 * Check 2 — support buildability: the fail-safe safety-factor core.
 *
 * Per-support survival estimate for the MSLA peel stroke. A cured layer sticks
 * to the FEP; as the plate lifts, the support chain (model → strut → raft) is
 * loaded and must not fail. SF < 1 predicts failure. Output is a RATIO, never a
 * boolean, so it can be colour-mapped and ranked.
 *
 *   SF = capacity / demand,  taken as the MINIMUM over failure modes.
 *
 * CRITICAL (Review Round 1, H1): tension-only is a *ceiling*, not a floor — a
 * slender or inclined strut can be strong in tension yet snap in BENDING. So
 * the core evaluates tension AND bending and returns the worse (smaller) SF.
 * Every approximation is biased pessimistic (§6 fail-safe principle): a wrong
 * answer must err toward "add more support," never a false "you're fine."
 *
 * Pure arithmetic, no I/O — unit-tested against hand-computed cases. Load
 * attribution (which support bears how much peel) is the CALLER's job and must
 * itself be conservative (worst-case, never crediting load-sharing relief — H2).
 */

/** Strut geometry at its weakest section (all lengths in mm). */
export interface StrutGeometry {
  /** True perpendicular MINIMUM cross-section diameter (rounded DOWN — §6). */
  minDiameterMm: number;
  /** Strut length = the lever arm for bending. */
  lengthMm: number;
  /** Inclination from vertical (radians). 0 = vertical (pure axial). */
  angleFromVerticalRad: number;
}

/** Peel demand on this support (from the conservative load attribution). */
export interface PeelDemand {
  /** Peel force this strut must resist, in newtons (= sigma_peel × A_peel). */
  peelForceN: number;
}

/** Calibratable material/printer constants. Conservative (low-strength) defaults. */
export interface SupportMaterial {
  /** Green strength of the cured resin, MPa (= N/mm²). */
  greenStrengthMPa: number;
  /**
   * Minimum lateral-load fraction applied even to a perfectly vertical strut,
   * accounting for the peel-front sweeping sideways (the load is never purely
   * axial in practice). Keeps the bending check honest for vertical struts
   * without flagging them infinitely. Fail-safe: higher = more pessimistic.
   */
  minLateralFraction: number;
}

export const DEFAULT_SUPPORT_MATERIAL: SupportMaterial = {
  // Conservative placeholder — real value is per-resin, calibrated from failures.
  greenStrengthMPa: 2.0,
  minLateralFraction: 0.15,
};

export type FailureMode = 'tension' | 'bending' | 'degenerate';

export interface SafetyResult {
  /** Governing (minimum) safety factor. <1 ⇒ predicted failure. */
  sf: number;
  /** Which mode governs (the smallest SF). */
  governingMode: FailureMode;
  sfTension: number;
  sfBending: number;
}

const clampNonNeg = (x: number) => (Number.isFinite(x) && x > 0 ? x : 0);

/**
 * Fail-safe support safety factor = min(tension, bending).
 *
 * Tension:  capacity = σ_green · A,  A = π/4 · d²                → SF = σ_green·A / F
 * Bending:  capacity moment = σ_green · Z,  Z = π/32 · d³         → SF = σ_green·Z / M
 *           applied moment M = F_lat · L,  F_lat = F · max(sinθ, minLateral)
 *
 * The bending term is what H1 demanded: it drops sharply for long (large L) and
 * inclined (large sinθ) struts, catching exactly the members tension misses.
 */
export function computeSupportSafetyFactor(
  g: StrutGeometry,
  demand: PeelDemand,
  mat: SupportMaterial = DEFAULT_SUPPORT_MATERIAL,
): SafetyResult {
  const d = clampNonNeg(g.minDiameterMm);
  const L = clampNonNeg(g.lengthMm);
  const F = clampNonNeg(demand.peelForceN);
  const sigma = clampNonNeg(mat.greenStrengthMPa);

  // A degenerate (zero-section) strut carries nothing → SF 0, always flagged.
  if (d <= 0 || sigma <= 0) {
    return { sf: 0, governingMode: 'degenerate', sfTension: 0, sfBending: 0 };
  }
  // No load → nothing can fail.
  if (F <= 0) {
    return { sf: Infinity, governingMode: 'tension', sfTension: Infinity, sfBending: Infinity };
  }

  const area = (Math.PI / 4) * d * d; // mm²
  const tensionCapacityN = sigma * area; // N
  const sfTension = tensionCapacityN / F;

  // Bending. Lateral load fraction: the peel is never purely axial.
  const lateralFrac = Math.max(Math.sin(Math.abs(g.angleFromVerticalRad)), clampNonNeg(mat.minLateralFraction));
  const sectionModulus = (Math.PI / 32) * d * d * d; // Z, mm³
  const bendingCapacityNmm = sigma * sectionModulus; // N·mm
  const appliedMomentNmm = F * lateralFrac * L; // N·mm
  const sfBending = appliedMomentNmm > 0 ? bendingCapacityNmm / appliedMomentNmm : Infinity;

  const sf = Math.min(sfTension, sfBending);
  const governingMode: FailureMode = sfBending < sfTension ? 'bending' : 'tension';
  return { sf, governingMode, sfTension, sfBending };
}

/** Traffic-light band for a safety factor. Warn-only: a pass recedes (§ review). */
export function safetyBand(sf: number): 'fail' | 'marginal' | 'ok' {
  if (sf < 1.0) return 'fail';
  if (sf < 2.0) return 'marginal';
  return 'ok';
}
