import React, { useMemo } from 'react';
import * as THREE from 'three';
import type { ShapedContact } from './types';
import { buildShapedContactGeometry } from './shapedContactGeometry';
import { getConeQuaternion } from '../../SupportPrimitives/ContactCone/contactConeUtils';

interface ShapedContactRendererProps {
    shapedContact: ShapedContact;
    color?: string;
    emissive?: string;
    emissiveIntensity?: number;
    transparent?: boolean;
    opacity?: number;
    raycast?: any;
}

/**
 * Computes a quaternion that:
 * 1. Aligns the loft Y-axis with the cone axis (via getConeQuaternion)
 * 2. Twists around that axis so the loft X-axis aligns with the A→B direction
 */
function computeShapedContactQuaternion(
    normal: { x: number; y: number; z: number },
    pointA: { x: number; y: number; z: number },
    pointB: { x: number; y: number; z: number },
): THREE.Quaternion {
    // Step 1: Base rotation — align local Y with cone axis (same as ContactCone)
    const baseQuat = getConeQuaternion(normal);

    // Step 2: Figure out what direction the local X-axis currently points in world space
    const currentLocalX = new THREE.Vector3(1, 0, 0).applyQuaternion(baseQuat);

    // Step 3: The desired local X direction is A→B projected onto the contact plane
    const coneAxis = new THREE.Vector3(-normal.x, -normal.y, -normal.z).normalize();
    const abDir = new THREE.Vector3(
        pointB.x - pointA.x,
        pointB.y - pointA.y,
        pointB.z - pointA.z,
    );
    // Project onto plane perpendicular to cone axis
    const desiredX = abDir.clone().addScaledVector(coneAxis, -abDir.dot(coneAxis));
    if (desiredX.lengthSq() < 0.000001) {
        return baseQuat; // Points are on same spot or along normal — no twist needed
    }
    desiredX.normalize();

    // Step 4: Compute twist angle between currentLocalX and desiredX around coneAxis
    // Project both onto the plane perpendicular to coneAxis (currentLocalX already is, but normalize)
    const currentXProj = currentLocalX.clone().addScaledVector(coneAxis, -currentLocalX.dot(coneAxis)).normalize();

    let angle = Math.acos(Math.max(-1, Math.min(1, currentXProj.dot(desiredX))));
    // Determine sign via cross product
    const cross = new THREE.Vector3().crossVectors(currentXProj, desiredX);
    if (cross.dot(coneAxis) < 0) angle = -angle;

    // Step 5: Apply twist rotation around the cone axis
    const twistQuat = new THREE.Quaternion().setFromAxisAngle(coneAxis, angle);
    return twistQuat.multiply(baseQuat);
}

export function ShapedContactRenderer({
    shapedContact,
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
        lengthMm,
        widthMm,
        chamferRadiusMm,
        profile,
        bodyHeightMm,
    } = shapedContact;

    const bodyRadius = profile.bodyDiameterMm / 2;

    // Build the loft geometry
    const geometry = useMemo(() => {
        return buildShapedContactGeometry({
            rectWidth: lengthMm,
            rectHeight: widthMm,
            chamferRadius: chamferRadiusMm,
            bottomRadius: bodyRadius,
            height: bodyHeightMm,
        });
    }, [lengthMm, widthMm, chamferRadiusMm, bodyRadius, bodyHeightMm]);

    // Center position of the loft body (midpoint along the axis)
    const center = useMemo(() => ({
        x: pos.x + normal.x * (bodyHeightMm / 2),
        y: pos.y + normal.y * (bodyHeightMm / 2),
        z: pos.z + normal.z * (bodyHeightMm / 2),
    }), [pos, normal, bodyHeightMm]);

    // Full orientation: Y along cone axis, X twisted to align with A→B
    const quaternion = useMemo(
        () => computeShapedContactQuaternion(normal, points.pointA, points.pointB),
        [normal, points.pointA, points.pointB],
    );

    return (
        <group>
            {/* Loft body */}
            <group
                position={[center.x, center.y, center.z]}
                quaternion={quaternion}
            >
                <mesh raycast={raycast} geometry={geometry}>
                    <meshStandardMaterial
                        color={color}
                        emissive={emissive}
                        emissiveIntensity={emissiveIntensity}
                        transparent={transparent}
                        opacity={opacity}
                        depthWrite={!transparent}
                        side={THREE.DoubleSide}
                    />
                </mesh>
            </group>

            {/* Socket joint sphere at the bottom of the loft */}
            <group position={[
                pos.x + normal.x * bodyHeightMm,
                pos.y + normal.y * bodyHeightMm,
                pos.z + normal.z * bodyHeightMm,
            ]}>
                <mesh raycast={raycast}>
                    <sphereGeometry args={[bodyRadius, 16, 12]} />
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
