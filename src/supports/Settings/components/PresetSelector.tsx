"use client";

import React, { useState, useEffect, useRef, useSyncExternalStore } from 'react';
import ReactDOM from 'react-dom';
import { PenLine, Pencil, Trash2, Save } from 'lucide-react';
import { StructuredDialogModal } from '@/components/ui/StructuredDialogModal';
import { Button } from '@/components/ui/primitives';
import {
    getPresetList,
    getActivePreset,
    setActivePreset,
    subscribeToPresets,
    savePreset,
    updateCustomPresetMetadata,
    createPreset,
    deletePreset,
    isPresetDirtyForSettings,
} from '../presets';
import { getSettings, subscribeToSettings } from '../state';
import { setAnatomyPreviewHoveredPresetSettings } from '../AnatomyPreview/previewState';

type PresetSelectorProps = {
    selectedPresetIdOverride?: string | null;
    onPresetSelected?: (presetId: string) => void;
    disableGlobalPresetActivation?: boolean;
};

export function PresetSelector({
    selectedPresetIdOverride,
    onPresetSelected,
    disableGlobalPresetActivation = false,
}: PresetSelectorProps) {
    const settings = useSyncExternalStore(subscribeToSettings, getSettings, getSettings);
    const [presets, setPresets] = useState(() => getPresetList());
    const [activePreset, setActivePresetState] = useState(() => getActivePreset());
    const [confirmId, setConfirmId] = useState<string | null>(null);
    const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null);
    const [hoveredPresetId, setHoveredPresetId] = useState<string | null>(null);
    const [isEditingName, setIsEditingName] = useState(false);
    const [tempName, setTempName] = useState('');
    const [tempDescription, setTempDescription] = useState('');
    const [newPresetName, setNewPresetName] = useState('My Preset');
    const [contextMenu, setContextMenu] = useState<{ x: number; y: number; presetId: string } | null>(null);
    const contextMenuRef = useRef<HTMLDivElement | null>(null);

    // Global click listener to dismiss the context menu
    useEffect(() => {
        if (!contextMenu) return;
        const handleClick = (e: MouseEvent) => {
            if (contextMenuRef.current && !contextMenuRef.current.contains(e.target as Node)) {
                setContextMenu(null);
            }
        };
        // Delay attachment so the right-click event doesn't immediately dismiss it
        requestAnimationFrame(() => window.addEventListener('click', handleClick));
        return () => window.removeEventListener('click', handleClick);
    }, [contextMenu]);

    useEffect(() => {
        const unsubscribe = subscribeToPresets(() => {
            setPresets(getPresetList());
            setActivePresetState(getActivePreset());
        });
        return unsubscribe;
    }, []);

    const builtInPresets = presets.filter((preset) => preset.isBuiltIn);
    const customPresets = presets.filter((preset) => !preset.isBuiltIn);

    const effectiveSelectedPresetId = selectedPresetIdOverride === undefined
        ? activePreset?.id ?? null
        : selectedPresetIdOverride;
    const selectedPreset = effectiveSelectedPresetId
        ? presets.find((preset) => preset.id === effectiveSelectedPresetId) ?? null
        : null;
    const selectedPresetIsBuiltIn = selectedPreset?.isBuiltIn ?? false;
    const hoveredPreset = hoveredPresetId ? presets.find((preset) => preset.id === hoveredPresetId) ?? null : null;
    const previewDescription = hoveredPreset?.description ?? selectedPreset?.description ?? '';
    const selectedPresetIsDirty = isPresetDirtyForSettings(effectiveSelectedPresetId, settings);

    useEffect(() => {
        if (!selectedPreset) {
            setTempName('');
            setTempDescription('');
            setIsEditingName(false);
            return;
        }

        if (!isEditingName) {
            setTempName(selectedPreset.name);
            setTempDescription(selectedPreset.description ?? '');
        }
    }, [selectedPreset, isEditingName]);

    // Dynamically calculate the available space for the preset list so we only shrink
    // it as much as needed to avoid the outer Support Studio panel becoming scrollable.
    const wrapperRef = useRef<HTMLDivElement | null>(null);
    const [computedMaxHeight, setComputedMaxHeight] = useState<string>('19rem');

    // Dynamically calculate the available space for the preset list so it never
    // overflows the outer Support Studio panel.
    useEffect(() => {
        function recalc() {
            if (!wrapperRef.current) return;
            const rect = wrapperRef.current.getBoundingClientRect();
            const top = rect.top;
            const viewportHeight = window.innerHeight;

            // Reserve 48px for the action button row below the list.
            const available = Math.max(120, viewportHeight - top - 48 - 24);
            const maxClamp = 304;
            const final = Math.min(available, maxClamp);
            setComputedMaxHeight(`${final}px`);
        }

        recalc();
        window.addEventListener('resize', recalc);
        return () => window.removeEventListener('resize', recalc);
    }, []);

    function renderPresetRow(preset: (typeof presets)[number]) {
        const isSelected = effectiveSelectedPresetId === preset.id;
        const showDirtyIndicator = isSelected && selectedPresetIsDirty;

        return (
            <button
                type="button"
                className="w-full px-3 py-2 text-sm relative rounded-[5px] border transition-colors"
                onClick={() => {
                    handlePresetSelect(preset.id);
                }}
                onContextMenu={(e) => {
                    if (preset.isBuiltIn) return;
                    e.preventDefault();
                    e.stopPropagation();
                    // Dismiss any other open context menus (e.g. the floating panel's "Reset this window" menu)
                    window.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, cancelable: true }));
                    setContextMenu({ x: e.clientX, y: e.clientY, presetId: preset.id });
                }}
                onMouseEnter={() => {
                    setHoveredPresetId(preset.id);
                    setAnatomyPreviewHoveredPresetSettings(preset.settings);
                }}
                onMouseLeave={() => {
                    setHoveredPresetId(null);
                    setAnatomyPreviewHoveredPresetSettings(null);
                }}
                onFocus={() => {
                    setHoveredPresetId(preset.id);
                    setAnatomyPreviewHoveredPresetSettings(preset.settings);
                }}
                onBlur={() => {
                    setHoveredPresetId(null);
                    setAnatomyPreviewHoveredPresetSettings(null);
                }}
                style={{
                    background: isSelected
                        ? 'color-mix(in srgb, var(--primary-button-surface), var(--surface-0) 90%)'
                        : 'var(--surface-0)',
                    borderColor: isSelected
                        ? 'color-mix(in srgb, var(--primary-button-surface), var(--border-subtle) 30%)'
                        : 'var(--border-subtle)',
                }}
            >
                {isSelected ? (
                    <span
                        aria-hidden="true"
                        className="pointer-events-none absolute left-2 top-1/2 inline-block h-2 w-2 -translate-y-1/2 rounded-full border"
                        style={{
                            background: 'var(--primary-button-surface)',
                            borderColor: 'color-mix(in srgb, var(--primary-button-surface), var(--surface-0) 40%)',
                        }}
                    />
                ) : null}
                {showDirtyIndicator ? (
                    <span
                        aria-hidden="true"
                        title="Preset has unsaved changes"
                        className="pointer-events-none absolute right-2 top-1/2 inline-flex -translate-y-1/2"
                        style={{ color: isSelected ? 'var(--text-muted)' : 'var(--text-muted)' }}
                    >
                        <PenLine className="h-3 w-3" />
                    </span>
                ) : null}
                <div className="w-full">
                    <div className="flex items-center justify-center text-center">
                        <div className="flex-1 truncate" style={{ color: isSelected ? 'var(--text-strong)' : undefined }}>
                            {preset.name}
                        </div>
                    </div>
                </div>
            </button>
        );
    }

    function rowSpanClass(index: number, total: number): string {
        // If a row has only one tile (odd trailing item), let it fill the full row
        return total % 2 === 1 && index === total - 1 ? 'col-span-2' : '';
    }

    const handlePresetSelect = (presetId: string) => {
        if (presetId === '__separator') {
            return;
        }

        if (!disableGlobalPresetActivation) {
            setActivePreset(presetId);
        }
        onPresetSelected?.(presetId);
        setHoveredPresetId(null);
        setConfirmId(null);
        setDeleteConfirmId(null);
        setIsEditingName(false);
    };

    const handleSaveRequest = () => {
        if (!selectedPreset || selectedPresetIsBuiltIn) return;
        setConfirmId(selectedPreset.id);
    };

    const handleEditClick = () => {
        if (!selectedPreset || selectedPreset.isBuiltIn) return;

        if (isEditingName) {
            const trimmed = tempName.trim();
            if (trimmed.length > 0) {
                updateCustomPresetMetadata(selectedPreset.id, trimmed, tempDescription);
            } else {
                setTempName(selectedPreset.name);
            }
            setTempDescription(
                tempDescription.trim().length > 0
                    ? tempDescription.trim()
                    : 'User custom preset',
            );
            setIsEditingName(false);
            return;
        }

        setTempName(selectedPreset.name);
        setTempDescription(selectedPreset.description ?? '');
        setIsEditingName(true);
    };

    const handleCreateNewClick = () => {
        const created = createPreset(newPresetName);
        setActivePreset(created.id);
        setConfirmId(null);
        setIsEditingName(false);
    };

    return (
        <div className="space-y-2">
            <div className="space-y-1">
                <div ref={wrapperRef}>
                    <div className="overflow-y-auto custom-scrollbar py-1 transition-[max-height] duration-200" style={{ maxHeight: computedMaxHeight }}>
                        <div className="grid grid-cols-2 gap-1">
                            {builtInPresets.map((preset, index) => (
                                <div key={preset.id} className={rowSpanClass(index, builtInPresets.length)}>
                                    {renderPresetRow(preset)}
                                </div>
                            ))}
                        </div>

                        {customPresets.length > 0 ? (
                            <>
                                <div className="mx-3 my-2 border-t" style={{ borderColor: 'var(--border-subtle)' }} />
                                <div className="grid grid-cols-2 gap-1 px-1">
                                    {customPresets.map((preset, index) => (
                                        <div key={preset.id} className={rowSpanClass(index, customPresets.length)}>
                                            {renderPresetRow(preset)}
                                        </div>
                                    ))}
                                </div>
                            </>
                        ) : null}
                    </div>
                </div>
                {previewDescription ? (
                    <div className="text-[11px] text-center" style={{ color: 'var(--text-muted)' }}>
                        {previewDescription}
                    </div>
                ) : null}
            </div>

            {/* Action row */}
            <div className="grid grid-cols-2 gap-1.5">
                <Button
                    type="button"
                    variant="accent"
                    size="md"
                    className="h-9 text-[12px] font-semibold"
                    onClick={handleCreateNewClick}
                    title="Create a new preset from current settings"
                >
                    New
                </Button>
                <Button
                    type="button"
                    variant="primary"
                    size="md"
                    className="h-9 text-[12px] font-semibold"
                    onClick={handleSaveRequest}
                    disabled={!selectedPreset || selectedPresetIsBuiltIn}
                    title={selectedPresetIsBuiltIn ? 'Built-in presets cannot be saved' : 'Save current settings to this preset'}
                >
                    Save
                </Button>
            </div>

            {/* ── Overwrite Preset Modal ─────────────────────────────────── */}
            <StructuredDialogModal
                open={confirmId !== null && selectedPreset !== null && confirmId === selectedPreset.id}
                ariaLabel="Overwrite preset"
                title={`Save Over "${selectedPreset?.name ?? ''}"?`}
                subtitle="This will replace the preset with your current settings."
                icon={<Save className="h-4 w-4" />}
                iconTone="accent"
                zIndexClassName="z-[300]"
                closeAriaLabel="Cancel overwrite"
                onClose={() => setConfirmId(null)}
                actions={(
                    <>
                        <button
                            type="button"
                            className="ui-button ui-button-secondary !h-9 w-full px-3 text-xs"
                            onClick={() => setConfirmId(null)}
                        >
                            Cancel
                        </button>
                        <button
                            type="button"
                            className="ui-button ui-button-primary !h-9 w-full px-3 text-xs inline-flex items-center justify-center gap-1.5"
                            onClick={() => {
                                if (selectedPreset) {
                                    savePreset(selectedPreset.id);
                                }
                                setConfirmId(null);
                            }}
                        >
                            <Save className="h-3.5 w-3.5" />
                            Save
                        </button>
                    </>
                )}
            >
                <p className="text-xs leading-relaxed" style={{ color: 'var(--text-muted)' }}>
                    Overwrite the preset <strong style={{ color: 'var(--text-strong)' }}>{selectedPreset?.name ?? ''}</strong> with the current scene settings?
                </p>
            </StructuredDialogModal>

            {/* ── Delete Preset Modal ────────────────────────────────────── */}
            <StructuredDialogModal
                open={deleteConfirmId !== null && selectedPreset !== null && deleteConfirmId === selectedPreset.id}
                ariaLabel="Delete preset"
                title={`Delete "${selectedPreset?.name ?? ''}"?`}
                subtitle="This action cannot be undone."
                icon={<Trash2 className="h-4 w-4" />}
                iconTone="warning"
                zIndexClassName="z-[300]"
                closeAriaLabel="Cancel delete"
                onClose={() => setDeleteConfirmId(null)}
                actions={(
                    <>
                        <button
                            type="button"
                            className="ui-button ui-button-secondary !h-9 w-full px-3 text-xs"
                            onClick={() => setDeleteConfirmId(null)}
                        >
                            Cancel
                        </button>
                        <button
                            type="button"
                            className="ui-button !h-9 w-full px-3 text-xs inline-flex items-center justify-center gap-1.5"
                            style={{
                                borderColor: 'color-mix(in srgb, #ef4444, var(--border-subtle) 45%)',
                                background: 'color-mix(in srgb, #ef4444, var(--surface-1) 86%)',
                                color: 'var(--danger)',
                            }}
                            onClick={() => {
                                if (selectedPreset) {
                                    deletePreset(selectedPreset.id);
                                }
                                setDeleteConfirmId(null);
                                setIsEditingName(false);
                            }}
                        >
                            <Trash2 className="h-3.5 w-3.5" />
                            Delete
                        </button>
                    </>
                )}
            >
                <p className="text-xs leading-relaxed" style={{ color: 'var(--text-muted)' }}>
                    This will permanently remove the preset <strong style={{ color: 'var(--text-strong)' }}>{selectedPreset?.name ?? ''}</strong> and all of its saved settings.
                </p>
            </StructuredDialogModal>

            {/* ── Edit Preset Modal ──────────────────────────────────────── */}
            <StructuredDialogModal
                open={isEditingName && selectedPreset !== null && !selectedPresetIsBuiltIn}
                ariaLabel="Edit preset"
                title="Edit Preset"
                subtitle={`Update name and description for "${selectedPreset?.name ?? ''}"`}
                icon={<Pencil className="h-4 w-4" />}
                iconTone="accent"
                zIndexClassName="z-[300]"
                closeAriaLabel="Cancel editing"
                onClose={() => {
                    if (selectedPreset) {
                        setTempName(selectedPreset.name);
                        setTempDescription(selectedPreset.description ?? '');
                    }
                    setIsEditingName(false);
                }}
                actions={(
                    <>
                        <button
                            type="button"
                            className="ui-button ui-button-secondary !h-9 w-full px-3 text-xs"
                            onClick={() => {
                                if (selectedPreset) {
                                    setTempName(selectedPreset.name);
                                    setTempDescription(selectedPreset.description ?? '');
                                }
                                setIsEditingName(false);
                            }}
                        >
                            Cancel
                        </button>
                        <button
                            type="button"
                            className="ui-button ui-button-primary !h-9 w-full px-3 text-xs inline-flex items-center justify-center gap-1.5"
                            disabled={tempName.trim().length === 0}
                            onClick={handleEditClick}
                        >
                            <Save className="h-3.5 w-3.5" />
                            Apply
                        </button>
                    </>
                )}
            >
                <div className="space-y-3">
                    <div>
                        <div className="text-[11px] font-medium mb-1" style={{ color: 'var(--text-muted)' }}>
                            Preset Name
                        </div>
                        <input
                            type="text"
                            value={tempName}
                            onChange={(event) => setTempName(event.target.value)}
                            onKeyDown={(event) => {
                                if (event.key === 'Enter' && tempName.trim().length > 0) {
                                    handleEditClick();
                                } else if (event.key === 'Escape') {
                                    if (selectedPreset) {
                                        setTempName(selectedPreset.name);
                                    }
                                    setIsEditingName(false);
                                }
                            }}
                            className="ui-input h-9 w-full px-3 text-sm"
                            placeholder="Preset name"
                            autoFocus
                        />
                    </div>
                    <div>
                        <div className="text-[11px] font-medium mb-1" style={{ color: 'var(--text-muted)' }}>
                            Description
                        </div>
                        <input
                            type="text"
                            value={tempDescription}
                            onChange={(event) => setTempDescription(event.target.value)}
                            onKeyDown={(event) => {
                                if (event.key === 'Enter' && tempName.trim().length > 0) {
                                    handleEditClick();
                                } else if (event.key === 'Escape') {
                                    if (selectedPreset) {
                                        setTempName(selectedPreset.name);
                                        setTempDescription(selectedPreset.description ?? '');
                                    }
                                    setIsEditingName(false);
                                }
                            }}
                            className="ui-input h-9 w-full px-3 text-sm"
                            placeholder="Preset description"
                        />
                    </div>
                </div>
            </StructuredDialogModal>

            {/* ── Right-click Context Menu ──────────────────────────────── */}
            {contextMenu ? ReactDOM.createPortal(
                <div
                    ref={contextMenuRef}
                    className="fixed z-[140] pointer-events-auto w-48 rounded-lg border p-1.5 shadow-xl"
                    style={{
                        left: contextMenu.x,
                        top: contextMenu.y,
                        borderColor: 'var(--border-subtle)',
                        background: 'color-mix(in srgb, var(--surface-0), #000 10%)',
                    }}
                    onPointerDown={(e) => e.stopPropagation()}
                >
                    <button
                        type="button"
                        className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-[13px] font-medium transition-colors"
                        style={{ color: 'var(--text-strong)' }}
                        onMouseEnter={(e) => { e.currentTarget.style.background = 'color-mix(in srgb, var(--accent), var(--surface-1) 84%)'; }}
                        onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; }}
                        onClick={() => {
                            const preset = presets.find((p) => p.id === contextMenu.presetId);
                            if (!preset || preset.isBuiltIn) return;
                            handlePresetSelect(preset.id);
                            setTempName(preset.name);
                            setTempDescription(preset.description ?? '');
                            setIsEditingName(true);
                            setContextMenu(null);
                        }}
                    >
                        <Pencil className="h-3.5 w-3.5" />
                        Rename
                    </button>
                    <button
                        type="button"
                        className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-[13px] font-medium transition-colors"
                        style={{ color: 'var(--danger)' }}
                        onMouseEnter={(e) => { e.currentTarget.style.background = 'color-mix(in srgb, var(--danger), var(--surface-1) 90%)'; }}
                        onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; }}
                        onClick={() => {
                            const preset = presets.find((p) => p.id === contextMenu.presetId);
                            if (!preset || preset.isBuiltIn) return;
                            handlePresetSelect(preset.id);
                            setDeleteConfirmId(preset.id);
                            setContextMenu(null);
                        }}
                    >
                        <Trash2 className="h-3.5 w-3.5" />
                        Delete
                    </button>
                </div>,
                document.body
            ) : null}
        </div>
    );
}
