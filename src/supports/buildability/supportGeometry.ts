/**
 * Check 2 — adapter: native support primitive → SupportInput for the sweep.
 *
 * Reduces a Trunk (a chain of Segments up to a ContactCone at the model) to the
 * few quantities the fail-safe checks need:
 *   - minDiameterMm : the weakest section along the load path (rounded down —
 *                     the thinnest of the segment diameters and the cone tip).
 *   - lengthMm      : the path length base→tip = the bending lever.
 *   - angle         : inclination of the base→tip axis from vertical.
 *   - contact XY    : the ContactCone.pos (model interface) on the plate.
 *
 * v1 assumes support positions are already in plate/world coordinates.
 */
import type { Trunk, Segment, Vec3 } from '@/supports/types';
import type { SupportInput } from './buildabilitySweep';

function dist(a: Vec3, b: Vec3): number {
  const dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
  return Math.sqrt(dx * dx + dy * dy + dz * dz);
}

function segEndpoints(seg: Segment): { bottom?: Vec3; top?: Vec3 } {
  return { bottom: seg.bottomJoint?.pos, top: seg.topJoint?.pos };
}

/**
 * Convert a Trunk to a SupportInput, or null if it has no usable section
 * (no segments / degenerate). Robust to missing joints.
 */
export function trunkToSupportInput(trunk: Trunk): SupportInput | null {
  const segments = trunk.segments ?? [];
  if (segments.length === 0) return null;

  // Weakest section: thinnest segment, and the cone tip if present.
  let minD = Infinity;
  for (const seg of segments) {
    if (Number.isFinite(seg.diameter) && seg.diameter > 0) minD = Math.min(minD, seg.diameter);
  }
  const cone = trunk.contactCone;
  const tipD = cone?.profile?.contactDiameterMm;
  if (typeof tipD === 'number' && tipD > 0) minD = Math.min(minD, tipD);
  if (!Number.isFinite(minD) || minD <= 0) return null;

  // Path length = sum of segment endpoint distances; collect base/tip.
  let lengthMm = 0;
  let basePos: Vec3 | undefined;
  let topmost: Vec3 | undefined;
  for (const seg of segments) {
    const { bottom, top } = segEndpoints(seg);
    if (bottom && top) lengthMm += dist(bottom, top);
    if (bottom && !basePos) basePos = bottom;
    if (top) topmost = top;
  }
  const tipPos = cone?.pos ?? topmost;
  if (!basePos && tipPos) basePos = tipPos;
  if (!tipPos || !basePos) return null;

  // Fallback length if joints were absent.
  if (lengthMm <= 0) lengthMm = dist(basePos, tipPos);
  if (lengthMm <= 0) lengthMm = Math.max(minD, 0.5); // degenerate guard

  // Inclination of the base→tip axis from vertical (Z).
  const axis = { x: tipPos.x - basePos.x, y: tipPos.y - basePos.y, z: tipPos.z - basePos.z };
  const axisLen = Math.sqrt(axis.x * axis.x + axis.y * axis.y + axis.z * axis.z);
  const angleFromVerticalRad = axisLen > 1e-9 ? Math.acos(Math.min(1, Math.abs(axis.z) / axisLen)) : 0;

  return {
    id: trunk.id,
    minDiameterMm: minD,
    lengthMm,
    angleFromVerticalRad,
    contactX: tipPos.x,
    contactY: tipPos.y,
  };
}

/** Adapt a whole trunk map (support state) to SupportInputs. */
export function trunksToSupportInputs(trunks: Record<string, Trunk>): SupportInput[] {
  const out: SupportInput[] = [];
  for (const trunk of Object.values(trunks)) {
    const si = trunkToSupportInput(trunk);
    if (si) out.push(si);
  }
  return out;
}
