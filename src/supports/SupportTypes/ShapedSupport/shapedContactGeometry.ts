import * as THREE from 'three';

/**
 * Generates a rounded-rectangle cross-section as an array of 2D points.
 *
 * @param width  - Full width (long axis)
 * @param height - Full height (short axis)
 * @param chamferRadius - Corner radius
 * @param segments - Number of vertices per quarter-circle chamfer
 * @returns Array of {x, y} points tracing the rounded rectangle CCW
 */
function roundedRectPoints(
    width: number,
    height: number,
    chamferRadius: number,
    segments: number,
): { x: number; y: number }[] {
    const hw = width / 2;
    const hh = height / 2;
    // Clamp chamfer so it doesn't exceed the smaller half-dimension
    const r = Math.min(chamferRadius, hw, hh);
    const pts: { x: number; y: number }[] = [];

    // Four corners, each with an arc
    // Corner order: top-right, top-left, bottom-left, bottom-right
    const corners = [
        { cx: hw - r, cy: hh - r, startAngle: 0 },
        { cx: -(hw - r), cy: hh - r, startAngle: Math.PI / 2 },
        { cx: -(hw - r), cy: -(hh - r), startAngle: Math.PI },
        { cx: hw - r, cy: -(hh - r), startAngle: (3 * Math.PI) / 2 },
    ];

    for (const corner of corners) {
        for (let i = 0; i <= segments; i++) {
            const angle = corner.startAngle + (i / segments) * (Math.PI / 2);
            pts.push({
                x: corner.cx + r * Math.cos(angle),
                y: corner.cy + r * Math.sin(angle),
            });
        }
    }

    return pts;
}

/**
 * Generates a circle cross-section as an array of 2D points.
 *
 * @param radius - Circle radius
 * @param count  - Number of vertices
 * @returns Array of {x, y} points tracing the circle CCW
 */
function circlePoints(radius: number, count: number): { x: number; y: number }[] {
    const pts: { x: number; y: number }[] = [];
    for (let i = 0; i < count; i++) {
        const angle = (i / count) * Math.PI * 2;
        pts.push({
            x: radius * Math.cos(angle),
            y: radius * Math.sin(angle),
        });
    }
    return pts;
}

/**
 * Interpolates between a rounded-rectangle cross-section and a circle.
 * At t=0, the shape is the rounded rectangle. At t=1, it's a perfect circle.
 *
 * Both shapes must have the same vertex count for the loft to work.
 * We parameterize the rounded rect so that as t→1, chamfer radius grows
 * and width/height converge to diameter, producing a circle.
 */
function interpolatedCrossSection(
    t: number,
    rectWidth: number,
    rectHeight: number,
    chamferRadius: number,
    circleRadius: number,
    verticesPerSection: number,
    chamferSegments: number,
): { x: number; y: number }[] {
    // Interpolate dimensions
    const targetDiameter = circleRadius * 2;
    const w = rectWidth + (targetDiameter - rectWidth) * t;
    const h = rectHeight + (targetDiameter - rectHeight) * t;
    // Chamfer grows toward half the smaller dimension (full circle)
    const maxChamfer = Math.min(w, h) / 2;
    const r = chamferRadius + (maxChamfer - chamferRadius) * t;

    const rectPts = roundedRectPoints(w, h, r, chamferSegments);

    // Resample to exactly verticesPerSection points via linear interpolation
    return resampleClosed(rectPts, verticesPerSection);
}

/**
 * Resample a closed polygon to exactly `count` evenly-spaced points.
 */
function resampleClosed(
    pts: { x: number; y: number }[],
    count: number,
): { x: number; y: number }[] {
    const n = pts.length;
    // Compute cumulative arc lengths
    const lengths: number[] = [0];
    let total = 0;
    for (let i = 1; i <= n; i++) {
        const prev = pts[(i - 1) % n];
        const curr = pts[i % n];
        total += Math.sqrt((curr.x - prev.x) ** 2 + (curr.y - prev.y) ** 2);
        lengths.push(total);
    }

    const result: { x: number; y: number }[] = [];
    for (let i = 0; i < count; i++) {
        const targetLen = (i / count) * total;
        // Find segment
        let seg = 0;
        for (seg = 0; seg < n; seg++) {
            if (lengths[seg + 1] >= targetLen) break;
        }
        const segLen = lengths[seg + 1] - lengths[seg];
        const frac = segLen > 0 ? (targetLen - lengths[seg]) / segLen : 0;
        const a = pts[seg % n];
        const b = pts[(seg + 1) % n];
        result.push({
            x: a.x + (b.x - a.x) * frac,
            y: a.y + (b.y - a.y) * frac,
        });
    }
    return result;
}

export interface ShapedContactLoftParams {
    /** Long-axis width of the contact rectangle (mm) */
    rectWidth: number;
    /** Short-axis height of the contact rectangle (mm) */
    rectHeight: number;
    /** Corner chamfer radius (mm) */
    chamferRadius: number;
    /** Circle radius at the bottom (socket joint end) (mm) */
    bottomRadius: number;
    /** Total loft height from contact face to socket (mm) */
    height: number;
    /** Number of cross-section rings along the loft */
    rings?: number;
    /** Number of vertices per ring */
    verticesPerRing?: number;
    /** Chamfer resolution (vertices per corner arc) */
    chamferSegments?: number;
    /** Offset of the bottom ring center from its default position (local space).
     *  Used to skew the funnel so it reaches the actual socket joint. */
    bottomOffset?: { x: number; y: number; z: number };
    /**
     * Per-vertex Y deformation along the X axis (local space).
     * Each entry maps a local X position to a Y offset.
     * Used to curve the top of the funnel to follow the tube's surface path.
     * Deformation fades from full at t=0 (top) to zero by t=0.3.
     */
    surfaceOffsets?: { localX: number; offsetY: number }[];
}

/**
 * Builds a THREE.BufferGeometry for the shaped contact loft body.
 *
 * The geometry transitions from a rounded rectangle at the top (model contact)
 * to a circle at the bottom (socket joint). The loft axis is +Y (top) to -Y (bottom),
 * matching three.js cylinder convention for easy rotation via quaternion.
 */
export function buildShapedContactGeometry(params: ShapedContactLoftParams): THREE.BufferGeometry {
    const {
        rectWidth,
        rectHeight,
        chamferRadius,
        bottomRadius,
        height,
        rings = 16,
        verticesPerRing = 32,
        chamferSegments = 8,
        bottomOffset = { x: 0, y: 0, z: 0 },
        surfaceOffsets,
    } = params;

    // Build surface offset interpolation function
    let surfaceOffsetFn: ((localX: number) => number) | null = null;
    if (surfaceOffsets && surfaceOffsets.length >= 2) {
        const sorted = [...surfaceOffsets].sort((a, b) => a.localX - b.localX);
        surfaceOffsetFn = (lx: number) => {
            if (lx <= sorted[0].localX) return sorted[0].offsetY;
            if (lx >= sorted[sorted.length - 1].localX) return sorted[sorted.length - 1].offsetY;
            for (let i = 0; i < sorted.length - 1; i++) {
                if (lx >= sorted[i].localX && lx <= sorted[i + 1].localX) {
                    const frac = (lx - sorted[i].localX) / (sorted[i + 1].localX - sorted[i].localX);
                    return sorted[i].offsetY + (sorted[i + 1].offsetY - sorted[i].offsetY) * frac;
                }
            }
            return 0;
        };
    }

    const positions: number[] = [];
    const normals: number[] = [];
    const indices: number[] = [];

    // Generate cross-section rings from top (t=0, rounded rect) to bottom (t=1, circle)
    const sections: { x: number; y: number }[][] = [];

    for (let ring = 0; ring <= rings; ring++) {
        const t = ring / rings;
        const y = (height / 2) - t * height; // top = +height/2, bottom = -height/2
        const section = interpolatedCrossSection(
            t,
            rectWidth,
            rectHeight,
            chamferRadius,
            bottomRadius,
            verticesPerRing,
            chamferSegments,
        );
        sections.push(section);

        // Shift ring center progressively from origin (top) to bottomOffset (bottom)
        const ox = bottomOffset.x * t;
        const oy = bottomOffset.y * t;
        const oz = bottomOffset.z * t;

        // Surface deformation: curve the rings to follow the tube
        // Full deformation at t=0 (top), fades to zero by t=1 (bottom)
        const deformBlend = surfaceOffsetFn ? (1 - t) : 0;

        for (let v = 0; v < verticesPerRing; v++) {
            let py = y + oy;
            if (surfaceOffsetFn && deformBlend > 0) {
                py += surfaceOffsetFn(section[v].x) * deformBlend;
            }
            positions.push(section[v].x + ox, py, section[v].y + oz);
            // Approximate normal: radially outward from center
            const len = Math.sqrt(section[v].x ** 2 + section[v].y ** 2);
            if (len > 0.0001) {
                normals.push(section[v].x / len, 0, section[v].y / len);
            } else {
                normals.push(0, 0, 1);
            }
        }
    }

    // Build triangle strip indices connecting adjacent rings
    for (let ring = 0; ring < rings; ring++) {
        for (let v = 0; v < verticesPerRing; v++) {
            const current = ring * verticesPerRing + v;
            const next = ring * verticesPerRing + ((v + 1) % verticesPerRing);
            const currentBelow = (ring + 1) * verticesPerRing + v;
            const nextBelow = (ring + 1) * verticesPerRing + ((v + 1) % verticesPerRing);

            indices.push(current, currentBelow, next);
            indices.push(next, currentBelow, nextBelow);
        }
    }

    // Cap the top (contact face) — fan from center
    const topCenterIdx = positions.length / 3;
    positions.push(0, height / 2, 0);
    normals.push(0, 1, 0);
    for (let v = 0; v < verticesPerRing; v++) {
        const nextV = (v + 1) % verticesPerRing;
        indices.push(topCenterIdx, nextV, v); // CCW winding looking down +Y
    }

    // Cap the bottom (socket face) — fan from center
    const bottomCenterIdx = positions.length / 3;
    positions.push(bottomOffset.x, -height / 2 + bottomOffset.y, bottomOffset.z);
    normals.push(0, -1, 0);
    const bottomRingStart = rings * verticesPerRing;
    for (let v = 0; v < verticesPerRing; v++) {
        const nextV = (v + 1) % verticesPerRing;
        indices.push(bottomCenterIdx, bottomRingStart + v, bottomRingStart + nextV);
    }

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
    geometry.setAttribute('normal', new THREE.Float32BufferAttribute(normals, 3));
    geometry.setIndex(indices);
    geometry.computeVertexNormals(); // Smooth normals across the loft

    return geometry;
}

// ---------------------------------------------------------------------------
// Surface sampling
// ---------------------------------------------------------------------------

export interface SurfaceSample {
    pos: THREE.Vector3;
    normal: THREE.Vector3;
}

const _raycaster = new THREE.Raycaster();

/**
 * Samples N points along the A→B path on the mesh surface by raycasting.
 * Falls back to linear interpolation if a ray misses.
 */
export function sampleMeshSurface(
    pointA: THREE.Vector3,
    pointB: THREE.Vector3,
    normalA: THREE.Vector3,
    normalB: THREE.Vector3,
    mesh: THREE.Mesh,
    sampleCount: number,
): SurfaceSample[] {
    const samples: SurfaceSample[] = [];
    const normalMatrix = new THREE.Matrix3().getNormalMatrix(mesh.matrixWorld);

    for (let i = 0; i < sampleCount; i++) {
        const t = sampleCount <= 1 ? 0.5 : i / (sampleCount - 1);

        const lerpPos = new THREE.Vector3().lerpVectors(pointA, pointB, t);
        const lerpNormal = new THREE.Vector3().lerpVectors(normalA, normalB, t).normalize();

        // Cast from outside the surface inward
        const castOrigin = lerpPos.clone().addScaledVector(lerpNormal, 2.0);
        const castDir = lerpNormal.clone().negate();

        _raycaster.set(castOrigin, castDir);
        _raycaster.far = 10.0;
        _raycaster.near = 0;

        const hits = _raycaster.intersectObject(mesh, false);

        if (hits.length > 0) {
            const hit = hits[0];
            const hitNormal = hit.face
                ? hit.face.normal.clone().applyNormalMatrix(normalMatrix).normalize()
                : lerpNormal.clone();
            samples.push({ pos: hit.point.clone(), normal: hitNormal });
        } else {
            samples.push({ pos: lerpPos.clone(), normal: lerpNormal.clone() });
        }
    }

    return samples;
}
