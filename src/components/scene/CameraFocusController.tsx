import { useEffect, useRef, useMemo } from 'react';
import { useThree } from '@react-three/fiber';
import * as THREE from 'three';
import type { OrbitControls as OrbitControlsImpl } from 'three-stdlib';
import type { IslandMarker } from '@/volumeAnalysis/IslandScan/islandOverlayLogic';

type CameraFocusControllerProps = {
  selectedIslandId: number | null;
  islandMarkers: IslandMarker[];
};

/**
 * Animates camera to focus on selected island.
 * Smoothly transitions camera position and target to center the island in view.
 */
export function CameraFocusController({ selectedIslandId, islandMarkers }: CameraFocusControllerProps) {
  const { camera, controls, scene } = useThree();
  const animatingRef = useRef(false);

  const lastSelectedIslandIdRef = useRef<number | null>(null);
  const lastCameraRef = useRef<THREE.Camera | null>(null);
  const islandMarkersRef = useRef(islandMarkers);
  islandMarkersRef.current = islandMarkers;

  const hasMarkers = islandMarkers.length > 0;

  useEffect(() => {
    const cameraChanged = lastCameraRef.current !== camera;
    if (lastSelectedIslandIdRef.current === selectedIslandId && !cameraChanged) return;

    if (!selectedIslandId) {
      lastSelectedIslandIdRef.current = null;
      lastCameraRef.current = camera;
      return;
    }
    if (!hasMarkers || !controls) return;

    lastSelectedIslandIdRef.current = selectedIslandId;
    lastCameraRef.current = camera;

    // Find the selected island marker
    const markers = islandMarkersRef.current;
    const marker = markers.find(m => m.id === selectedIslandId);
    if (!marker) return;

    // Get OrbitControls instance
    const orbitControls = controls as unknown as OrbitControlsImpl;
    if (!orbitControls.target) return;

    // Calculate island center position
    const islandCenter = new THREE.Vector3(marker.centerX, marker.centerY, marker.baseZ);
    
    // Calculate optimal camera distance based on island size - closer to the model
    const pixelSize = 0.1; // Approximate pixel size in mm
    const estimatedRadius = Math.sqrt(marker.pixelCount) * pixelSize;
    const optimalDistance = Math.max(estimatedRadius * 4, 20); // Original focus distance

    // Try multiple viewing angles to find the best one
    const candidateDirections: THREE.Vector3[] = [];
    
    // Generate candidate directions in a full 360-degree sphere sampling ring-by-ring
    const elevations = [-0.8, -0.4, 0.0, 0.4, 0.8];
    const azimuthAngles = [0, 45, 90, 135, 180, 225, 270, 315];
    for (const zVal of elevations) {
      const rXY = Math.sqrt(Math.max(0, 1.0 - zVal * zVal));
      for (const deg of azimuthAngles) {
        const rad = (deg * Math.PI) / 180;
        candidateDirections.push(new THREE.Vector3(
          Math.cos(rad) * rXY,
          Math.sin(rad) * rXY,
          zVal
        ));
      }
    }
    
    candidateDirections.push(new THREE.Vector3(0, 0, -1)); // Straight up look
    candidateDirections.push(new THREE.Vector3(0, 0, 1));  // Straight down look
    
    // Test each candidate position to see if island would be in view
    let targetCameraPos: THREE.Vector3 | null = null;
    let bestScore = -Infinity;
    
    console.log('[CameraFocus] Testing', candidateDirections.length, 'candidate directions for island', marker.id);
    console.log('[CameraFocus] Island center:', islandCenter);
    
    const raycaster = new THREE.Raycaster();
    const rayDir = new THREE.Vector3();

    for (let i = 0; i < candidateDirections.length; i++) {
      const direction = candidateDirections[i];
      const testPos = new THREE.Vector3(
        islandCenter.x + direction.x * optimalDistance,
        islandCenter.y + direction.y * optimalDistance,
        islandCenter.z + direction.z * optimalDistance
      );
      
      // Calculate score for this position
      let score = 0;
      
      // Strongly prefer low camera angles looking up (worm's eye)
      if (testPos.z < islandCenter.z) {
        score += 500;
      }
      
      // Prefer side views over directly vertical top/bottom poles for better context
      const viewVector = new THREE.Vector3().subVectors(islandCenter, testPos).normalize();
      const steepness = Math.abs(viewVector.z);
      score += (1.0 - steepness) * 50;
      
      // Check if distance is appropriate
      const distanceToIsland = testPos.distanceTo(islandCenter);
      if (distanceToIsland > optimalDistance * 0.5 && distanceToIsland < optimalDistance * 2) {
        score += 20;
      }
      
      // Query model meshes dynamically for this run
      const modelMeshes: THREE.Mesh[] = [];
      scene.traverse((obj) => {
        if (obj instanceof THREE.Mesh && obj.userData?.thumbnailTintTarget === 'modelMesh') {
          modelMeshes.push(obj);
        }
      });

      // Occlusion check: raycast along camera projection direction towards islandCenter
      if (modelMeshes.length > 0) {
        rayDir.subVectors(islandCenter, testPos).normalize();
        
        // Orthographic camera uses parallel projection and near=-50000, so geometry 
        // behind the camera position still renders and can occlude.
        // Therefore, we start the raycast 1000 mm behind the camera position for orthographic cameras.
        const isOrthographicCamera = camera instanceof THREE.OrthographicCamera;
        const rayStart = isOrthographicCamera 
          ? testPos.clone().addScaledVector(rayDir, -1000) 
          : testPos;

        raycaster.set(rayStart, rayDir);
        const hits = raycaster.intersectObjects(modelMeshes, true);
        if (hits.length > 0) {
          const hitDist = hits[0].distance;
          const targetDist = rayStart.distanceTo(islandCenter);
          if (hitDist < targetDist - 0.5) {
            score -= 10000; // Penalize heavily if occluded by other parts of the model
          }
        }
      }
      
      if (score > bestScore) {
        bestScore = score;
        targetCameraPos = testPos;
      }
    }
    
    console.log('[CameraFocus] Best score:', bestScore, 'Position:', targetCameraPos?.toArray().map(v => v.toFixed(1)));
    
    // Final fallback: position below and to the side
    if (!targetCameraPos) {
      targetCameraPos = new THREE.Vector3(
        islandCenter.x + optimalDistance * 0.5,
        islandCenter.y + optimalDistance * 0.5,
        islandCenter.z - optimalDistance * 0.7
      );
      console.log('[CameraFocus] Using fallback position:', targetCameraPos.toArray().map(v => v.toFixed(1)));
    }

    // Convert starting position to spherical coordinates relative to islandCenter
    const startRel = new THREE.Vector3().subVectors(camera.position, islandCenter);
    const rStart = startRel.length();
    const phiStart = Math.acos(Math.max(-1, Math.min(1, startRel.z / (rStart || 1))));
    const thetaStart = Math.atan2(startRel.y, startRel.x);

    // Convert target position to spherical coordinates relative to islandCenter
    const targetRel = new THREE.Vector3().subVectors(targetCameraPos, islandCenter);
    const rTarget = targetRel.length();
    const phiTarget = Math.acos(Math.max(-1, Math.min(1, targetRel.z / (rTarget || 1))));
    const thetaTarget = Math.atan2(targetRel.y, targetRel.x);

    // Compute shortest path for theta (azimuth) rotation
    let thetaDiff = thetaTarget - thetaStart;
    thetaDiff = Math.atan2(Math.sin(thetaDiff), Math.cos(thetaDiff));

    // Orthographic camera zoom tracking
    const isOrthographic = camera instanceof THREE.OrthographicCamera;
    const startZoom = isOrthographic ? (camera as THREE.OrthographicCamera).zoom : 1;
    let targetZoom = startZoom;
    
    if (isOrthographic) {
      const ortho = camera as THREE.OrthographicCamera;
      const targetHalfHeight = optimalDistance * Math.tan(THREE.MathUtils.degToRad(50 * 0.5)); // 50 degrees fov equivalent
      targetZoom = THREE.MathUtils.clamp(ortho.top / Math.max(1e-6, targetHalfHeight), 0.0001, 200);
    }

    // Animate camera and controls
    animatingRef.current = true;
    
    const startTarget = orbitControls.target.clone();
    const duration = 800; // ms
    const startTime = performance.now();

    const animate = () => {
      if (!animatingRef.current) return;

      const elapsed = performance.now() - startTime;
      const t = Math.min(elapsed / duration, 1);
      
      // Ease-in-out function for smooth animation
      const eased = t < 0.5 
        ? 2 * t * t 
        : -1 + (4 - 2 * t) * t;

      // Spherical coordinate interpolation
      const r = THREE.MathUtils.lerp(rStart, rTarget, eased);
      const phi = THREE.MathUtils.lerp(phiStart, phiTarget, eased);
      const theta = thetaStart + thetaDiff * eased;

      // Convert back to Cartesian relative to islandCenter
      const x = islandCenter.x + r * Math.sin(phi) * Math.cos(theta);
      const y = islandCenter.y + r * Math.sin(phi) * Math.sin(theta);
      const z = islandCenter.z + r * Math.cos(phi);
      
      camera.position.set(x, y, z);

      // Interpolate zoom for Orthographic camera
      if (isOrthographic) {
        const ortho = camera as THREE.OrthographicCamera;
        ortho.zoom = THREE.MathUtils.lerp(startZoom, targetZoom, eased);
        ortho.updateProjectionMatrix();
      }
      
      // Interpolate controls target
      orbitControls.target.lerpVectors(startTarget, islandCenter, eased);
      orbitControls.update();

      if (t < 1) {
        requestAnimationFrame(animate);
      } else {
        animatingRef.current = false;
      }
    };

    animate();

    return () => {
      animatingRef.current = false;
    };
  }, [selectedIslandId, hasMarkers, camera, controls, scene]);

  return null;
}
