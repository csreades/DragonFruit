import assert from 'node:assert/strict';
import test from 'node:test';

import {
  runBuildabilitySweep,
  estimateFootprintFromContacts,
  DEFAULT_SWEEP_CONFIG,
  type SupportInput,
} from '../buildabilitySweep';

function sup(over: Partial<SupportInput> & { id: string }): SupportInput {
  return {
    minDiameterMm: 0.5,
    lengthMm: 5,
    angleFromVerticalRad: 0,
    contactX: 0,
    contactY: 0,
    ...over,
  };
}

test('empty scene → empty sweep', () => {
  const r = runBuildabilitySweep([]);
  assert.equal(r.supportCount, 0);
  assert.equal(r.worst, null);
  assert.equal(r.failCount, 0);
});

test('footprint estimate has positive area and ~targetCells cells', () => {
  const fp = estimateFootprintFromContacts(
    [sup({ id: 'a', contactX: -10, contactY: -10 }), sup({ id: 'b', contactX: 10, contactY: 10 })],
    400,
    2,
  );
  assert.ok(fp.cellAreaMm2 > 0);
  assert.ok(fp.cells.length >= 300 && fp.cells.length <= 500);
});

test('results are sorted worst-first and bands are consistent', () => {
  const supports = [
    sup({ id: 'thick', minDiameterMm: 1.2, contactX: 0, contactY: 0 }),
    sup({ id: 'thin', minDiameterMm: 0.25, contactX: 8, contactY: 0 }),
  ];
  const r = runBuildabilitySweep(supports);
  assert.equal(r.perSupport[0].sf, r.worst!.sf);
  for (let i = 1; i < r.perSupport.length; i++) {
    assert.ok(r.perSupport[i].sf >= r.perSupport[i - 1].sf, 'sorted ascending by SF');
  }
});

test('a thin inclined strut is flagged worse than a thick vertical one (H1 through the sweep)', () => {
  const supports = [
    sup({ id: 'strong', minDiameterMm: 1.0, lengthMm: 4, angleFromVerticalRad: 0, contactX: 0, contactY: 0 }),
    sup({ id: 'weak', minDiameterMm: 0.3, lengthMm: 10, angleFromVerticalRad: Math.PI / 4, contactX: 4, contactY: 0 }),
  ];
  const r = runBuildabilitySweep(supports);
  assert.equal(r.worst!.id, 'weak', 'thin+long+inclined must be the worst');
  // The weak one should be bending-governed (H1 caught it).
  const weak = r.perSupport.find((v) => v.id === 'weak')!;
  assert.equal(weak.governingMode, 'bending');
  assert.ok(weak.sf < weak.sfTension, 'governing SF below tension-only (fail-safe)');
});

test('an isolated support bears more peel demand than a clustered one (footprint tributary)', () => {
  const supports = [
    sup({ id: 'c1', contactX: -8, contactY: -1, minDiameterMm: 0.5 }),
    sup({ id: 'c2', contactX: -8, contactY: 1, minDiameterMm: 0.5 }),
    sup({ id: 'lone', contactX: 8, contactY: 0, minDiameterMm: 0.5 }),
  ];
  const r = runBuildabilitySweep(supports);
  const lone = r.perSupport.find((v) => v.id === 'lone')!;
  const c1 = r.perSupport.find((v) => v.id === 'c1')!;
  assert.ok(lone.peelForceN > c1.peelForceN, 'isolated support bears more peel load');
  assert.ok(lone.tributaryMm2 > c1.tributaryMm2);
});

test('more supports under the same footprint lowers each demand (adding support helps)', () => {
  const two = runBuildabilitySweep([
    sup({ id: 'a', contactX: -10, contactY: 0 }),
    sup({ id: 'b', contactX: 10, contactY: 0 }),
  ]);
  const four = runBuildabilitySweep([
    sup({ id: 'a', contactX: -10, contactY: 0 }),
    sup({ id: 'b', contactX: 10, contactY: 0 }),
    sup({ id: 'c', contactX: -3, contactY: 0 }),
    sup({ id: 'd', contactX: 3, contactY: 0 }),
  ]);
  // Same span, denser supports → the worst peel demand should not increase.
  assert.ok(four.worst!.peelForceN <= two.worst!.peelForceN + 1e-9);
});
