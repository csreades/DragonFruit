import React, { useMemo } from 'react';
import * as THREE from 'three';
import type { Vec3 } from '../../types';
import type { ShapedContact } from './types';
import { buildShapedContactGeometry } from './shapedContactGeometry';

interface ShapedContactRendererProps {
    shapedContact: ShapedContact;
    /** Actual socket joint position from the segment data */
    socketJointPos?: Vec3;
    /** Actual socket joint diameter */
    socketDiameter?: number;
    color?: string;
    emissive?: string;
    emissiveIntensity?: number;
    transparent?: boolean;
    opacity?: number;
    raycast?: any;
}

/**
 * Builds a funnel geometry in local space that transitions from a circle
 * of topRadius at Y=+height/2 to a circle of bottomRadius at Y=-height/2.
 * Optionally skewed via bottomOffset so the bottom ring shifts to reach
 * the actual socket position.
 */
function buildFunnelGeometry(
    topRadius: number,
    bottomRadius: number,
    height: number,
    bottomOffset: { x: number; y: number; z: number },
    rings = 12,
    segments = 24,
): THREE.BufferGeometry {
    const positions: number[] = [];
    const normals: number[] = [];
    const indices: number[] = [];

    for (let ring = 0; ring <= rings; ring++) {
        const t = ring / rings;
        const y = (height / 2) - t * height;
        const radius = topRadius + (bottomRadius - topRadius) * t;
        const ox = bottomOffset.x * t;
        const oy = bottomOffset.y * t;
        const oz = bottomOffset.z * t;

        for (let s = 0; s < segments; s++) {
            const angle = (s / segments) * Math.PI * 2;
            const px = radius * Math.cos(angle) + ox;
            const py = y + oy;
            const pz = radius * Math.sin(angle) + oz;
            positions.push(px, py, pz);

            const nx = Math.cos(angle);
            const nz = Math.sin(angle);
            normals.push(nx, 0, nz);
        }
    }

    for (let ring = 0; ring < rings; ring++) {
        for (let s = 0; s < segments; s++) {
            const curr = ring * segments + s;
            const next = ring * segments + ((s + 1) % segments);
            const currBelow = (ring + 1) * segments + s;
            const nextBelow = (ring + 1) * segments + ((s + 1) % segments);
            indices.push(curr, currBelow, next);
            indices.push(next, currBelow, nextBelow);
        }
    }

    // Top cap
    const topIdx = positions.length / 3;
    positions.push(0, height / 2, 0);
    normals.push(0, 1, 0);
    for (let s = 0; s < segments; s++) {
        indices.push(topIdx, (s + 1) % segments, s);
    }

    // Bottom cap
    const bottomIdx = positions.length / 3;
    positions.push(bottomOffset.x, -height / 2 + bottomOffset.y, bottomOffset.z);
    normals.push(0, -1, 0);
    const bottomStart = rings * segments;
    for (let s = 0; s < segments; s++) {
        indices.push(bottomIdx, bottomStart + s, bottomStart + ((s + 1) % segments));
    }

    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
    geo.setAttribute('normal', new THREE.Float32BufferAttribute(normals, 3));
    geo.setIndex(indices);
    geo.computeVertexNormals();
    return geo;
}

export function ShapedContactRenderer({
    shapedContact,
    socketJointPos,
    socketDiameter,
    color = '#c8752a',
    emissive = '#000000',
    emissiveIntensity = 0,
    transparent = false,
    opacity = 1,
    raycast,
}: ShapedContactRendererProps) {
    const {
        pos,
        normal,
        points,
        widthMm,
        profile,
        bodyHeightMm,
        surfaceSamples,
    } = shapedContact;

    const contactRadius = widthMm / 2;
    const socketRadius = (socketDiameter ?? profile.bodyDiameterMm) / 2;

    // Socket position from data or fallback
    const socketPos = useMemo(() => {
        if (socketJointPos) return socketJointPos;
        return {
            x: pos.x + normal.x * bodyHeightMm,
            y: pos.y + normal.y * bodyHeightMm,
            z: pos.z + normal.z * bodyHeightMm,
        };
    }, [socketJointPos, pos, normal, bodyHeightMm]);

    // --- TUBE: segmented cylinders following surface samples, or straight A→B ---
    const tubeSegments = useMemo(() => {
        // Build list of points the tube passes through
        const pts: { x: number; y: number; z: number }[] = [];

        if (surfaceSamples && surfaceSamples.length >= 2) {
            for (const s of surfaceSamples) {
                pts.push(s.pos);
            }
        } else {
            pts.push(points.pointA);
            pts.push(points.pointB);
        }

        // Build cylinder segments between consecutive points
        const segs: { center: THREE.Vector3; length: number; quaternion: THREE.Quaternion }[] = [];
        for (let i = 0; i < pts.length - 1; i++) {
            const a = new THREE.Vector3(pts[i].x, pts[i].y, pts[i].z);
            const b = new THREE.Vector3(pts[i + 1].x, pts[i + 1].y, pts[i + 1].z);
            const dir = b.clone().sub(a);
            const len = dir.length();
            if (len < 0.001) continue;
            dir.normalize();

            segs.push({
                center: a.clone().add(b).multiplyScalar(0.5),
                length: len,
                quaternion: new THREE.Quaternion().setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir),
            });
        }
        return segs;
    }, [surfaceSamples, points.pointA, points.pointB]);

    // Endpoints for hemisphere caps (use surface samples if available)
    const capA = useMemo(() => {
        if (surfaceSamples && surfaceSamples.length >= 1) return surfaceSamples[0].pos;
        return points.pointA;
    }, [surfaceSamples, points.pointA]);

    const capB = useMemo(() => {
        if (surfaceSamples && surfaceSamples.length >= 1) return surfaceSamples[surfaceSamples.length - 1].pos;
        return points.pointB;
    }, [surfaceSamples, points.pointB]);

    // Tube half-span (distance from midpoint to either end)
    const tubeHalfSpan = useMemo(() => {
        if (surfaceSamples && surfaceSamples.length >= 2) {
            const first = surfaceSamples[0].pos;
            const last = surfaceSamples[surfaceSamples.length - 1].pos;
            const dx = last.x - first.x;
            const dy = last.y - first.y;
            const dz = last.z - first.z;
            return Math.sqrt(dx * dx + dy * dy + dz * dz) / 2;
        }
        const dx = points.pointB.x - points.pointA.x;
        const dy = points.pointB.y - points.pointA.y;
        const dz = points.pointB.z - points.pointA.z;
        return Math.sqrt(dx * dx + dy * dy + dz * dz) / 2;
    }, [surfaceSamples, points.pointA, points.pointB]);

    // --- FUNNEL: original rounded-rect-to-circle loft from tube midpoint to socket ---
    const funnelTop = useMemo(() => {
        if (surfaceSamples && surfaceSamples.length >= 2) {
            const mid = surfaceSamples[Math.floor(surfaceSamples.length / 2)];
            return new THREE.Vector3(mid.pos.x, mid.pos.y, mid.pos.z);
        }
        return new THREE.Vector3(pos.x, pos.y, pos.z);
    }, [surfaceSamples, pos]);

    const funnelBottom = useMemo(() =>
        new THREE.Vector3(socketPos.x, socketPos.y, socketPos.z),
    [socketPos]);

    // Funnel axis direction and length
    const funnelDir = useMemo(() => {
        const d = funnelBottom.clone().sub(funnelTop);
        if (d.lengthSq() < 0.0001) d.set(0, 0, -1);
        return d;
    }, [funnelTop, funnelBottom]);

    const funnelLength = useMemo(() => funnelDir.length(), [funnelDir]);

    const funnelCenter = useMemo(() =>
        funnelTop.clone().add(funnelBottom).multiplyScalar(0.5),
    [funnelTop, funnelBottom]);

    // Funnel orientation: align local +Y with the A→B direction (for the rect long axis),
    // and the loft axis (local Y in the geometry = top-to-bottom) with the funnel direction.
    // The geometry's loft axis is Y. We need Y → funnelDir direction.
    // And the geometry's X axis (rect long axis) should align with the tube (A→B).
    const funnelQuaternion = useMemo(() => {
        const funnelAxis = funnelDir.clone().normalize();
        // getConeQuaternion(n) aligns Y with -n. We want Y aligned with -funnelAxis
        // (top of loft = +Y = toward tube, away from socket).
        // So pass funnelAxis directly.
        const baseQuat = new THREE.Quaternion().setFromUnitVectors(
            new THREE.Vector3(0, 1, 0),
            funnelAxis.clone().negate(), // +Y points opposite to funnel direction (toward tube)
        );

        // Twist so X aligns with A→B direction
        const currentX = new THREE.Vector3(1, 0, 0).applyQuaternion(baseQuat);
        const abDir = new THREE.Vector3(
            points.pointB.x - points.pointA.x,
            points.pointB.y - points.pointA.y,
            points.pointB.z - points.pointA.z,
        );
        // Project A→B onto plane perpendicular to funnel axis
        const desiredX = abDir.clone().addScaledVector(funnelAxis, -abDir.dot(funnelAxis));
        if (desiredX.lengthSq() < 0.0001) return baseQuat;
        desiredX.normalize();

        const currentXProj = currentX.clone().addScaledVector(funnelAxis, -currentX.dot(funnelAxis));
        if (currentXProj.lengthSq() < 0.0001) return baseQuat;
        currentXProj.normalize();

        let angle = Math.acos(Math.max(-1, Math.min(1, currentXProj.dot(desiredX))));
        const cross = new THREE.Vector3().crossVectors(currentXProj, desiredX);
        if (cross.dot(funnelAxis) < 0) angle = -angle;

        const twistQuat = new THREE.Quaternion().setFromAxisAngle(funnelAxis, angle);
        return twistQuat.multiply(baseQuat);
    }, [funnelDir, points.pointA, points.pointB]);

    // Funnel geometry: single solid manifold mesh.
    // Each ring has the same vertex count. Top ring traces the tube outline
    // (stadium shape following the spine). Bottom ring is a circle at the socket.
    const funnelGeometry = useMemo(() => {
        const pts = surfaceSamples
            ? surfaceSamples.map(s => new THREE.Vector3(s.pos.x, s.pos.y, s.pos.z))
            : [
                new THREE.Vector3(points.pointA.x, points.pointA.y, points.pointA.z),
                new THREE.Vector3(points.pointB.x, points.pointB.y, points.pointB.z),
            ];

        if (pts.length < 2) return new THREE.BufferGeometry();

        const socket = new THREE.Vector3(socketPos.x, socketPos.y, socketPos.z);
        const transitionRings = 12;

        // Build the top ring: a stadium/capsule outline around the tube's spine.
        // One side traces along the spine offset by +contactRadius,
        // the other side traces in reverse offset by -contactRadius,
        // with semicircle caps at each end.
        const spineDir = pts[pts.length - 1].clone().sub(pts[0]).normalize();

        // Compute a consistent "side" direction perpendicular to spine
        // and pointing away from the socket (outward)
        const toSocket = socket.clone().sub(pts[Math.floor(pts.length / 2)]).normalize();
        let sideDir = new THREE.Vector3().crossVectors(spineDir, toSocket);
        if (sideDir.lengthSq() < 0.0001) {
            sideDir = new THREE.Vector3().crossVectors(spineDir, new THREE.Vector3(0, 0, 1));
        }
        sideDir.normalize();

        const capSegments = 8; // Vertices per semicircle cap
        const vertsPerRing = pts.length * 2 + capSegments * 2;

        const positions: number[] = [];
        const normals: number[] = [];
        const indices: number[] = [];

        // Generate rings from top (t=0, tube outline) to bottom (t=1, circle at socket)
        for (let ring = 0; ring <= transitionRings; ring++) {
            const t = ring / transitionRings;
            const radius = contactRadius + (socketRadius - contactRadius) * t;
            const ringStartIdx = positions.length / 3;

            // At t=0: trace the tube outline. At t=1: circle at socket.
            // Intermediate: lerp each point toward socket, shrink spread.

            const ringVerts: THREE.Vector3[] = [];
            const ringNorms: THREE.Vector3[] = [];

            // Side A: forward along spine, offset by +sideDir * radius
            for (let si = 0; si < pts.length; si++) {
                const center = pts[si].clone().lerp(socket, t);
                const p = center.clone().addScaledVector(sideDir, radius);
                ringVerts.push(p);
                ringNorms.push(sideDir.clone());
            }

            // Cap at end B: semicircle from +sideDir to -sideDir around the last spine point
            const endCenter = pts[pts.length - 1].clone().lerp(socket, t);
            const capAxisEnd = spineDir.clone(); // Cap curves around the spine end
            for (let ci = 1; ci < capSegments; ci++) {
                const angle = (ci / capSegments) * Math.PI; // 0 to PI
                const p = endCenter.clone()
                    .addScaledVector(sideDir, radius * Math.cos(angle))
                    .addScaledVector(capAxisEnd, radius * Math.sin(angle));
                ringVerts.push(p);
                const norm = p.clone().sub(endCenter).normalize();
                ringNorms.push(norm);
            }

            // Side B: backward along spine, offset by -sideDir * radius
            for (let si = pts.length - 1; si >= 0; si--) {
                const center = pts[si].clone().lerp(socket, t);
                const p = center.clone().addScaledVector(sideDir, -radius);
                ringVerts.push(p);
                ringNorms.push(sideDir.clone().negate());
            }

            // Cap at end A: semicircle from -sideDir to +sideDir around the first spine point
            const startCenter = pts[0].clone().lerp(socket, t);
            const capAxisStart = spineDir.clone().negate();
            for (let ci = 1; ci < capSegments; ci++) {
                const angle = (ci / capSegments) * Math.PI;
                const p = startCenter.clone()
                    .addScaledVector(sideDir, -radius * Math.cos(angle))
                    .addScaledVector(capAxisStart, radius * Math.sin(angle));
                ringVerts.push(p);
                const norm = p.clone().sub(startCenter).normalize();
                ringNorms.push(norm);
            }

            // Emit vertices
            for (let i = 0; i < ringVerts.length; i++) {
                positions.push(ringVerts[i].x, ringVerts[i].y, ringVerts[i].z);
                normals.push(ringNorms[i].x, ringNorms[i].y, ringNorms[i].z);
            }

            const actualVertsThisRing = ringVerts.length;

            // Connect to previous ring
            if (ring > 0) {
                const prevStart = ringStartIdx - actualVertsThisRing;
                for (let v = 0; v < actualVertsThisRing; v++) {
                    const nextV = (v + 1) % actualVertsThisRing;
                    const prev = prevStart + v;
                    const prevN = prevStart + nextV;
                    const curr = ringStartIdx + v;
                    const currN = ringStartIdx + nextV;
                    indices.push(prev, curr, prevN);
                    indices.push(prevN, curr, currN);
                }
            }
        }

        // Bottom cap
        const lastRingStart = positions.length / 3 - vertsPerRing;
        const bottomIdx = positions.length / 3;
        positions.push(socket.x, socket.y, socket.z);
        const downDir = pts[Math.floor(pts.length / 2)].clone().sub(socket).negate().normalize();
        normals.push(downDir.x, downDir.y, downDir.z);
        for (let v = 0; v < vertsPerRing; v++) {
            const nextV = (v + 1) % vertsPerRing;
            indices.push(bottomIdx, lastRingStart + v, lastRingStart + nextV);
        }

        const geo = new THREE.BufferGeometry();
        geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
        geo.setAttribute('normal', new THREE.Float32BufferAttribute(normals, 3));
        geo.setIndex(indices);
        geo.computeVertexNormals();
        return geo;
    }, [surfaceSamples, points.pointA, points.pointB, contactRadius, socketRadius, socketPos]);

    const materialProps = {
        color,
        emissive,
        emissiveIntensity,
        transparent,
        opacity,
        depthWrite: !transparent,
    };

    return (
        <group>
            {/* Tube: segmented cylinders following the surface with sphere joints */}
            {tubeSegments.map((seg, i) => (
                <group key={`tube-${i}`} position={[seg.center.x, seg.center.y, seg.center.z]} quaternion={seg.quaternion}>
                    <mesh raycast={raycast}>
                        <cylinderGeometry args={[contactRadius, contactRadius, seg.length, 16]} />
                        <meshStandardMaterial {...materialProps} color="#00aaff" />
                    </mesh>
                </group>
            ))}

            {/* Sphere joints at each surface sample point (fills gaps between tube segments) */}
            {surfaceSamples && surfaceSamples.map((s, i) => (
                <group key={`joint-${i}`} position={[s.pos.x, s.pos.y, s.pos.z]}>
                    <mesh raycast={raycast}>
                        <sphereGeometry args={[contactRadius, 12, 8]} />
                        <meshStandardMaterial {...materialProps} color="#00aaff" />
                    </mesh>
                </group>
            ))}

            {/* Hemisphere cap at point A (fallback if no surface samples) */}
            {!surfaceSamples && (
                <group position={[capA.x, capA.y, capA.z]}>
                    <mesh raycast={raycast}>
                        <sphereGeometry args={[contactRadius, 16, 12]} />
                        <meshStandardMaterial {...materialProps} />
                    </mesh>
                </group>
            )}

            {/* Hemisphere cap at point B (fallback if no surface samples) */}
            {!surfaceSamples && (
                <group position={[capB.x, capB.y, capB.z]}>
                    <mesh raycast={raycast}>
                        <sphereGeometry args={[contactRadius, 16, 12]} />
                        <meshStandardMaterial {...materialProps} />
                    </mesh>
                </group>
            )}

            {/* Funnel: world-space geometry matching tube at top, socket at bottom */}
            <mesh raycast={raycast} geometry={funnelGeometry}>
                <meshStandardMaterial {...materialProps} side={THREE.DoubleSide} />
            </mesh>

            {/* Socket joint sphere */}
            <group position={[socketPos.x, socketPos.y, socketPos.z]}>
                <mesh raycast={raycast}>
                    <sphereGeometry args={[socketRadius, 16, 12]} />
                    <meshStandardMaterial
                        color="#888888"
                        emissive={emissive}
                        emissiveIntensity={emissiveIntensity}
                        transparent={transparent}
                        opacity={opacity}
                        depthWrite={!transparent}
                    />
                </mesh>
            </group>
        </group>
    );
}
