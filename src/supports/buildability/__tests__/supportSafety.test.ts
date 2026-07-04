import assert from 'node:assert/strict';
import test from 'node:test';

import {
  computeSupportSafetyFactor,
  safetyBand,
  DEFAULT_SUPPORT_MATERIAL,
  type StrutGeometry,
  type SupportMaterial,
} from '../supportSafety';

// A material with NO lateral floor, so vertical struts are pure tension — lets
// tests isolate each mode cleanly.
const MAT_NO_FLOOR: SupportMaterial = { greenStrengthMPa: 2.0, minLateralFraction: 0 };

const vertical = (d: number, L: number): StrutGeometry => ({
  minDiameterMm: d,
  lengthMm: L,
  angleFromVerticalRad: 0,
});

test('degenerate strut (zero diameter) → SF 0, always flagged', () => {
  const r = computeSupportSafetyFactor(vertical(0, 5), { peelForceN: 1 }, MAT_NO_FLOOR);
  assert.equal(r.sf, 0);
  assert.equal(r.governingMode, 'degenerate');
  assert.equal(safetyBand(r.sf), 'fail');
});

test('no load → SF infinite', () => {
  const r = computeSupportSafetyFactor(vertical(0.5, 5), { peelForceN: 0 }, MAT_NO_FLOOR);
  assert.equal(r.sf, Infinity);
});

test('vertical strut is governed by tension; matches σ·A/F exactly', () => {
  // d=0.5mm → A = π/4·0.25 = 0.19635 mm²; σ=2 MPa → cap = 0.3927 N.
  const r = computeSupportSafetyFactor(vertical(0.5, 5), { peelForceN: 0.2 }, MAT_NO_FLOOR);
  const expectedTension = (2.0 * (Math.PI / 4) * 0.5 * 0.5) / 0.2;
  assert.equal(r.governingMode, 'tension');
  assert.ok(Math.abs(r.sfTension - expectedTension) < 1e-9, `${r.sfTension} vs ${expectedTension}`);
  assert.equal(r.sf, r.sfTension);
});

test('H1 FIX: an inclined strut is governed by BENDING and reads far weaker than tension-only', () => {
  // Same diameter/load as the vertical case, but tilted 45°. Tension-only would
  // report the SAME safe number; the bending term must pull SF down sharply —
  // this is the exact false-green the review flagged.
  const tilted: StrutGeometry = { minDiameterMm: 0.5, lengthMm: 5, angleFromVerticalRad: Math.PI / 4 };
  const r = computeSupportSafetyFactor(tilted, { peelForceN: 0.2 }, MAT_NO_FLOOR);
  assert.equal(r.governingMode, 'bending', 'inclined slender strut must fail in bending');
  assert.ok(r.sf < r.sfTension, 'governing SF must be BELOW the tension-only SF (fail-safe)');
  // Bending must be dramatically lower here (long lever): sanity that it is < half.
  assert.ok(r.sf < r.sfTension * 0.5, `bending SF ${r.sf} should be far below tension ${r.sfTension}`);
});

test('bending SF scales down with length (longer lever = weaker)', () => {
  const tilt = Math.PI / 6;
  const short = computeSupportSafetyFactor({ minDiameterMm: 0.4, lengthMm: 2, angleFromVerticalRad: tilt }, { peelForceN: 0.15 }, MAT_NO_FLOOR);
  const long = computeSupportSafetyFactor({ minDiameterMm: 0.4, lengthMm: 8, angleFromVerticalRad: tilt }, { peelForceN: 0.15 }, MAT_NO_FLOOR);
  assert.ok(long.sfBending < short.sfBending, 'longer strut must have lower bending SF');
  // 4× length → 4× moment → 1/4 the bending SF.
  assert.ok(Math.abs(long.sfBending - short.sfBending / 4) < 1e-9);
});

test('thicker strut helps bending more than tension (d³ vs d²)', () => {
  const tilt = Math.PI / 4;
  const thin = computeSupportSafetyFactor({ minDiameterMm: 0.3, lengthMm: 5, angleFromVerticalRad: tilt }, { peelForceN: 0.1 }, MAT_NO_FLOOR);
  const thick = computeSupportSafetyFactor({ minDiameterMm: 0.6, lengthMm: 5, angleFromVerticalRad: tilt }, { peelForceN: 0.1 }, MAT_NO_FLOOR);
  const tensionRatio = thick.sfTension / thin.sfTension; // ~ (0.6/0.3)² = 4
  const bendingRatio = thick.sfBending / thin.sfBending; // ~ (0.6/0.3)³ = 8
  assert.ok(Math.abs(tensionRatio - 4) < 1e-6);
  assert.ok(Math.abs(bendingRatio - 8) < 1e-6);
});

test('minLateralFraction makes even a vertical strut show finite bending (fail-safe floor)', () => {
  // With the default lateral floor, a vertical strut is NOT infinitely strong in
  // bending — the peel front sweeps sideways.
  const r = computeSupportSafetyFactor(vertical(0.3, 12), { peelForceN: 0.5 }, DEFAULT_SUPPORT_MATERIAL);
  assert.ok(Number.isFinite(r.sfBending), 'vertical strut must still have a finite bending SF');
  assert.ok(r.sf <= r.sfTension, 'governing SF never exceeds tension-only (fail-safe)');
});

test('safetyBand thresholds', () => {
  assert.equal(safetyBand(0.5), 'fail');
  assert.equal(safetyBand(1.5), 'marginal');
  assert.equal(safetyBand(3.0), 'ok');
});
