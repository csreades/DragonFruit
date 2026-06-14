
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
 * Renders 3D island shapes based on actual island geometry.
 * Creates low-poly 3D objects from the first few layers of each island.
 * Applies the same transform as the main mesh to keep overlays aligned.
 */
export function IslandOverlay({ markers, meshRef, brushRadiusMm, color, opacity, transform, centerOffset, selectedIslandId, clipLower, clipUpper }: IslandOverlayProps) {
  // console.log(`[${ new Date().toISOString() }][IslandOverlay] Render start`);
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

  // console.log('[IslandOverlay] Rendering with:', {
  //   markerCount: markers.length,
  //   color,
  //   opacity,
  //   hasTransform: !!transform,
  //   hasCenterOffset: !!centerOffset,
  //   centerOffset: centerOffset ? { x: centerOffset.x, y: centerOffset.y, z: centerOffset.z } : null,
  //   selectedIslandId
  // });

  if (markers.length === 0) {
    // console.log('[IslandOverlay] No markers to render');
    return null;
  }

  // Apply X/Y translation only - marker geometries are already in world space (including auto-lift and rotation)
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
              <GlowMesh
                geometry={marker.geometry}
                color={occludedColor}
                opacity={0.95}
                selected={true}
                clippingPlanes={clippingPlanes}
                depthTest={false}
                depthWrite={false}
                renderOrder={999}
              />

              {/* Visible state - yellow, with depth test, renders on top */}
              <GlowMesh
                geometry={marker.geometry}
                color={visibleColor}
                opacity={0.95}
                selected={true}
                clippingPlanes={clippingPlanes}
                depthTest={true}
                depthWrite={false}
                renderOrder={1000}
              />
            </group>
          );
        } else {
          return (
            <GlowMesh
              key={marker.id}
              geometry={marker.geometry}
              color={threeColor}
              opacity={opacity}
              selected={false}
              clippingPlanes={clippingPlanes}
              depthTest={true}
              depthWrite={false}
            />
          );
        }
      })}
    </group>
  );
}

const VERTEX_SHADER = `
#include <clipping_planes_pars_vertex>
varying vec3 vNormal;
varying vec3 vViewPosition;

void main() {
  #include <clipping_planes_vertex>
  vNormal = normalize(normalMatrix * normal);
  vec4 mvPosition = modelViewMatrix * vec4(position, 1.0);
  vViewPosition = -mvPosition.xyz;
  gl_Position = projectionMatrix * mvPosition;
}
`;

const FRAGMENT_SHADER = `
#include <clipping_planes_pars_fragment>
uniform vec3 uColor;
uniform float uOpacity;
uniform float uTime;
uniform float uSelected;

varying vec3 vNormal;
varying vec3 vViewPosition;

void main() {
  #include <clipping_planes_fragment>
  
  vec3 normal = normalize(vNormal);
  vec3 viewDir = normalize(vViewPosition);
  
  float dotProduct = abs(dot(normal, viewDir));
  float fresnel = pow(1.0 - dotProduct, 2.5);
  float laserCore = pow(dotProduct, 16.0);
  float pulse = 1.0 + 0.15 * sin(uTime * 4.0);
  
  float selectionMultiplier = uSelected > 0.5 ? 1.5 : 1.0;
  
  vec3 coreColor = vec3(1.0);
  vec3 glowColor = mix(uColor, coreColor, laserCore * 0.4);
  
  float alpha = clamp((uOpacity * (0.6 + 0.4 * fresnel) + laserCore * 0.3) * pulse * selectionMultiplier, 0.0, 0.95);
  
  gl_FragColor = vec4(glowColor, alpha);
}
`;

interface GlowMeshProps {
  geometry: THREE.BufferGeometry;
  color: THREE.Color;
  opacity: number;
  selected?: boolean;
  clippingPlanes?: THREE.Plane[];
  depthTest?: boolean;
  depthWrite?: boolean;
  renderOrder?: number;
}

function GlowMesh({
  geometry,
  color,
  opacity,
  selected = false,
  clippingPlanes = [],
  depthTest = true,
  depthWrite = false,
  renderOrder = 0,
}: GlowMeshProps) {
  const materialRef = React.useRef<THREE.ShaderMaterial>(null);

  useFrame((state) => {
    if (materialRef.current) {
      materialRef.current.uniforms.uTime.value = state.clock.getElapsedTime();
    }
  });

  const uniforms = useMemo(() => ({
    uColor: { value: color },
    uOpacity: { value: opacity },
    uTime: { value: 0 },
    uSelected: { value: selected ? 1.0 : 0.0 },
  }), [color, opacity, selected]);

  React.useEffect(() => {
    if (materialRef.current) {
      materialRef.current.uniforms.uColor.value = color;
      materialRef.current.uniforms.uOpacity.value = opacity;
      materialRef.current.uniforms.uSelected.value = selected ? 1.0 : 0.0;
    }
  }, [color, opacity, selected]);

  return (
    <mesh geometry={geometry} renderOrder={renderOrder}>
      <shaderMaterial
        ref={materialRef}
        clipping={true}
        clippingPlanes={clippingPlanes}
        depthTest={depthTest}
        depthWrite={depthWrite}
        transparent={true}
        uniforms={uniforms}
        vertexShader={VERTEX_SHADER}
        fragmentShader={FRAGMENT_SHADER}
        clipIntersection={true}
      />
    </mesh>
  );
}
