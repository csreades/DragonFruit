import assert from 'node:assert/strict';
import test from 'node:test';

import { trunkToSupportInput, trunksToSupportInputs } from '../supportGeometry';
import type { Trunk } from '@/supports/types';

// Minimal trunk factory (only the fields the adapter reads).
function makeTrunk(over: Partial<Trunk> = {}): Trunk {
  return {
    id: 't1',
    segments: [
      { id: 's1', diameter: 0.8, bottomJoint: { id: 'j0', pos: { x: 0, y: 0, z: 0 }, diameter: 0.8 }, topJoint: { id: 'j1', pos: { x: 0, y: 0, z: 5 }, diameter: 0.6 } },
      { id: 's2', diameter: 0.6, bottomJoint: { id: 'j1', pos: { x: 0, y: 0, z: 5 }, diameter: 0.6 }, topJoint: { id: 'j2', pos: { x: 3, y: 0, z: 9 }, diameter: 0.4 } },
    ],
    contactCone: { pos: { x: 3, y: 0, z: 9 }, profile: { contactDiameterMm: 0.3 } },
    ...over,
  } as unknown as Trunk;
}

test('extracts the weakest section = cone tip diameter', () => {
  const r = trunkToSupportInput(makeTrunk())!;
  assert.ok(r);
  assert.ok(Math.abs(r.minDiameterMm - 0.3) < 1e-9, `minDiameter ${r.minDiameterMm} should be the 0.3mm cone tip`);
});

test('path length = sum of segment endpoint distances', () => {
  const r = trunkToSupportInput(makeTrunk())!;
  // seg1: (0,0,0)->(0,0,5) = 5;  seg2: (0,0,5)->(3,0,9) = 5  → total 10
  assert.ok(Math.abs(r.lengthMm - 10) < 1e-9, `length ${r.lengthMm} should be 10`);
});

test('contact XY comes from the cone pos', () => {
  const r = trunkToSupportInput(makeTrunk())!;
  assert.equal(r.contactX, 3);
  assert.equal(r.contactY, 0);
});

test('inclination is the base→tip axis angle from vertical', () => {
  const r = trunkToSupportInput(makeTrunk())!;
  // base (0,0,0) → tip (3,0,9): horizontal 3, vertical 9 → atan2(3,9) ≈ 18.4°
  const deg = (r.angleFromVerticalRad * 180) / Math.PI;
  assert.ok(Math.abs(deg - Math.atan2(3, 9) * 180 / Math.PI) < 1e-6, `angle ${deg}°`);
});

test('a purely vertical trunk has ~0 inclination', () => {
  const t = makeTrunk({
    segments: [
      { id: 's1', diameter: 0.5, bottomJoint: { id: 'a', pos: { x: 1, y: 2, z: 0 }, diameter: 0.5 }, topJoint: { id: 'b', pos: { x: 1, y: 2, z: 6 }, diameter: 0.5 } },
    ] as any,
    contactCone: { pos: { x: 1, y: 2, z: 6 }, profile: { contactDiameterMm: 0.4 } } as any,
  });
  const r = trunkToSupportInput(t)!;
  assert.ok(r.angleFromVerticalRad < 1e-6);
  assert.equal(r.minDiameterMm, 0.4);
});

test('empty / segmentless trunk → null', () => {
  assert.equal(trunkToSupportInput(makeTrunk({ segments: [] })), null);
});

test('trunksToSupportInputs adapts a map and drops the unusable', () => {
  const map: Record<string, Trunk> = {
    good: makeTrunk({ id: 'good' }),
    empty: makeTrunk({ id: 'empty', segments: [] }),
  };
  const out = trunksToSupportInputs(map);
  assert.equal(out.length, 1);
  assert.equal(out[0].id, 'good');
});
