/** Snap angle to nearest increment using Math.round quantization. */
export function snapAngle(angle: number, increment: number): number {
  return Math.round(angle / increment) * increment;
}

/** Coarse snap increment: 45 degrees */
export const SNAP_COARSE = Math.PI / 4;

/** Fine snap increment: 15 degrees */
export const SNAP_FINE = Math.PI / 12;

/** localStorage key for persistent snap toggle */
export const SNAP_STORAGE_KEY = 'dragonfruit:rotation-snap-enabled';

/** Minor (visual-only) tick increment: 5 degrees. Major/medium reuse SNAP_COARSE/SNAP_FINE. */
export const TICK_MINOR = Math.PI / 36;

/** Visual size class of a tick mark. */
export type TickTier = 'major' | 'medium' | 'minor';

/** A single tick mark around the rotation ring. */
export interface SnapTick {
  /** Angle around the ring in radians, normalized to [0, 2*PI). */
  angleRad: number;
  /** Which tier this position belongs to (its highest applicable tier). */
  tier: TickTier;
  /** Tick length as a fraction of the full (major) tick length. */
  lengthMult: number;
}

/** Tick interval per tier, in whole degrees. */
export interface SnapTickConfig {
  /** Major (coarse-snap) interval, e.g. 45. */
  majorDeg: number;
  /** Medium (fine-snap) interval, e.g. 15. */
  mediumDeg: number;
  /** Minor (visual-only) interval, e.g. 5. */
  minorDeg: number;
}

/** Tick length as a fraction of the major tick length, by tier. */
export const TICK_LENGTH_MULT: Record<TickTier, number> = {
  major: 1.0,
  medium: 0.6,
  minor: 0.3,
};

/** Default tiers: 45 / 15 / 5 degrees (the common slicer default). */
export const DEFAULT_SNAP_TICK_CONFIG: SnapTickConfig = {
  majorDeg: 45,
  mediumDeg: 15,
  minorDeg: 5,
};

const DEG_TO_RAD = Math.PI / 180;

/**
 * Build the de-duplicated set of tick marks around the full 360-degree ring for
 * the given tier config. Every angular position (a multiple of `minorDeg`) is
 * classified to its highest applicable tier: a multiple of `majorDeg` is
 * `major` and is never also emitted as `medium`/`minor`. Classification is done
 * in integer degrees so overlapping tiers (e.g. 45 is a multiple of both 45 and
 * 15) can never be misclassified by floating-point modulo.
 *
 * Tiers do not have to nest (e.g. a Custom 45/10/5 config is accepted); when
 * they do not, ticks of different tiers simply will not coincide.
 */
export function getSnapTicks(config: SnapTickConfig = DEFAULT_SNAP_TICK_CONFIG): SnapTick[] {
  const { majorDeg, mediumDeg, minorDeg } = config;
  const steps = Math.round(360 / minorDeg);
  const ticks: SnapTick[] = [];
  for (let i = 0; i < steps; i++) {
    const deg = i * minorDeg;
    const tier: TickTier =
      deg % majorDeg === 0 ? 'major' : deg % mediumDeg === 0 ? 'medium' : 'minor';
    ticks.push({ angleRad: deg * DEG_TO_RAD, tier, lengthMult: TICK_LENGTH_MULT[tier] });
  }
  return ticks;
}

/**
 * Build flat line-segment endpoints for all ticks of one tier, as [x, y, z]
 * triples in the ring's local XY plane (z = 0). Each tick yields two points:
 * an outer point on the ring radius and an inner point shortened toward centre
 * by `baseLength * lengthMult`. Points come in outer/inner pairs so the result
 * can be fed to a drei <Line segments> as disconnected radial ticks. Pure and
 * deterministic — unit-tested independently of the R3F render.
 */
export function buildTierSegmentPoints(
  ticks: SnapTick[],
  radius: number,
  baseLength: number,
  tier: TickTier,
): [number, number, number][] {
  const points: [number, number, number][] = [];
  for (const tick of ticks) {
    if (tick.tier !== tier) continue;
    const innerRadius = radius - baseLength * tick.lengthMult;
    const cos = Math.cos(tick.angleRad);
    const sin = Math.sin(tick.angleRad);
    points.push([cos * radius, sin * radius, 0]);
    points.push([cos * innerRadius, sin * innerRadius, 0]);
  }
  return points;
}
