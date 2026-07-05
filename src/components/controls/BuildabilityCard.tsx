import React from 'react';
import { Anchor } from 'lucide-react';
import { Card, CardHeader, Button } from '@/components/ui/primitives';
import { getSnapshot as getSupportSnapshot } from '@/supports/state';
import { trunksToSupportInputs } from '@/supports/buildability/supportGeometry';
import { runBuildabilitySweep, type SweepResult } from '@/supports/buildability/buildabilitySweep';

/**
 * Check 2 — Support Buildability Sweep (v1, native, on-demand).
 *
 * Runs the fail-safe safety-factor check over the current supports:
 * min(tension, bending) SF per strut (H1), worst-case peel attribution (H2).
 * Warn-only palette; a pass recedes. NOT reactive yet, NOT for imported
 * pre-supported meshes (both deferred per review).
 */
export function BuildabilityCard() {
  const [result, setResult] = React.useState<SweepResult | null>(null);
  const [ran, setRan] = React.useState(false);

  const run = React.useCallback(() => {
    const trunks = getSupportSnapshot().trunks;
    const inputs = trunksToSupportInputs(trunks);
    setResult(runBuildabilitySweep(inputs));
    setRan(true);
  }, []);

  const bandColor = (band: 'fail' | 'marginal' | 'ok') =>
    band === 'fail' ? '#e0503a' : band === 'marginal' ? '#d9a441' : '#5a6b5a';

  const fmtSf = (sf: number) => (Number.isFinite(sf) ? sf.toFixed(2) : '∞');

  return (
    <Card>
      <CardHeader
        left={(
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, fontWeight: 600 }}>
            <Anchor size={15} /> Pre-Flight · Support Buildability
          </span>
        )}
      />
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10, padding: '4px 2px' }}>
        <Button onClick={run}>Run buildability sweep</Button>

        {ran && result && result.supportCount === 0 && (
          <div style={{ fontSize: 12, color: '#6b7280' }}>
            No supports in the scene — generate supports first, then run.
          </div>
        )}

        {result && result.supportCount > 0 && (
          <>
            <div style={{ fontSize: 13, fontWeight: 600, color: result.failCount > 0 ? '#e0503a' : result.marginalCount > 0 ? '#d9a441' : '#5a6b5a' }}>
              {result.failCount > 0 ? '⚠ ' : result.marginalCount > 0 ? '△ ' : '· '}
              {result.failCount} predicted-fail · {result.marginalCount} marginal
              <span style={{ color: '#6b7280', fontWeight: 400 }}> of {result.supportCount} supports</span>
            </div>

            <div style={{ display: 'flex', flexDirection: 'column', gap: 3, maxHeight: 160, overflowY: 'auto' }}>
              {result.perSupport.slice(0, 12).map((v) => (
                <div key={v.id} style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11.5, color: bandColor(v.band), padding: '1px 4px' }}>
                  <span>{v.id.slice(0, 8)}</span>
                  <span>
                    SF {fmtSf(v.sf)} <span style={{ color: '#6b7280' }}>({v.governingMode})</span>
                  </span>
                </div>
              ))}
              {result.perSupport.length > 12 && (
                <div style={{ fontSize: 10.5, color: '#6b7280', padding: '1px 4px' }}>
                  +{result.perSupport.length - 12} more…
                </div>
              )}
            </div>
          </>
        )}

        <div style={{ fontSize: 10.5, color: '#6b7280', lineHeight: 1.4, borderTop: '1px solid #23232b', paddingTop: 6 }}>
          SF &lt; 1 predicts failure (min of tension &amp; bending, fail-safe). Peel
          demand is a conservative estimate, NOT a truss solve; v1 covers native
          supports only. A pass is <b>necessary, not sufficient</b>.
        </div>
      </div>
    </Card>
  );
}
