import React, { useMemo } from 'react';
import * as THREE from 'three';
import type { Vec3 } from '../../types';
import type { ShapedPlacementPreview } from './useShapedSupportPlacement';
import { buildShapedSupportData } from './shapedSupportBuilder';
import { RootsRenderer } from '../../SupportPrimitives/Roots/RootsRenderer';
import { ShaftRenderer } from '../../SupportPrimitives/Shaft/ShaftRenderer';
import { ShapedContactRenderer } from './ShapedContactRenderer';

interface ShapedSupportPreviewProps {
    preview: ShapedPlacementPreview;
}

/**
 * Renders a transparent ghost preview of a shaped support
 * while the user is dragging from point A to the cursor (point B).
 */
export function ShapedSupportPreview({ preview }: ShapedSupportPreviewProps) {
    const buildResult = useMemo(() => {
        try {
            return buildShapedSupportData({
                pointA: preview.pointA,
                normalA: preview.normalA,
                pointB: preview.pointB,
                normalB: preview.normalB,
                modelId: 'preview',
                mesh: preview.mesh,
            });
        } catch {
            return null;
        }
    }, [preview.pointA, preview.normalA, preview.pointB, preview.normalB, preview.mesh]);

    if (!buildResult) return null;

    const { root, shapedSupport } = buildResult;
    const shaftDiameter = shapedSupport.segments[0]?.diameter ?? 1.2;
    const rootTopZ = root.transform.pos.z + root.diskHeight + root.coneHeight;

    return (
        <group>
            {/* Roots */}
            <RootsRenderer
                root={root}
                shaftDiameter={shaftDiameter}
                color="#4a9eff"
                emissive="#000000"
                emissiveIntensity={0}
                transparent
                opacity={0.4}
                radialSegments={12}
                sphereSegments={12}
            />

            {/* Shafts */}
            {shapedSupport.segments.map((segment, index) => {
                const startPos = index === 0
                    ? { x: root.transform.pos.x, y: root.transform.pos.y, z: rootTopZ }
                    : segment.bottomJoint?.pos ?? { x: 0, y: 0, z: 0 };
                const endPos = segment.topJoint?.pos ?? { x: 0, y: 0, z: 0 };

                return (
                    <ShaftRenderer
                        key={segment.id}
                        id={segment.id}
                        start={startPos}
                        end={endPos}
                        diameter={segment.diameter}
                        color="#4a9eff"
                        transparent
                        opacity={0.4}
                        radialSegments={12}
                        isInteractable={false}
                    />
                );
            })}

            {/* Shaped contact */}
            <ShapedContactRenderer
                shapedContact={shapedSupport.shapedContact}
                socketJointPos={shapedSupport.segments[shapedSupport.segments.length - 1]?.topJoint?.pos}
                color="#4a9eff"
                transparent
                opacity={0.4}
            />

            {/* Point A marker */}
            <group position={[preview.pointA.x, preview.pointA.y, preview.pointA.z]}>
                <mesh>
                    <sphereGeometry args={[0.3, 12, 8]} />
                    <meshStandardMaterial color="#00ff88" emissive="#00ff88" emissiveIntensity={0.5} />
                </mesh>
            </group>

            {/* Point B marker (cursor) */}
            <group position={[preview.pointB.x, preview.pointB.y, preview.pointB.z]}>
                <mesh>
                    <sphereGeometry args={[0.3, 12, 8]} />
                    <meshStandardMaterial color="#ff8800" emissive="#ff8800" emissiveIntensity={0.5} />
                </mesh>
            </group>
        </group>
    );
}
