import assert from 'node:assert/strict';
import test from 'node:test';

import { nodeSections, analyzeSectionGraph, type SectionNode } from '../sectionGraph';

const M = { greenStrengthMPa: 18, sigmaPeelMPa: 0.012 }; // green/peel = 1500

test('nodeSections: min/max per node', () => {
  const s = nodeSections([{ id: 'a', areaByLayerMm2: [10, 2, 40, 5], baseLayer: 0, childIds: [] }]);
  assert.equal(s[0].minSectionMm2, 2);
  assert.equal(s[0].maxSectionMm2, 40);
});

test('single node matches the linear profile core (inverted pyramid fails at point)', () => {
  const g = analyzeSectionGraph([{ id: 'p', areaByLayerMm2: [0.5, 50, 200, 450, 800], baseLayer: 0, childIds: [] }], M);
  assert.equal(g.worst!.id, 'p');
  assert.equal(g.worst!.worstLayer, 0);
  assert.ok(g.worst!.sf < 1, 'inverted point fails');
});

test('BRANCHING: a thin base under a big blob child is flagged by the child mass', () => {
  // base node: uniform thin 1mm² column, layers 0..2.
  // child (above): a big 900mm² blob, layers 3..4.
  const nodes: SectionNode[] = [
    { id: 'base', areaByLayerMm2: [1, 1, 1], baseLayer: 0, childIds: ['blob'] },
    { id: 'blob', areaByLayerMm2: [900, 900], baseLayer: 3, childIds: [] },
  ];
  const g = analyzeSectionGraph(nodes, M);
  const base = g.perNode.find((v) => v.id === 'base')!;
  // base neck sees peel of the 900 blob above → SF = 1500*1/900 ≈ 1.67 (marginal),
  // FAR below what it would read in isolation (1500).
  assert.ok(base.peelAreaAboveMm2 >= 900, 'base must inherit the blob mass above');
  assert.ok(base.sf < 2, `base SF ${base.sf} should be pulled down by the blob`);
  assert.ok(base.sf < g.perNode.find((v) => v.id === 'blob')!.sf, 'base neck weaker than the blob itself');
});

test('a base feeding TWO children inherits the LARGER branch', () => {
  const nodes: SectionNode[] = [
    { id: 'base', areaByLayerMm2: [2, 2], baseLayer: 0, childIds: ['small', 'big'] },
    { id: 'small', areaByLayerMm2: [30], baseLayer: 2, childIds: [] },
    { id: 'big', areaByLayerMm2: [600], baseLayer: 2, childIds: [] },
  ];
  const g = analyzeSectionGraph(nodes, M);
  const base = g.perNode.find((v) => v.id === 'base')!;
  assert.ok(base.peelAreaAboveMm2 >= 600, 'base inherits the bigger of the two branches');
});

test('worstLayer is a GLOBAL layer index (baseLayer offset applied)', () => {
  const g = analyzeSectionGraph([{ id: 'n', areaByLayerMm2: [50, 0.4, 50], baseLayer: 100, childIds: [] }], M);
  assert.equal(g.worst!.worstLayer, 101, 'neck at local idx 1 → global 101');
});

test('upright stack (area decreasing upward) has no failing neck', () => {
  const nodes: SectionNode[] = [
    { id: 'base', areaByLayerMm2: [100, 100], baseLayer: 0, childIds: ['top'] },
    { id: 'top', areaByLayerMm2: [40, 16], baseLayer: 2, childIds: [] },
  ];
  const g = analyzeSectionGraph(nodes, M);
  assert.equal(g.failCount, 0);
  assert.ok(g.worst!.sf >= 1);
});

test('results sorted worst-first; counts consistent', () => {
  const nodes: SectionNode[] = [
    { id: 'weak', areaByLayerMm2: [0.3], baseLayer: 0, childIds: ['heavy'] },
    { id: 'heavy', areaByLayerMm2: [500], baseLayer: 1, childIds: [] },
  ];
  const g = analyzeSectionGraph(nodes, M);
  assert.equal(g.perNode[0].id, g.worst!.id);
  for (let i = 1; i < g.perNode.length; i++) assert.ok(g.perNode[i].sf >= g.perNode[i - 1].sf);
  assert.equal(g.nodeCount, 2);
});

test('cycle guard: self/mutual references do not hang', () => {
  const nodes: SectionNode[] = [
    { id: 'a', areaByLayerMm2: [5], baseLayer: 0, childIds: ['b'] },
    { id: 'b', areaByLayerMm2: [5], baseLayer: 1, childIds: ['a'] },
  ];
  const g = analyzeSectionGraph(nodes, M); // must terminate
  assert.equal(g.nodeCount, 2);
});
