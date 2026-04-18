import type { ShapedPlacementPreview } from './useShapedSupportPlacement';

let currentPreview: ShapedPlacementPreview | null = null;

type Listener = () => void;
const listeners = new Set<Listener>();

function notify() {
    listeners.forEach((l) => {
        try { l(); } catch (e) { console.error('[ShapedPreviewState] listener error', e); }
    });
}

export function setShapedPreview(preview: ShapedPlacementPreview | null) {
    currentPreview = preview;
    notify();
}

export function getShapedPreview(): ShapedPlacementPreview | null {
    return currentPreview;
}

export function subscribeToShapedPreview(listener: Listener): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
}
