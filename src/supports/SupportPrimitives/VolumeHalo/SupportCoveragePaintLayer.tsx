import React from 'react';
import { subscribe, getSnapshot } from '@/supports/state';
import type { SupportCoverageTipData } from '@/features/shaders/mesh/registry';
import { MAX_SUPPORT_TIPS } from '@/features/shaders/mesh/softClay';

// Halo radius (mm) = contact diameter × this. Bigger tips → bigger halos.
const COVERAGE_RADIUS_FACTOR = 5;
const MIN_BRUSH_RADIUS_MM = 0.5;
const MAX_BRUSH_RADIUS_MM = 4.0;

interface ContactConeLike {
  pos?: { x: number; y: number; z: number };
  profile?: { contactDiameterMm?: number };
}

interface ContactDiskLike {
  pos?: { x: number; y: number; z: number };
  contactDiameterMm?: number;
}

interface SupportWithContact {
  modelId?: string;
  contactCone?: ContactConeLike;
  contactConeA?: ContactConeLike;
  contactConeB?: ContactConeLike;
  contactDiskA?: ContactDiskLike;
  contactDiskB?: ContactDiskLike;
}

function packCone(
  buf: Float32Array,
  offset: number,
  cone: ContactConeLike | undefined,
  fallbackDiameter: number,
): boolean {
  if (!cone?.pos) return false;
  const diameter = cone.profile?.contactDiameterMm ?? fallbackDiameter;
  const radius = Math.min(
    MAX_BRUSH_RADIUS_MM,
    Math.max(MIN_BRUSH_RADIUS_MM, (diameter * COVERAGE_RADIUS_FACTOR) / 2),
  );
  buf[offset + 0] = cone.pos.x;
  buf[offset + 1] = cone.pos.y;
  buf[offset + 2] = cone.pos.z;
  buf[offset + 3] = radius;
  return true;
}

function packDisk(
  buf: Float32Array,
  offset: number,
  disk: ContactDiskLike | undefined,
  fallbackDiameter: number,
): boolean {
  if (!disk?.pos) return false;
  const diameter = disk.contactDiameterMm ?? fallbackDiameter;
  const radius = Math.min(
    MAX_BRUSH_RADIUS_MM,
    Math.max(MIN_BRUSH_RADIUS_MM, (diameter * COVERAGE_RADIUS_FACTOR) / 2),
  );
  buf[offset + 0] = disk.pos.x;
  buf[offset + 1] = disk.pos.y;
  buf[offset + 2] = disk.pos.z;
  buf[offset + 3] = radius;
  return true;
}

function packTips(modelId: string | null | undefined): SupportCoverageTipData {
  const snap = getSnapshot();
  const tips = new Float32Array(MAX_SUPPORT_TIPS * 4);
  let count = 0;

  const visit = (
    collection: Record<string, unknown> | undefined,
    kind: 'cone' | 'sticklike' | 'twiglike',
  ) => {
    if (!collection) return;
    for (const value of Object.values(collection)) {
      if (count >= MAX_SUPPORT_TIPS) return;
      const sup = value as SupportWithContact;
      if (modelId != null && sup.modelId != null && sup.modelId !== modelId) continue;
      const fallback = 0.4;
      if (kind === 'cone') {
        if (packCone(tips, count * 4, sup.contactCone, fallback)) count += 1;
      } else if (kind === 'sticklike') {
        if (count < MAX_SUPPORT_TIPS && packCone(tips, count * 4, sup.contactConeA, fallback)) count += 1;
        if (count < MAX_SUPPORT_TIPS && packCone(tips, count * 4, sup.contactConeB, fallback)) count += 1;
      } else if (kind === 'twiglike') {
        if (count < MAX_SUPPORT_TIPS && packDisk(tips, count * 4, sup.contactDiskA, fallback)) count += 1;
        if (count < MAX_SUPPORT_TIPS && packDisk(tips, count * 4, sup.contactDiskB, fallback)) count += 1;
      }
    }
  };

  const anySnap = snap as unknown as Record<string, Record<string, unknown>>;
  visit(anySnap.trunks, 'cone');
  visit(anySnap.branches, 'cone');
  visit(anySnap.leaves, 'cone');
  visit(anySnap.sticks, 'sticklike');
  visit(anySnap.twigs, 'twiglike');

  return { tips, count };
}

// Subscribes to support state, packs the current support tips into a
// flat Float32Array, and returns it to the caller. The caller feeds this
// into <StlMesh supportCoverageTips={...}/>, which routes it through the
// MeshShaderMaterial registry into SoftClayMaterial's shader patch — the
// halo is computed PER PIXEL there, polygon-independent.
export function useSupportCoverageTips(args: {
  enabled: boolean;
  modelId?: string | null;
}): SupportCoverageTipData | undefined {
  const [tipData, setTipData] = React.useState<SupportCoverageTipData | undefined>(undefined);

  React.useEffect(() => {
    if (!args.enabled) {
      setTipData(undefined);
      return;
    }

    setTipData(packTips(args.modelId));

    // 80ms debounce so rapid placement/drag/undo cascades don't
    // re-pack 12 times per second. The shader uniform update is cheap
    // (a Float32Array copy) but React state churn isn't.
    let pending = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const unsub = subscribe(() => {
      if (pending) return;
      pending = true;
      timer = setTimeout(() => {
        pending = false;
        timer = null;
        setTipData(packTips(args.modelId));
      }, 80);
    });
    return () => {
      unsub();
      if (timer != null) clearTimeout(timer);
    };
  }, [args.enabled, args.modelId]);

  return tipData;
}

// Backward-compat: previous component-style entry point. Renders nothing
// itself (data is now consumed via the shader uniform path); the old
// `<SupportCoveragePaintLayer ...>` mount remained in SceneCanvas, so we
// kept the export name and shape but reduced the body to a no-op
// renderer that just runs the hook for its side effect. Replace its
// mount with the hook directly when convenient.
interface LegacyProps {
  meshRef: { current: unknown };
  enabled: boolean;
  tintColor: string;
  intensity?: number;
  modelId?: string | null;
}
export function SupportCoveragePaintLayer(_props: LegacyProps) {
  return null;
}
