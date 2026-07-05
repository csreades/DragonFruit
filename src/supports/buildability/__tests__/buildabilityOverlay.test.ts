import assert from 'node:assert/strict';
import test from 'node:test';

import {
  bandOverlayColor,
  setBuildabilityOverlay,
  setBuildabilityOverlayEnabled,
  clearBuildabilityOverlay,
  getBuildabilityOverlay,
  overlayColorForSupport,
  subscribeBuildabilityOverlay,
} from '../buildabilityOverlay';

test('bandOverlayColor: fail=red, marginal=amber, ok recedes (null)', () => {
  assert.equal(bandOverlayColor('fail'), '#e0503a');
  assert.equal(bandOverlayColor('marginal'), '#d9a441');
  assert.equal(bandOverlayColor('ok'), null); // warn-only: a pass has no override
});

test('setBuildabilityOverlay maps only fail/marginal into colorById (ok omitted)', () => {
  setBuildabilityOverlay({ a: 'fail', b: 'marginal', c: 'ok' }, true);
  const s = getBuildabilityOverlay();
  assert.equal(s.enabled, true);
  assert.equal(s.colorById['a'], '#e0503a');
  assert.equal(s.colorById['b'], '#d9a441');
  assert.equal('c' in s.colorById, false, 'ok supports must not be recoloured');
});

test('overlayColorForSupport respects the enabled flag', () => {
  setBuildabilityOverlay({ a: 'fail' }, true);
  assert.equal(overlayColorForSupport('a'), '#e0503a');
  assert.equal(overlayColorForSupport('missing'), null);
  setBuildabilityOverlayEnabled(false);
  assert.equal(overlayColorForSupport('a'), null, 'disabled overlay returns no colour');
});

test('subscribers fire on change; clear resets', () => {
  let hits = 0;
  const unsub = subscribeBuildabilityOverlay(() => { hits++; });
  setBuildabilityOverlay({ a: 'fail' }, true);
  setBuildabilityOverlayEnabled(false);
  assert.ok(hits >= 2);
  clearBuildabilityOverlay();
  const s = getBuildabilityOverlay();
  assert.equal(s.enabled, false);
  assert.deepEqual(s.colorById, {});
  unsub();
});

test('setBuildabilityOverlayEnabled no-ops (no emit) when unchanged', () => {
  clearBuildabilityOverlay();
  let hits = 0;
  const unsub = subscribeBuildabilityOverlay(() => { hits++; });
  setBuildabilityOverlayEnabled(false); // already false
  assert.equal(hits, 0);
  unsub();
});
