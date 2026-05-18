import React, { useState } from 'react';
import { NumberInput } from '@/components/ui/NumberInput';
import { Card, CardHeader, IconButton, Input } from '@/components/ui/primitives';

type IslandOverlayControlsProps = {
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
  // Color + opacity drive the support-coverage halo uniforms (SoftClay
  // shader path). The legacy vertex-colour painter that used to read
  // these was retired alongside IslandOverlay.tsx.
  color: string;
  onColorChange: (color: string) => void;
  opacity: number;
  onOpacityChange: (opacity: number) => void;
  islandCount: number;
  // Halo shader controls — primary surface.
  haloIntensity?: number;
  onHaloIntensityChange?: (intensity: number) => void;
  // Support coverage halo toggle (step 10).
  showSupportVolumeHalo?: boolean;
  onShowSupportVolumeHaloChange?: (show: boolean) => void;
  // Island column highlight + overhang highlight (SoftClay shader path).
  showIslands?: boolean;
  onShowIslandsChange?: (show: boolean) => void;
  islandColor?: string;
  onIslandColorChange?: (color: string) => void;
  islandIntensity?: number;
  onIslandIntensityChange?: (v: number) => void;
  showOverhang?: boolean;
  onShowOverhangChange?: (show: boolean) => void;
  overhangColor?: string;
  onOverhangColorChange?: (color: string) => void;
  overhangAngleDeg?: number;
  onOverhangAngleDegChange?: (v: number) => void;
  // Advanced section (color + opacity for the support-coverage halo).
  // Defaults: visible + collapsed. Analysis tab passes hideAdvanced;
  // support tab passes defaultAdvancedExpanded.
  hideAdvanced?: boolean;
  defaultAdvancedExpanded?: boolean;
};

/**
 * Control card for the island overlay. Primary surface is the Islands &
 * Overhangs sub-section (toggles + colours + intensity/angle). The
 * Advanced disclosure holds colour + opacity for the support-coverage
 * halo (driven by the same overlayColor / overlayOpacity state that
 * once drove the now-retired vertex-colour painter).
 */
export function IslandOverlayControls({
  enabled,
  onEnabledChange,
  color,
  onColorChange,
  opacity,
  onOpacityChange,
  islandCount,
  haloIntensity = 0.7,
  onHaloIntensityChange,
  showSupportVolumeHalo = false,
  onShowSupportVolumeHaloChange,
  showIslands = true,
  onShowIslandsChange,
  islandColor = '#00E5FF',
  onIslandColorChange,
  islandIntensity = 0.85,
  onIslandIntensityChange,
  showOverhang = true,
  onShowOverhangChange,
  overhangColor = '#FFEB3B',
  onOverhangColorChange,
  overhangAngleDeg = 45,
  onOverhangAngleDegChange,
  hideAdvanced = false,
  defaultAdvancedExpanded = false,
}: IslandOverlayControlsProps) {
  const [expanded, setExpanded] = useState(enabled);
  const [advancedExpanded, setAdvancedExpanded] = useState(defaultAdvancedExpanded);
  const [editingColor, setEditingColor] = useState(color);

  React.useEffect(() => {
    setEditingColor(color);
  }, [color]);

  return (
    <Card>
      <CardHeader
        left={(
          <>
            <IconButton
            onClick={() => setExpanded(!expanded)}
            className="!p-0.5"
            title={expanded ? 'Collapse card' : 'Expand card'}
          >
            <svg
              className="w-3 h-3 transform transition-transform"
              style={{ color: expanded ? 'var(--accent)' : 'var(--text-muted)' }}
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              {expanded ? (
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              ) : (
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
              )}
            </svg>
            </IconButton>
            <h3 className="text-sm font-semibold" style={{ color: 'var(--text-strong)' }}>Island Overlay</h3>
          </>
        )}
        right={(
          <button
            type="button"
            onClick={() => onEnabledChange(!enabled)}
            className="h-8 min-w-[74px] rounded-md border px-2.5 text-[11px] font-semibold uppercase tracking-wide transition-colors"
            style={enabled
              ? {
                  borderColor: 'color-mix(in srgb, var(--accent), white 10%)',
                  background: 'color-mix(in srgb, var(--accent), var(--surface-0) 76%)',
                  color: 'var(--accent-contrast)',
                }
              : {
                  borderColor: 'var(--border-subtle)',
                  background: 'var(--surface-1)',
                  color: 'var(--text-muted)',
                }}
            title="Toggle Island Overlay"
          >
            {enabled ? 'ON' : 'OFF'}
          </button>
        )}
        hideDivider={!expanded}
      />

      {expanded && (
        <div className="px-2.5 pt-2 pb-3 space-y-2.5">
          {islandCount > 0 && (
            <div className="ui-meta">
              {islandCount} island{islandCount !== 1 ? 's' : ''} detected
            </div>
          )}

          {/* Support Coverage sub-section — visually divided from island controls. */}
          <div
            className="pt-2.5 mt-1.5"
            style={{ borderTop: '1px solid var(--border-subtle)' }}
          >
            <div className="ui-meta mb-1.5" style={{ color: 'var(--text-strong)' }}>
              Support Coverage
            </div>
            <label
              className="ui-meta flex items-center gap-2 cursor-pointer"
              title="Visualise roughly how much model each support is holding up."
            >
              <input
                type="checkbox"
                checked={showSupportVolumeHalo}
                onChange={(e) => onShowSupportVolumeHaloChange?.(e.target.checked)}
              />
              <span>Show support coverage</span>
            </label>
          </div>

          {/* Islands & Overhangs — SoftClay per-pixel shader path.
              Primary surface only: toggle + colour + one perceptual knob per
              effect. Geometry tuning (radius factor, column height, overhang
              proximity, overhang intensity) lives in useIslandManager defaults;
              re-introduce sliders here when there's a concrete tuning request. */}
          <div
            className="pt-2.5 mt-1.5"
            style={{ borderTop: '1px solid var(--border-subtle)' }}
          >
            <div className="ui-meta mb-1.5" style={{ color: 'var(--text-strong)' }}>
              Islands &amp; Overhangs
            </div>

            <label
              className="ui-meta flex items-center gap-2 cursor-pointer mb-2"
              title="Highlight detected unsupported regions on the model surface (vertical columns rising from each island base)."
            >
              <input
                type="checkbox"
                checked={showIslands}
                onChange={(e) => onShowIslandsChange?.(e.target.checked)}
              />
              <span>Show island columns</span>
            </label>

            {showIslands && (
              <div className="space-y-2 pl-5 mb-2">
                {islandCount === 0 && (
                  <div className="ui-meta" style={{ opacity: 0.6 }}>
                    No islands detected — run the scan to see highlights.
                  </div>
                )}
                <div className="flex items-center gap-2" title="Halo colour painted around each detected island.">
                  <span className="ui-meta" style={{ minWidth: 70 }}>Color</span>
                  <input
                    type="color"
                    value={islandColor}
                    onChange={(e) => onIslandColorChange?.(e.target.value)}
                  />
                </div>
                <div className="flex items-center gap-2" title="How vividly the halo blends over the model surface.">
                  <span className="ui-meta" style={{ minWidth: 70 }}>Intensity</span>
                  <input
                    type="range" min={0} max={1} step={0.01}
                    value={islandIntensity}
                    onChange={(e) => onIslandIntensityChange?.(parseFloat(e.target.value))}
                    style={{ flex: 1 }}
                  />
                  <span className="ui-meta" style={{ minWidth: 32, textAlign: 'right' }}>
                    {Math.round(islandIntensity * 100)}%
                  </span>
                </div>
              </div>
            )}

            <label
              className="ui-meta flex items-center gap-2 cursor-pointer mb-2"
              title={
                islandCount === 0
                  ? 'Run an island scan first — overhang highlights are gated on detected islands.'
                  : 'Highlight downward-facing surfaces near islands (the actual problem zones).'
              }
              style={{ opacity: islandCount === 0 ? 0.5 : 1 }}
            >
              <input
                type="checkbox"
                checked={showOverhang}
                disabled={islandCount === 0}
                onChange={(e) => onShowOverhangChange?.(e.target.checked)}
              />
              <span>Show overhang</span>
            </label>

            {showOverhang && islandCount > 0 && (
              <div className="space-y-2 pl-5">
                <div className="flex items-center gap-2" title="Highlight colour for downward-facing surfaces.">
                  <span className="ui-meta" style={{ minWidth: 70 }}>Color</span>
                  <input
                    type="color"
                    value={overhangColor}
                    onChange={(e) => onOverhangColorChange?.(e.target.value)}
                  />
                </div>
                <div
                  className="flex items-center gap-2"
                  title="Surface angle past vertical that counts as overhang. Lower = stricter (fewer fragments highlighted)."
                >
                  <span className="ui-meta" style={{ minWidth: 70 }}>Angle</span>
                  <input
                    type="range" min={10} max={80} step={1}
                    value={overhangAngleDeg}
                    onChange={(e) => onOverhangAngleDegChange?.(parseFloat(e.target.value))}
                    style={{ flex: 1 }}
                  />
                  <span className="ui-meta" style={{ minWidth: 32, textAlign: 'right' }}>
                    {overhangAngleDeg}°
                  </span>
                </div>
              </div>
            )}
          </div>

          {/* Advanced disclosure — preserves the legacy painter knobs.
              Analysis tab passes hideAdvanced; support tab passes
              defaultAdvancedExpanded so the section opens by default. */}
          {!hideAdvanced && (
          <div
            className="pt-2.5 mt-1.5"
            style={{ borderTop: '1px solid var(--border-subtle)' }}
          >
            <button
              type="button"
              onClick={() => setAdvancedExpanded(!advancedExpanded)}
              className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide"
              style={{ color: 'var(--text-muted)' }}
              title="Override automatic halo parameters and the legacy vertex-colour brush."
            >
              <svg
                className="w-3 h-3 transform transition-transform"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
                style={{ transform: advancedExpanded ? 'rotate(90deg)' : undefined }}
              >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
              </svg>
              <span>Advanced</span>
            </button>

            {advancedExpanded && (
              <div className="mt-2 space-y-2.5">
                <div className="ui-meta" style={{ opacity: 0.7, fontStyle: 'italic' }}>
                  Colour and opacity of the support-coverage halo
                  (the blob around each support tip).
                </div>
                <div className="space-y-1">
                  <label className="ui-meta">Color</label>
                  <div className="flex gap-1.5 items-center">
                    <input
                      type="color"
                      value={color}
                      onChange={(e) => {
                        const newColor = e.target.value;
                        setEditingColor(newColor);
                        onColorChange(newColor);
                      }}
                      className="w-10 h-8 rounded border cursor-pointer p-0"
                      style={{ borderColor: 'var(--border-subtle)', background: 'var(--surface-1)' }}
                    />
                    <Input
                      type="text"
                      value={editingColor}
                      onChange={(e) => setEditingColor(e.target.value)}
                      onBlur={(e) => {
                        const val = e.target.value.trim();
                        if (/^#[0-9a-fA-F]{6}$/.test(val) || /^#[0-9a-fA-F]{3}$/.test(val)) {
                          onColorChange(val);
                        } else {
                          setEditingColor(color);
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          e.currentTarget.blur();
                        }
                      }}
                      className="flex-1 !h-8 px-2 py-0 text-sm uppercase"
                      placeholder="#0433FF"
                    />
                  </div>
                </div>

                <div className="space-y-1">
                  <label className="ui-meta flex justify-between">
                    <span>Opacity</span>
                    <span style={{ color: 'var(--text-strong)' }}>{Math.round(opacity * 100)}%</span>
                  </label>
                  <input
                    type="range"
                    min="0.1"
                    max="1.0"
                    step="0.05"
                    value={opacity}
                    onChange={(e) => onOpacityChange(parseFloat(e.target.value))}
                    className="ui-range"
                  />
                </div>

                <div className="space-y-1" title="How vividly the support-coverage halo blends over the model surface.">
                  <label className="ui-meta flex justify-between">
                    <span>Halo intensity</span>
                    <span style={{ color: 'var(--text-strong)' }}>{Math.round(haloIntensity * 100)}%</span>
                  </label>
                  <input
                    type="range"
                    min="0"
                    max="1"
                    step="0.05"
                    value={haloIntensity}
                    onChange={(e) => onHaloIntensityChange?.(parseFloat(e.target.value))}
                    className="ui-range"
                  />
                </div>
              </div>
            )}
          </div>
          )}
        </div>
      )}
    </Card>
  );
}
