import * as THREE from 'three';
import { Vec3 } from '../types';
import { checkShaftCollision } from './CollisionUtils';
import { SDFCache } from './Pathfinding/SDFCache';

const sdfCacheByMeshUuid = new Map<string, SDFCache>();

function getOrCreateCollisionSdf(mesh: THREE.Mesh): SDFCache | null {
    const geometry = mesh.geometry as any;
    if (!geometry?.boundsTree) {
        return null;
    }

    const existing = sdfCacheByMeshUuid.get(mesh.uuid);
    if (existing) {
        existing.refreshMatrix();
        return existing;
    }

    const sdf = new SDFCache(mesh, { cellSize: 0.25 });
    sdf.refreshMatrix();
    sdfCacheByMeshUuid.set(mesh.uuid, sdf);
    return sdf;
}

function segmentBlockedWithBestAvailableMethod(
    start: Vec3,
    end: Vec3,
    collisionRadius: number,
    mesh: THREE.Mesh,
): boolean {
    const sdf = getOrCreateCollisionSdf(mesh);
    if (sdf) {
        return sdf.segmentBlocked(start.x, start.y, start.z, end.x, end.y, end.z, collisionRadius);
    }

    return checkShaftCollision(start, end, collisionRadius, mesh).hit;
}

export function isCollisionSegmentBlocked(
    start: Vec3,
    end: Vec3,
    collisionRadius: number,
    mesh: THREE.Mesh,
): boolean {
    return segmentBlockedWithBestAvailableMethod(start, end, collisionRadius, mesh);
}

/**
 * Calculates the required standoff distance (offset) from a surface to ensure
 * a connecting element (like a cone) does not collide with the mesh.
 * 
 * @param surfacePos - The starting point on the model surface.
 * @param surfaceNormal - The normal vector at the surface point (direction to extend).
 * @param targetPos - The target position the element connects to (e.g., Socket).
 * @param collisionRadius - The radius of the element for collision checking (include safety margin).
 * @param mesh - The model mesh to check against.
 * @param minOffset - The minimum/starting offset to test.
 * @param maxOffset - The maximum allowable offset.
 * @param step - The increment for iterative testing (default: 0.2mm).
 * @returns The calculated safe offset distance.
 */
export function calculateSafeOffset(
    surfacePos: Vec3,
    surfaceNormal: Vec3,
    targetPos: Vec3,
    collisionRadius: number,
    mesh: THREE.Mesh,
    minOffset: number,
    maxOffset: number,
    step: number = 0.2
): number {
    const start = new THREE.Vector3(surfacePos.x, surfacePos.y, surfacePos.z);
    const normal = new THREE.Vector3(surfaceNormal.x, surfaceNormal.y, surfaceNormal.z).normalize();
    const normalizedStep = Math.max(0.025, step);
    let previousBlockedOffset = minOffset;

    const testOffset = (offset: number): boolean => {
        // Calculate proposed start position: Surface + (Normal * t)
        const testStartVec = start.clone().add(normal.clone().multiplyScalar(offset));
        const testStart: Vec3 = { x: testStartVec.x, y: testStartVec.y, z: testStartVec.z };

        return segmentBlockedWithBestAvailableMethod(testStart, targetPos, collisionRadius, mesh);
    };

    if (!testOffset(minOffset)) {
        return minOffset;
    }

    for (let t = minOffset + normalizedStep; t <= maxOffset + 0.000001; t += normalizedStep) {
        const clampedOffset = Math.min(t, maxOffset);
        if (!testOffset(clampedOffset)) {
            let low = previousBlockedOffset;
            let high = clampedOffset;

            for (let i = 0; i < 6; i++) {
                const mid = (low + high) / 2;
                if (testOffset(mid)) {
                    low = mid;
                } else {
                    high = mid;
                }
            }

            return high;
        }

        previousBlockedOffset = clampedOffset;
    }

    return maxOffset;
}
