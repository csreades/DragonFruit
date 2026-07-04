'use client';

import React from 'react';
import { Zap, X } from 'lucide-react';

type Props = {
  adapterName: string;
  backendApi?: string;
  onEnable: () => void;
  onDismiss: () => void;
};

/**
 * One-time startup prompt shown when a usable slicing GPU is detected and
 * GPU acceleration hasn't been enabled (or the prompt dismissed) yet.
 * Either choice can be changed later in Settings → Slicing.
 */
export function GpuDetectedModal({ adapterName, backendApi, onEnable, onDismiss }: Props) {
  return (
    <div className="fixed inset-0 z-[220] flex items-center justify-center bg-black/55 backdrop-blur-sm px-3">
      <div
        className="w-full max-w-lg overflow-hidden rounded-xl border shadow-2xl"
        style={{
          background: 'var(--surface-0)',
          borderColor: 'var(--border-subtle)',
          boxShadow: '0 24px 46px rgba(0,0,0,0.42)',
        }}
        role="dialog"
        aria-modal="true"
        aria-label="Enable GPU acceleration"
      >
        {/* Header */}
        <div
          className="flex items-center justify-between gap-4 border-b px-5 py-4"
          style={{ borderColor: 'var(--border-subtle)' }}
        >
          <div className="flex min-w-0 items-center gap-3">
            <span
              className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-md border"
              style={{
                borderColor: 'color-mix(in srgb, var(--accent), var(--border-subtle) 50%)',
                background: 'color-mix(in srgb, var(--accent), var(--surface-1) 85%)',
                color: 'var(--accent)',
              }}
            >
              <Zap className="h-4 w-4" />
            </span>
            <div className="min-w-0 pr-2">
              <h2 className="text-base font-semibold leading-tight" style={{ color: 'var(--text-strong)' }}>
                GPU Detected
              </h2>
              <p className="mt-0.5 text-[11px] leading-snug" style={{ color: 'var(--text-muted)' }}>
                Your graphics card can accelerate slicing
              </p>
            </div>
          </div>

          <button
            type="button"
            className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-md border transition-colors"
            style={{
              borderColor: 'var(--border-subtle)',
              background: 'var(--surface-1)',
              color: 'var(--text-muted)',
            }}
            aria-label="Dismiss"
            onClick={onDismiss}
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body */}
        <div className="space-y-4 p-5">
          <p className="text-xs leading-relaxed" style={{ color: 'var(--text-muted)' }}>
            DragonFruit can slice on your GPU for a large speed-up on busy build plates.
            If anything goes wrong on the GPU, slicing automatically falls back to the CPU
            engine, so output is always correct. You can change this anytime in
            Settings&nbsp;→&nbsp;Slicing.
          </p>

          <div
            className="rounded-lg border px-3 py-2.5"
            style={{
              borderColor: 'var(--border-subtle)',
              background: 'color-mix(in srgb, var(--surface-1), black 8%)',
            }}
          >
            <div className="text-[11px] uppercase tracking-wide" style={{ color: 'var(--text-muted)' }}>
              Detected GPU
            </div>
            <div className="mt-1 text-sm font-semibold leading-tight" style={{ color: 'var(--text-strong)' }}>
              {adapterName}
              {backendApi ? (
                <span className="ml-2 text-[11px] font-normal" style={{ color: 'var(--text-muted)' }}>
                  ({backendApi})
                </span>
              ) : null}
            </div>
          </div>

          <div className="flex shrink-0 items-center justify-end gap-2 pt-1">
            <button
              type="button"
              className="ui-button ui-button-secondary !h-9 px-3 text-xs inline-flex items-center gap-1.5"
              onClick={onDismiss}
            >
              Not Now
            </button>
            <button
              type="button"
              className="ui-button !h-9 px-3 text-xs inline-flex items-center gap-1.5"
              style={{
                borderColor: 'color-mix(in srgb, var(--accent), var(--border-subtle) 45%)',
                background: 'color-mix(in srgb, var(--accent), var(--surface-1) 86%)',
                color: 'color-mix(in srgb, var(--accent), var(--text-strong) 18%)',
              }}
              onClick={onEnable}
            >
              <Zap className="h-3.5 w-3.5" />
              Enable GPU Acceleration
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
