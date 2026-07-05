import React from 'react';
import { Anchor } from 'lucide-react';
import { Card, CardHeader, Button } from '@/components/ui/primitives';
import { getSnapshot as getSupportSnapshot } from '@/supports/state';
import { trunksToSupportInputs } from '@/supports/buildability/supportGeometry';
import { runBuildabilitySweep, type SweepResult } from '@/supports/buildability/buildabilitySweep';
import { setBuildabilityOverlay, setBuildabilityOverlayEnabled, clearBuildabilityOverlay } from '@/supports/buildability/buildabilityOverlay';
import type { useIslandManager } from '@/volumeAnalysis/IslandScan/useIslandManager';

/**
 * Check 2 — Support Buildability Sweep (v1, native, on-demand).
 *
 * Runs the fail-safe safety-factor check over the current supports:
 * min(tension, bending) SF per strut (H1), worst-case peel attribution (H2).
 * Warn-only palette; a pass recedes. NOT reactive yet, NOT for imported
 * pre-supported meshes (both deferred per review).
 */
interface BuildabilityCardProps {
  islands: ReturnType<typeof useIslandManager>;
  hasGeometry: boolean;
}

export function BuildabilityCard({ islands, hasGeometry }: BuildabilityCardProps) {
  const [result, setResult] = React.useState<SweepResult | null>(null);
  const [ran, setRan] = React.useState(false);
  const [showOn3D, setShowOn3D] = React.useState(true);

  const sections = islands.sectionsResult;
  const sectionsRunning = islands.sectionsRunning;
  const sectionsError = islands.sectionsError;

  const run = React.useCallback(() => {
    const trunks = getSupportSnapshot().trunks;
    const inputs = trunksToSupportInputs(trunks);
    const r = runBuildabilitySweep(inputs);
    setResult(r);
    setRan(true);
    // Publish to the 3D overlay: recolour each strut by its SF band.
    const bandById: Record<string, 'fail' | 'marginal' | 'ok'> = {};
    for (const v of r.perSupport) bandById[v.id] = v.band;
    setBuildabilityOverlay(bandById, showOn3D);
  }, [showOn3D]);

  const toggle3D = React.useCallback((on: boolean) => {
    setShowOn3D(on);
    setBuildabilityOverlayEnabled(on);
  }, []);

  // Clear the overlay when the card unmounts (leaving analysis mode).
  React.useEffect(() => () => clearBuildabilityOverlay(), []);

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
        {/* Geometry mode — works on ANY mesh (part, baked-in supports). */}
        <div style={{ fontSize: 10.5, color: '#8a8a92', fontWeight: 600 }}>PART GEOMETRY (any mesh)</div>
        <Button onClick={() => islands.onRunPreflightSections({ pxMm: 0.1 })} disabled={!hasGeometry || sectionsRunning}>
          {sectionsRunning ? 'Analyzing…' : 'Analyze part geometry (necks)'}
        </Button>
        {sectionsError && <div style={{ color: '#e0503a', fontSize: 12 }}>Error: {sectionsError}</div>}
        {sections && (
          <>
            <div style={{ fontSize: 13, fontWeight: 600, color: sections.fail_count > 0 ? '#e0503a' : sections.marginal_count > 0 ? '#d9a441' : '#5a6b5a' }}>
              {sections.fail_count > 0 ? '⚠ ' : sections.marginal_count > 0 ? '△ ' : '· '}
              worst SF {Number.isFinite(sections.worst_sf) && sections.worst_sf < 1e8 ? sections.worst_sf.toFixed(2) : '∞'}
              <span style={{ color: '#6b7280', fontWeight: 400 }}>
                {' '}· {sections.fail_count} fail · {sections.marginal_count} marginal of {sections.component_count} bodies
              </span>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 3, maxHeight: 130, overflowY: 'auto' }}>
              {sections.necks.filter((n) => n.band !== 'ok').slice(0, 10).map((n, i) => (
                <div key={i} style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11.5, color: n.band === 'fail' ? '#e0503a' : '#d9a441', padding: '1px 4px' }}>
                  <span>layer {n.layer}</span>
                  <span>SF {n.sf.toFixed(2)} · {n.area_mm2.toFixed(2)}mm² neck</span>
                </div>
              ))}
            </div>
          </>
        )}

        <div style={{ borderTop: '1px solid #23232b', margin: '2px 0' }} />
        <div style={{ fontSize: 10.5, color: '#8a8a92', fontWeight: 600 }}>NATIVE SUPPORTS (exact struts)</div>
        <Button onClick={run}>Run buildability sweep</Button>

        {result && result.supportCount > 0 && (
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11.5, color: '#8a8a92', cursor: 'pointer' }}>
            <input type="checkbox" checked={showOn3D} onChange={(e) => toggle3D(e.target.checked)} />
            Recolour supports in 3D (fail = red, marginal = amber; a pass recedes)
          </label>
        )}

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
