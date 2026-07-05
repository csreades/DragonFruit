import React, { useMemo } from 'react';
import * as THREE from 'three';
import { getScanVisualPosition } from '@/utils/scanPositioning';
import type { ModelTransform } from '@/hooks/useModelTransform';
import type { SectionNeck } from '@/volumeAnalysis/Preflight/nativePreflightSections';

/**
 * Check 2 (geometry mode) — 3D neck markers.
 *
 * Renders a coloured sphere at each fail/marginal peel neck. Mirrors
 * IslandOverlay's positioning exactly: neck world coords come from the same
 * scan grid→world mapping (the analysis runs on the transformed geom), so
 * wrapping in the same getScanVisualPosition(transform) group aligns them with
 * the rendered mesh. Warn-only: an `ok` neck is not drawn.
 */
interface NeckMarkersProps {
  necks: SectionNeck[] | undefined;
  enabled: boolean;
  transform?: ModelTransform;
  clipLower?: number | null;
  clipUpper?: number | null;
}

export function NeckMarkers({ necks, enabled, transform, clipLower, clipUpper }: NeckMarkersProps) {
  const clippingPlanes = useMemo(() => {
    const planes: THREE.Plane[] = [];
    if (clipLower != null) planes.push(new THREE.Plane(new THREE.Vector3(0, 0, 1), -clipLower));
    if (clipUpper != null) planes.push(new THREE.Plane(new THREE.Vector3(0, 0, -1), clipUpper));
    return planes;
  }, [clipLower, clipUpper]);

  if (!enabled || !necks || necks.length === 0) return null;

  const flagged = necks.filter((n) => n.band !== 'ok');
  if (flagged.length === 0) return null;

  return (
    <group position={getScanVisualPosition(transform)}>
      {flagged.map((n, i) => (
        <mesh key={i} position={[n.x_world, n.y_world, n.z_world]} renderOrder={999}>
          <sphereGeometry args={[0.9, 16, 16]} />
          <meshBasicMaterial
            color={n.band === 'fail' ? '#e0503a' : '#d9a441'}
            transparent
            opacity={0.92}
            depthTest={false}
            clippingPlanes={clippingPlanes.length ? clippingPlanes : undefined}
          />
        </mesh>
      ))}
    </group>
  );
}
