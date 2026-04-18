import React, { useMemo } from 'react';
import * as THREE from 'three';
import type { Vec3 } from '../../types';
import type { ShapedContact } from './types';
import { buildShapedContactGeometry } from './shapedContactGeometry';
import { getConeQuaternion } from '../../SupportPrimitives/ContactCone/contactConeUtils';

interface ShapedContactRendererProps {
    shapedContact: ShapedContact;
    /** Actual socket joint position from the segment data */
    socketJointPos?: Vec3;
    /** Actual socket joint diameter (to match funnel bottom to joint sphere) */
    socketDiameter?: number;
    color?: string;
    emissive?: string;
    emissiveIntensity?: number;
    transparent?: boolean;
    opacity?: number;
    raycast?: any;
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
        lengthMm,
        widthMm,
        chamferRadiusMm,
        profile,
        bodyHeightMm,
    } = shapedContact;

    const bodyRadius = (socketDiameter ?? profile.bodyDiameterMm) / 2;

    // The actual socket position — use the real joint position from segment data,
    // fall back to normal-based computation if not provided (e.g. preview)
    const socketPos = useMemo(() => {
        if (socketJointPos) return socketJointPos;
        return {
            x: pos.x + normal.x * bodyHeightMm,
            y: pos.y + normal.y * bodyHeightMm,
            z: pos.z + normal.z * bodyHeightMm,
        };
    }, [socketJointPos, pos, normal, bodyHeightMm]);

    // Where the normal-based bottom would be
    const normalBottom = useMemo(() => new THREE.Vector3(
        pos.x + normal.x * bodyHeightMm,
        pos.y + normal.y * bodyHeightMm,
        pos.z + normal.z * bodyHeightMm,
    ), [pos, normal, bodyHeightMm]);

    // World-space offset from normal bottom to actual socket
    const worldOffset = useMemo(() => new THREE.Vector3(
        socketPos.x - normalBottom.x,
        socketPos.y - normalBottom.y,
        socketPos.z - normalBottom.z,
    ), [socketPos, normalBottom]);

    // Orientation: Y along cone axis (normal), X twisted to align with A→B
    const quaternion = useMemo(() => {
        const baseQuat = getConeQuaternion(normal);
        const currentLocalX = new THREE.Vector3(1, 0, 0).applyQuaternion(baseQuat);
        const coneAxis = new THREE.Vector3(-normal.x, -normal.y, -normal.z).normalize();

        const abDir = new THREE.Vector3(
            points.pointB.x - points.pointA.x,
            points.pointB.y - points.pointA.y,
            points.pointB.z - points.pointA.z,
        );
        const desiredX = abDir.clone().addScaledVector(coneAxis, -abDir.dot(coneAxis));
        if (desiredX.lengthSq() < 0.000001) return baseQuat;
        desiredX.normalize();

        const currentXProj = currentLocalX.clone().addScaledVector(coneAxis, -currentLocalX.dot(coneAxis));
        if (currentXProj.lengthSq() < 0.000001) return baseQuat;
        currentXProj.normalize();

        let angle = Math.acos(Math.max(-1, Math.min(1, currentXProj.dot(desiredX))));
        const cross = new THREE.Vector3().crossVectors(currentXProj, desiredX);
        if (cross.dot(coneAxis) < 0) angle = -angle;

        const twistQuat = new THREE.Quaternion().setFromAxisAngle(coneAxis, angle);
        return twistQuat.multiply(baseQuat);
    }, [normal, points.pointA, points.pointB]);

    // Build the loft geometry with bottom offset to reach the socket joint
    const geometry = useMemo(() => {
        const invQuat = quaternion.clone().invert();
        const localOffset = worldOffset.clone().applyQuaternion(invQuat);

        return buildShapedContactGeometry({
            rectWidth: lengthMm,
            rectHeight: widthMm,
            chamferRadius: chamferRadiusMm,
            bottomRadius: bodyRadius,
            height: bodyHeightMm,
            bottomOffset: { x: localOffset.x, y: localOffset.y, z: localOffset.z },
        });
    }, [lengthMm, widthMm, chamferRadiusMm, bodyRadius, bodyHeightMm, worldOffset, quaternion]);

    // Center position: midpoint along the normal (same as original)
    const center = useMemo(() => ({
        x: pos.x + normal.x * (bodyHeightMm / 2),
        y: pos.y + normal.y * (bodyHeightMm / 2),
        z: pos.z + normal.z * (bodyHeightMm / 2),
    }), [pos, normal, bodyHeightMm]);

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

            {/* Socket joint sphere */}
            <group position={[socketPos.x, socketPos.y, socketPos.z]}>
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
