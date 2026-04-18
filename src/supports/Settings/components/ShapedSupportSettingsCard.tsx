"use client";

import React from 'react';
import type { ShapedSupportSettings } from '../../SupportTypes/ShapedSupport/types';
import { NumberInput } from '@/components/ui/NumberInput';
import { setAnatomyPreviewActiveSettingKey } from '../AnatomyPreview/previewState';

interface ShapedSupportSettingsCardProps {
    settings: ShapedSupportSettings;
    onChange: (settings: Partial<ShapedSupportSettings>) => void;
}

export function ShapedSupportSettingsCard({ settings, onChange }: ShapedSupportSettingsCardProps) {
    if (!settings) return null;

    const makeFocusHandlers = React.useCallback((key: string) => {
        return {
            onFocusCapture: () => {
                setAnatomyPreviewActiveSettingKey(key);
            },
            onBlurCapture: (e: React.FocusEvent<HTMLElement>) => {
                const next = e.relatedTarget as Node | null;
                if (next && e.currentTarget.contains(next)) return;
                setAnatomyPreviewActiveSettingKey(null);
            },
        };
    }, []);

    const compactInputClass = 'ui-input h-8 w-full px-2.5 text-xs sm:text-sm no-spinners';

    return (
        <div className="space-y-2.5">
            <div className="space-y-1 min-w-0" {...makeFocusHandlers('shaped.contactDiameterMm')}>
                <div className="text-[11px] font-medium" style={{ color: 'var(--text-muted)' }}>
                    Contact Diameter (mm)
                </div>
                <NumberInput
                    value={settings.contactDiameterMm}
                    onChange={(val) => onChange({ contactDiameterMm: val })}
                    min={0.1}
                    max={5.0}
                    step={0.05}
                    className={compactInputClass}
                />
            </div>

            <div className="space-y-1 min-w-0" {...makeFocusHandlers('shaped.maxLengthMm')}>
                <div className="text-[11px] font-medium" style={{ color: 'var(--text-muted)' }}>
                    Max Length (mm)
                </div>
                <NumberInput
                    value={settings.maxLengthMm}
                    onChange={(val) => onChange({ maxLengthMm: val })}
                    min={0.5}
                    max={50.0}
                    step={0.5}
                    className={compactInputClass}
                />
            </div>

            <div className="space-y-1 min-w-0" {...makeFocusHandlers('shaped.chamferRadiusMm')}>
                <div className="text-[11px] font-medium" style={{ color: 'var(--text-muted)' }}>
                    Chamfer Radius (mm)
                </div>
                <NumberInput
                    value={settings.chamferRadiusMm}
                    onChange={(val) => onChange({ chamferRadiusMm: val })}
                    min={0.01}
                    max={2.0}
                    step={0.05}
                    className={compactInputClass}
                />
            </div>

            <div className="space-y-1 min-w-0" {...makeFocusHandlers('shaped.bodyHeightMm')}>
                <div className="text-[11px] font-medium" style={{ color: 'var(--text-muted)' }}>
                    Body Height (mm)
                </div>
                <NumberInput
                    value={settings.bodyHeightMm}
                    onChange={(val) => onChange({ bodyHeightMm: val })}
                    min={0.5}
                    max={10.0}
                    step={0.25}
                    className={compactInputClass}
                />
            </div>

            <div className="space-y-1 min-w-0" {...makeFocusHandlers('shaped.penetrationMm')}>
                <div className="text-[11px] font-medium" style={{ color: 'var(--text-muted)' }}>
                    Penetration (mm)
                </div>
                <NumberInput
                    value={settings.penetrationMm}
                    onChange={(val) => onChange({ penetrationMm: val })}
                    min={0}
                    max={0.5}
                    step={0.01}
                    className={compactInputClass}
                />
            </div>
        </div>
    );
}
