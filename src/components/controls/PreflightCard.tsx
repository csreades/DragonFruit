import React from 'react';
import { Waves } from 'lucide-react';
import { Button, Card, CardHeader } from '@/components/ui/primitives';
import { NumberInput } from '@/components/ui/NumberInput';
import type { useIslandManager } from '@/volumeAnalysis/IslandScan/useIslandManager';

interface PreflightCardProps {
  islands: ReturnType<typeof useIslandManager>;
  hasGeometry: boolean;
}

/**
 * Pre-flight Check 1 — resin escape / squeeze-flow (bottom-band).
 *
 * Runs the anisotropic 2-D distance transform over the bottom N layers and
 * paints the worst layer's escape heatmap: RED = deep landlocked resin, which
 * on plate re-approach flexes the FEP instead of draining → thick layers.
 *
 * Warn-only palette: a pass recedes; the banner always states what this does
 * NOT check (sealed 3-D cavities; flats above the bottom band).
 */
export function PreflightCard({ islands, hasGeometry }: PreflightCardProps) {
  const canvasRef = React.useRef<HTMLCanvasElement | null>(null);
  const [pxMm, setPxMm] = React.useState(0.05);
  const [layers, setLayers] = React.useState(20);
  const [warnUm, setWarnUm] = React.useState(1500);

  const result = islands.preflightResult;
  const running = islands.preflightRunning;
  const error = islands.preflightError;

  // Paint the worst layer's heatmap onto the canvas whenever a result arrives.
  React.useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !result) return;
    const { heatmap_width: w, heatmap_height: h, heatmap_rgba } = result;
    if (!w || !h || heatmap_rgba.length < w * h * 4) return;
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const img = new ImageData(new Uint8ClampedArray(heatmap_rgba), w, h);
    ctx.putImageData(img, 0, 0);
  }, [result]);

  const worstMm = result ? result.worst_escape_um / 1000 : 0;
  const warnMm = warnUm / 1000;
  const severity = !result
    ? 'idle'
    : result.worst_escape_um > warnUm
      ? 'high'
      : result.worst_escape_um > warnUm * 0.66
        ? 'warn'
        : 'ok';
  const sevColor = severity === 'high' ? '#e0503a' : severity === 'warn' ? '#d9a441' : '#6b7280';

  return (
    <Card>
      <CardHeader
        left={(
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, fontWeight: 600 }}>
            <Waves size={15} /> Pre-Flight · Resin Escape
          </span>
        )}
      />
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10, padding: '4px 2px' }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
          <div className="flex flex-col gap-0.5">
            <label className="ui-meta" style={{ color: 'var(--text-muted)' }}>Layers</label>
            <NumberInput value={layers} onChange={(v) => setLayers(Math.max(1, Math.round(v)))} step={1} />
          </div>
          <div className="flex flex-col gap-0.5">
            <label className="ui-meta" style={{ color: 'var(--text-muted)' }}>Pixel (mm)</label>
            <NumberInput value={pxMm} onChange={(v) => setPxMm(Math.max(0.005, v))} step={0.01} />
          </div>
          <div className="flex flex-col gap-0.5">
            <label className="ui-meta" style={{ color: 'var(--text-muted)' }}>Warn (µm)</label>
            <NumberInput value={warnUm} onChange={(v) => setWarnUm(Math.max(0, v))} step={100} />
          </div>
        </div>

        <Button
          onClick={() => islands.onRunPreflightEscape({ pxMm, layers, warnUm })}
          disabled={!hasGeometry || running}
        >
          {running ? 'Checking…' : 'Run resin-escape check'}
        </Button>

        {error && <div style={{ color: '#e0503a', fontSize: 12 }}>Error: {error}</div>}

        {result && (
          <>
            <div style={{ fontSize: 13, color: sevColor, fontWeight: 600 }}>
              {severity === 'high' ? '⚠ ' : severity === 'warn' ? '△ ' : '· '}
              worst escape {worstMm.toFixed(2)} mm{' '}
              <span style={{ color: '#6b7280', fontWeight: 400 }}>
                (warn &gt; {warnMm.toFixed(2)} mm · layer {result.worst_layer} · {result.flagged_layers}/{result.layers_checked} flagged)
              </span>
            </div>

            <canvas
              ref={canvasRef}
              style={{
                width: '100%',
                imageRendering: 'pixelated',
                borderRadius: 6,
                border: '1px solid #2a2a33',
                background: '#0f0f14',
              }}
            />
            <div style={{ fontSize: 11, color: '#6b7280' }}>
              RED = deep landlocked resin (FEP-flex / thick-layer risk). Drain-hole
              candidates: {result.per_layer[result.worst_layer]?.drain_candidates.length ?? 0} peaks.
            </div>
          </>
        )}

        <div style={{ fontSize: 10.5, color: '#6b7280', lineHeight: 1.4, borderTop: '1px solid #23232b', paddingTop: 6 }}>
          Measures worst <b>in-plane lateral</b> escape over the bottom {layers} layers only.
          Does NOT check sealed 3-D cavities or flats above the bottom band.
        </div>
      </div>
    </Card>
  );
}
