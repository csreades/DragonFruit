import assert from 'node:assert/strict';
import test from 'node:test';

import { analyzeSectionProfile, sectionBand, DEFAULT_SECTION_MATERIAL } from '../sectionPeelProfile';

const M = { greenStrengthMPa: 18, sigmaPeelMPa: 0.012 }; // ratio green/peel = 1500

test('uniform column: SF = green/peel at every layer', () => {
  const r = analyzeSectionProfile([100, 100, 100, 100], M);
  for (const sf of r.sfByLayer) assert.ok(Math.abs(sf - 1500) < 1e-6);
  assert.ok(Math.abs(r.worst.sf - 1500) < 1e-6);
});

test('UPRIGHT pyramid (area decreasing upward) is safe everywhere', () => {
  // Each layer carries only what is above it (which is smaller) → SF = green/peel.
  const r = analyzeSectionProfile([100, 64, 36, 16, 4], M);
  for (const sf of r.sfByLayer) assert.ok(Math.abs(sf - 1500) < 1e-6, `sf ${sf}`);
  assert.equal(sectionBand(r.worst.sf), 'ok');
});

test('INVERTED pyramid (area increasing upward) fails at the point', () => {
  // The tiny base bears the whole mass above it → SF collapses at layer 0.
  const r = analyzeSectionProfile([0.5, 50, 200, 450, 800], M);
  assert.equal(r.worst.layerIndex, 0, 'the point (base) is the weakest');
  // SF0 = 1500 * 0.5 / 800 ≈ 0.94 → predicted fail.
  assert.ok(r.worst.sf < 1.0, `worst SF ${r.worst.sf} should predict failure`);
  assert.equal(sectionBand(r.worst.sf), 'fail');
});

test('blob on a thin stalk: the stalk is the governing neck', () => {
  const r = analyzeSectionProfile([80, 80, 0.6, 80, 80], M);
  assert.equal(r.worst.layerIndex, 2, 'the thin layer is the neck');
  // SF = 1500 * 0.6 / 80 = 11.25 (peels are only 80 here) → marginal-ish, but the
  // point is it is the MIN and correctly localised.
  assert.ok(r.worst.sf < r.sfByLayer[0], 'neck SF is below the thick base');
});

test('peel demand uses the MAX single layer above (suffix-max), not the sum', () => {
  const r = analyzeSectionProfile([10, 100, 10], M);
  // layer 0: maxAbove = 100 → SF = 1500*10/100 = 150
  // layer 1: maxAbove = 100 → SF = 1500
  // layer 2: maxAbove = 10  → SF = 1500
  assert.ok(Math.abs(r.sfByLayer[0] - 150) < 1e-6);
  assert.ok(Math.abs(r.sfByLayer[1] - 1500) < 1e-6);
  assert.equal(r.worst.layerIndex, 0);
});

test('degenerate: empty or zero-area profile → SF 0 (flagged)', () => {
  assert.equal(analyzeSectionProfile([], M).worst.sf, 0);
  const z = analyzeSectionProfile([0, 0], M);
  assert.equal(z.worst.sf, 0);
  assert.equal(sectionBand(z.worst.sf), 'fail');
});

test('default material is the shared calibrated pair', () => {
  assert.equal(DEFAULT_SECTION_MATERIAL.greenStrengthMPa, 18);
  assert.equal(DEFAULT_SECTION_MATERIAL.sigmaPeelMPa, 0.012);
});
