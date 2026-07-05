import assert from 'node:assert/strict';
import test from 'node:test';

import {
  attributePeelLoad,
  type FootprintSample,
  type SupportContact,
  type AttributionParams,
} from '../loadAttribution';

// Build a rectangular footprint of unit cells spanning [x0,x1]×[y0,y1].
function grid(x0: number, x1: number, y0: number, y1: number, cellAreaMm2 = 1): FootprintSample {
  const cells: { x: number; y: number }[] = [];
  for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) cells.push({ x, y });
  return { cells, cellAreaMm2 };
}

const P: AttributionParams = { sigmaPeelMPa: 0.3, concentrationFactor: 2.0 };

test('empty supports → empty result (no crash)', () => {
  const r = attributePeelLoad([], grid(0, 4, 0, 4), P);
  assert.deepEqual(r.peelForceN, {});
});

test('single support bears the whole footprint', () => {
  const fp = grid(-5, 5, -5, 5); // 11×11 = 121 cells
  const r = attributePeelLoad([{ id: 'a', x: 0, y: 0 }], fp, P);
  assert.equal(r.tributaryMm2['a'], 121);
  assert.equal(r.peelForceN['a'], 0.3 * 121 * 2.0);
});

test('two symmetric supports split the footprint by nearest (Voronoi)', () => {
  // Columns x=-5..5; the midline x=0 splits evenly. Even columns count is 11
  // (x=-5..5); left support gets x<0, right gets x>0, x=0 goes to first (a).
  const fp = grid(-5, 5, 0, 2); // 11 cols × 3 rows
  const supports: SupportContact[] = [
    { id: 'a', x: -5, y: 1 },
    { id: 'b', x: 5, y: 1 },
  ];
  const r = attributePeelLoad(supports, fp, P);
  // total conserved
  assert.equal(r.tributaryMm2['a'] + r.tributaryMm2['b'], fp.cells.length);
  // roughly balanced (x=0 tie-breaks to 'a', so a slightly larger)
  assert.ok(Math.abs(r.tributaryMm2['a'] - r.tributaryMm2['b']) <= 3);
});

test('tributary is conserved across all supports', () => {
  const fp = grid(-8, 8, -8, 8);
  const supports: SupportContact[] = [
    { id: 'a', x: -6, y: -6 },
    { id: 'b', x: 6, y: -6 },
    { id: 'c', x: 0, y: 6 },
  ];
  const r = attributePeelLoad(supports, fp, P);
  const total = Object.values(r.tributaryMm2).reduce((s, v) => s + v, 0);
  assert.equal(total, fp.cells.length);
});

test('a lone support in a sparse region bears MORE than one in a cluster', () => {
  // Two supports clustered on the left share the left half; one alone on the
  // right owns the whole right half → the lone one has the largest tributary.
  const fp = grid(-10, 10, -3, 3);
  const supports: SupportContact[] = [
    { id: 'l1', x: -8, y: -1 },
    { id: 'l2', x: -8, y: 1 },
    { id: 'r', x: 8, y: 0 },
  ];
  const r = attributePeelLoad(supports, fp, P);
  assert.ok(r.tributaryMm2['r'] > r.tributaryMm2['l1'], 'lone support bears more');
  assert.ok(r.tributaryMm2['r'] > r.tributaryMm2['l2']);
});

test('H2: concentrationFactor scales demand UP (never credits sharing relief)', () => {
  const fp = grid(-5, 5, 0, 2);
  const supports: SupportContact[] = [{ id: 'a', x: 0, y: 1 }];
  const even = attributePeelLoad(supports, fp, { sigmaPeelMPa: 0.3, concentrationFactor: 1.0 });
  const safe = attributePeelLoad(supports, fp, { sigmaPeelMPa: 0.3, concentrationFactor: 2.5 });
  assert.ok(safe.peelForceN['a'] > even.peelForceN['a'], 'higher factor = more pessimistic');
  assert.equal(safe.peelForceN['a'], even.peelForceN['a'] * 2.5);
});

test('concentrationFactor is floored at 1 (never below the even split)', () => {
  const fp = grid(0, 3, 0, 3);
  const supports: SupportContact[] = [{ id: 'a', x: 1, y: 1 }];
  const r = attributePeelLoad(supports, fp, { sigmaPeelMPa: 0.3, concentrationFactor: 0.2 });
  // clamped to 1.0
  assert.equal(r.peelForceN['a'], 0.3 * r.tributaryMm2['a'] * 1.0);
});
