import * as THREE from 'three';

export function NormalDebugMaterial({
  clippingPlanes,
  invertNormals = false,
}: {
  clippingPlanes: THREE.Plane[];
  invertNormals?: boolean;
}) {
  return (
    <meshNormalMaterial
      clippingPlanes={clippingPlanes}
      side={invertNormals ? THREE.BackSide : THREE.FrontSide}
    />
  );
}
