/**
 * Check 2 — 3D overlay store.
 *
 * Bridges the buildability sweep result to the support renderer: the card
 * publishes a per-support-id → band-colour map + an on/off flag; the renderer
 * subscribes and recolours each strut by its safety-factor band.
 *
 * Warn-only palette (per review): problems stand out, a pass RECEDES — an
 * `ok` support keeps its normal colour so only fail/marginal draw the eye.
 */
export type SafetyBand = 'fail' | 'marginal' | 'ok';

/** Fail = red, marginal = amber, ok = null (recede to the normal support colour). */
export function bandOverlayColor(band: SafetyBand): string | null {
  if (band === 'fail') return '#e0503a';
  if (band === 'marginal') return '#d9a441';
  return null;
}

interface OverlayState {
  enabled: boolean;
  /** trunk/support id → override hex (only fail/marginal are present). */
  colorById: Record<string, string>;
}

let state: OverlayState = { enabled: false, colorById: {} };
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

/** Publish a fresh sweep result to the overlay (only fail/marginal get colours). */
export function setBuildabilityOverlay(
  bandById: Record<string, SafetyBand>,
  enabled: boolean,
): void {
  const colorById: Record<string, string> = {};
  for (const [id, band] of Object.entries(bandById)) {
    const c = bandOverlayColor(band);
    if (c) colorById[id] = c;
  }
  state = { enabled, colorById };
  emit();
}

export function setBuildabilityOverlayEnabled(enabled: boolean): void {
  if (state.enabled === enabled) return;
  state = { ...state, enabled };
  emit();
}

export function clearBuildabilityOverlay(): void {
  state = { enabled: false, colorById: {} };
  emit();
}

export function getBuildabilityOverlay(): OverlayState {
  return state;
}

/** Renderer helper: the override colour for a support id, or null. */
export function overlayColorForSupport(id: string): string | null {
  if (!state.enabled) return null;
  return state.colorById[id] ?? null;
}

export function subscribeBuildabilityOverlay(fn: () => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}
