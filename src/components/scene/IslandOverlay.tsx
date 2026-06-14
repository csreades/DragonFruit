import React, { useMemo } from 'react';
import * as THREE from 'three';
import { useFrame } from '@react-three/fiber';
import type { IslandMarker } from '@/volumeAnalysis/IslandScan/islandOverlayLogic';
import { applyIslandOverlay as drawIslandOverlay } from '@/volumeAnalysis/IslandScan/islandOverlayPainter';
import type { ModelTransform } from '@/hooks/useModelTransform';
import { getScanVisualPosition } from '@/utils/scanPositioning';

type IslandOverlayProps = {
  markers: IslandMarker[];
  meshRef?: THREE.Mesh | null;
  brushRadiusMm: number;
  color: string;
  opacity: number;
  transform?: ModelTransform;
  centerOffset?: THREE.Vector3;
  selectedIslandId?: number | null;
  clipLower?: number | null;
  clipUpper?: number | null;
};

/**
 * Renders 3D island shapes based on actual island geometry (cylinder pucks).
 * Applies a synchronized 2 Hz breathing opacity pulse across all overlays.
 */
export function IslandOverlay({
  markers,
  meshRef,
  brushRadiusMm,
  color,
  opacity,
  transform,
  centerOffset,
  selectedIslandId,
  clipLower,
  clipUpper,
}: IslandOverlayProps) {
  const threeColor = useMemo(() => new THREE.Color(color), [color]);
  const visibleColor = useMemo(() => new THREE.Color('#ffff00'), []); // Bright yellow when visible
  const occludedColor = useMemo(() => new THREE.Color('#fF6600'), []); // Vibrant red-orange when behind mesh

  // Initialize clipping planes once (update in-place to avoid recreation)
  const clippingPlanesRef = React.useRef<THREE.Plane[]>([]);

  React.useEffect(() => {
    const planes: THREE.Plane[] = [];

    if (clipLower != null) {
      planes.push(new THREE.Plane(new THREE.Vector3(0, 0, 1), -clipLower));
    }
    if (clipUpper != null) {
      planes.push(new THREE.Plane(new THREE.Vector3(0, 0, -1), clipUpper));
    }

    clippingPlanesRef.current = planes;
  }, [clipLower, clipUpper]);

  const clippingPlanes = clippingPlanesRef.current;

  // Memoize materials to optimize draw calls and allow synchronized animation
  const unselectedMaterial = useMemo(() => {
    return new THREE.MeshStandardMaterial({
      color: threeColor,
      transparent: true,
      opacity: opacity,
      depthTest: true,
      roughness: 0.8,
      metalness: 0.0,
      clippingPlanes: clippingPlanes,
      clipIntersection: true,
    });
  }, [threeColor, opacity, clippingPlanes]);

  const selectedOccludedMaterial = useMemo(() => {
    return new THREE.MeshStandardMaterial({
      color: occludedColor,
      transparent: true,
      opacity: 0.95,
      depthTest: false,
      depthWrite: false,
      roughness: 0.8,
      metalness: 0.0,
      clippingPlanes: clippingPlanes,
      clipIntersection: true,
    });
  }, [occludedColor, clippingPlanes]);

  const selectedVisibleMaterial = useMemo(() => {
    return new THREE.MeshStandardMaterial({
      color: visibleColor,
      transparent: true,
      opacity: 0.95,
      depthTest: true,
      depthWrite: false,
      roughness: 0.8,
      metalness: 0.0,
      clippingPlanes: clippingPlanes,
      clipIntersection: true,
    });
  }, [visibleColor, clippingPlanes]);

  // Clean up materials on unmount/re-creation
  React.useEffect(() => {
    return () => {
      unselectedMaterial.dispose();
      selectedOccludedMaterial.dispose();
      selectedVisibleMaterial.dispose();
    };
  }, [unselectedMaterial, selectedOccludedMaterial, selectedVisibleMaterial]);

  // Synchronize 2 Hz opacity breathing pulse across all overlay instances
  useFrame(({ clock }) => {
    const elapsed = clock.getElapsedTime();
    const pulseAlpha = 0.7 + 0.3 * Math.sin(elapsed * 12.566370618);
    unselectedMaterial.opacity = opacity * pulseAlpha;
    selectedOccludedMaterial.opacity = 0.95 * pulseAlpha;
    selectedVisibleMaterial.opacity = 0.95 * pulseAlpha;
  });

  if (markers.length === 0) {
    return null;
  }

  return (
    <group position={getScanVisualPosition(transform)}>
      {markers.map((marker) => {
        if (!marker.geometry) return null;

        // Special handling for Markers (Negative IDs)
        if (marker.id < 0) {
          const isSeed = marker.id < -1_000_000;
          const markerColor = isSeed ? '#00ff00' : '#ffff00'; // Green for Seed, Yellow for Center

          return (
            <mesh
              key={marker.id}
              geometry={marker.geometry}
              renderOrder={99999}
            >
              <meshBasicMaterial
                color={markerColor}
                depthTest={false}
                depthWrite={false}
                clippingPlanes={clippingPlanes}
              />
            </mesh>
          );
        }

        const isSelected = marker.id === selectedIslandId;

        if (isSelected) {
          return (
            <group key={marker.id}>
              {/* Occluded state - orange, no depth test, renders behind */}
              <mesh
                geometry={marker.geometry}
                material={selectedOccludedMaterial}
                renderOrder={999}
              />
              {/* Visible state - yellow, with depth test, renders on top */}
              <mesh
                geometry={marker.geometry}
                material={selectedVisibleMaterial}
                renderOrder={1000}
              />
            </group>
          );
        } else {
          return (
            <mesh
              key={marker.id}
              geometry={marker.geometry}
              material={unselectedMaterial}
            />
          );
        }
      })}
    </group>
  );
}
