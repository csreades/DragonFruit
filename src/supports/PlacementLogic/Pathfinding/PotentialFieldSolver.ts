import { Vec3 } from '../../types';
import type { SDFCache } from './SDFCache';

export interface PotentialFieldSolverOptions {
    /** Minimum allowed clearance from the model. */
    clearanceMm: number;
    /** Safety margin within which repulsion begins to scale up. Default 2.0mm. */
    marginMm?: number;
    /** The repulsion coefficient/force multiplier. Default 5.0. */
    repulsionStrength?: number;
    /** Step size of the integration path in mm. Default 1.0mm. */
    stepMm?: number;
    /** Maximum steps/iterations allowed. Default 300. */
    maxSteps?: number;
    /** Maximum lateral (XY) deviation allowed from the starting position. Default 30mm. */
    maxLateralMm?: number;
    /** Set to true to simplify the path to straight segments/joints. Default true. */
    simplify?: boolean;
}

export interface PotentialFieldSolverResult {
    /** Solved waypoints (from socket position down to goalZ). */
    path: Vec3[];
    /** Whether the path successfully reached goalZ without stagnating. */
    reached: boolean;
    /** Whether search stagnated (Z progress stopped). */
    stagnated: boolean;
    /** Number of integration steps executed. */
    iterations: number;
    /** Position where the path stagnated, if reached is false. */
    stagnationPos?: Vec3;
}

/**
 * Greedy line-of-sight simplification: keep only waypoints where the
 * direct segment to the next kept waypoint would be blocked by model geometry.
 */
function simplifyPath(path: Vec3[], sdf: SDFCache, clearance: number): Vec3[] {
    if (path.length <= 2) return path;

    // First pass: enforce Z-monotonicity.
    const monoPath: Vec3[] = [path[0]];
    let minZ = path[0].z;
    for (let i = 1; i < path.length; i++) {
        if (path[i].z <= minZ) {
            monoPath.push(path[i]);
            minZ = path[i].z;
        }
    }
    if (monoPath.length <= 2) return monoPath;

    // Second pass: greedy line-of-sight collapse.
    const result: Vec3[] = [monoPath[0]];
    let anchor = 0;

    for (let probe = 2; probe < monoPath.length; probe++) {
        const a = monoPath[anchor];
        const b = monoPath[probe];

        if (sdf.segmentBlocked(a.x, a.y, a.z, b.x, b.y, b.z, clearance)) {
            result.push(monoPath[probe - 1]);
            anchor = probe - 1;
        }
    }

    result.push(monoPath[monoPath.length - 1]);
    return result;
}

/**
 * Solves support routing using continuous potential field integration.
 * Routes a virtual particle from startPos downwards to goalZ, repelled by model geometry via the SDF.
 */
export function solvePotentialField(
    sdf: SDFCache,
    startPos: Vec3,
    goalZ: number,
    opts: PotentialFieldSolverOptions
): PotentialFieldSolverResult {
    const clearance = opts.clearanceMm;
    const margin = opts.marginMm ?? 2.0;
    const repulsionStrength = opts.repulsionStrength ?? 5.0;
    const stepMm = opts.stepMm ?? 1.0;
    const maxSteps = opts.maxSteps ?? 300;
    const maxLateral = opts.maxLateralMm ?? 30;
    const maxLateralSq = maxLateral * maxLateral;
    const simplify = opts.simplify ?? true;

    const path: Vec3[] = [{ ...startPos }];
    let current = { ...startPos };
    let iterations = 0;
    let reached = false;
    let stagnated = false;
    let stagnationPos: Vec3 | undefined;

    let bestZ = startPos.z;
    let bestLateral = 0;
    let stepsWithoutProgress = 0;
    const STAGNATION_STEPS_LIMIT = 40;

    while (iterations < maxSteps) {
        iterations++;

        if (current.z <= goalZ) {
            reached = true;
            current.z = goalZ;
            path.push({ ...current });
            break;
        }

        let { distance: d, gradient: grad } = sdf.distanceAndGradientAt(current.x, current.y, current.z);

        // If we are inside/close to an obstacle but the gradient is zero (e.g. flat mock SDF or local minimum),
        // default the gradient to point straight UP (0, 0, 1) to repel against gravity.
        if (d < clearance + margin && grad.x === 0 && grad.y === 0 && grad.z === 0) {
            grad = { x: 0, y: 0, z: 1 };
        }

        let wRepulsion = 0;
        if (d < clearance + margin) {
            const penetration = (clearance + margin) - d;
            wRepulsion = repulsionStrength * Math.pow(penetration / margin, 2);
        }

        // Calculate lateral escape direction (normalized XY gradient).
        // If the particle is under an overhang (grad.z > 0), we want to slide
        // laterally to find an exit.
        let escapeX = 0;
        let escapeY = 0;
        const hLen = Math.sqrt(grad.x * grad.x + grad.y * grad.y);
        if (hLen > 1e-4) {
            escapeX = grad.x / hLen;
            escapeY = grad.y / hLen;
        } else {
            // Break symmetry if the gradient is purely vertical
            escapeX = 1;
            escapeY = 0;
        }

        // Transfer vertical repulsion to lateral escape force to slide out under overhangs.
        const lateralSlideWeight = grad.z > 0 ? grad.z * 0.75 : 0;

        let vx = 0 + wRepulsion * (grad.x + escapeX * lateralSlideWeight);
        let vy = 0 + wRepulsion * (grad.y + escapeY * lateralSlideWeight);
        let vz = -1.0 + wRepulsion * grad.z * (1 - lateralSlideWeight * 0.5);

        const vLen = Math.sqrt(vx * vx + vy * vy + vz * vz);
        if (vLen > 1e-6) {
            vx /= vLen;
            vy /= vLen;
            vz /= vLen;
        } else {
            vx = 0;
            vy = 0;
            vz = -1;
        }

        const nextX = current.x + vx * stepMm;
        const nextY = current.y + vy * stepMm;
        const nextZ = current.z + vz * stepMm;

        const dx = nextX - startPos.x;
        const dy = nextY - startPos.y;
        const lateralDist = Math.sqrt(dx * dx + dy * dy);

        if (lateralDist > maxLateral) {
            stagnated = true;
            stagnationPos = { ...current };
            break;
        }

        current = { x: nextX, y: nextY, z: nextZ };
        path.push({ ...current });

        // Progress check: Z progress or lateral escape progress
        let madeProgress = false;
        if (current.z < bestZ - 0.05 * stepMm) {
            bestZ = current.z;
            madeProgress = true;
        }
        if (lateralDist > bestLateral + 0.1 * stepMm) {
            bestLateral = lateralDist;
            madeProgress = true;
        }

        if (madeProgress) {
            stepsWithoutProgress = 0;
        } else {
            stepsWithoutProgress++;
            if (stepsWithoutProgress > STAGNATION_STEPS_LIMIT) {
                stagnated = true;
                stagnationPos = { ...current };
                break;
            }
        }
    }

    if (!reached && !stagnated) {
        stagnated = true;
        stagnationPos = { ...current };
    }

    let finalPath = path;
    if (reached && simplify) {
        finalPath = simplifyPath(path, sdf, clearance);
    }

    return {
        path: finalPath,
        reached,
        stagnated,
        iterations,
        stagnationPos,
    };
}

