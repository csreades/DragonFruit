import * as THREE from 'three';

export function OpaqueWireOverlayMaterial({
  clippingPlanes,
  invertNormals = false,
}: {
  clippingPlanes: THREE.Plane[];
  invertNormals?: boolean;
}) {
  return (
    <meshBasicMaterial
      color="#AAAAAA"
      clippingPlanes={clippingPlanes}
      side={invertNormals ? THREE.BackSide : THREE.DoubleSide}
      wireframe
      polygonOffset
      polygonOffsetFactor={-1}
      polygonOffsetUnits={-1}
      transparent
      opacity={0.9}
      depthTest
      depthWrite={false}
    />
  );
}
