/**
 * GPU slicing acceleration bridge.
 *
 * The native side keeps a runtime backend preference (Tauri command
 * `set_slice_backend_preference`) that the frontend OWNS via the persisted
 * "GPU acceleration" setting in Settings → Slicing — the frontend replays it
 * on startup and on every settings change. The `DF_SLICE_BACKEND` env var
 * (power-user shortcut hook) still overrides whatever is set here.
 */

export type GpuDetectResult = {
  /** True only for a GPU-enabled build with a usable hardware adapter. */
  available: boolean;
  adapterName?: string | null;
  /** Graphics API the adapter would use (e.g. "Vulkan", "Dx12"). */
  backendApi?: string | null;
};

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

let detectPromise: Promise<GpuDetectResult> | null = null;

/**
 * Probe for a usable slicing GPU (no device creation, ~instant). Memoized —
 * the adapter set doesn't change while the app runs. Outside the Tauri
 * runtime (browser dev) this resolves to unavailable.
 */
export function detectSlicingGpu(): Promise<GpuDetectResult> {
  if (!isTauriRuntime()) return Promise.resolve({ available: false });
  if (!detectPromise) {
    detectPromise = import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<GpuDetectResult>('detect_gpu'))
      .then((result) => result ?? { available: false })
      .catch(() => ({ available: false }));
  }
  return detectPromise;
}

/** Push the persisted "GPU acceleration" preference into the native slicer. */
export async function applySliceBackendPreference(gpuEnabled: boolean): Promise<void> {
  if (!isTauriRuntime()) return;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('set_slice_backend_preference', { backend: gpuEnabled ? 'gpu' : 'default' });
  } catch (error) {
    console.error('[gpu] failed to apply slice backend preference:', error);
  }
}
