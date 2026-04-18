import React, { useMemo } from 'react';
import type { Roots } from '../../types';
import type { ShapedSupport } from './types';
import { RootsRenderer } from '../../SupportPrimitives/Roots/RootsRenderer';
import { ShaftRenderer } from '../../SupportPrimitives/Shaft/ShaftRenderer';
import { JointRenderer } from '../../SupportPrimitives/Joint/JointRenderer';
import { ShapedContactRenderer } from './ShapedContactRenderer';

interface ShapedSupportRendererProps {
    shapedSupport: ShapedSupport;
    root: Roots;
    isSelected?: boolean;
    selectedId?: string | null;
    dimNonSelected?: boolean;
    isHovered?: boolean;
    baseColor?: string;
    suppressHover?: boolean;
    isInteractable?: boolean;
}

const SELECTED_COLOR = '#c11f61';
const HOVER_COLOR = '#efd8c2';
const DIM_EMISSIVE = '#000000';
const DIM_EMISSIVE_INTENSITY = 0;

export const ShapedSupportRenderer = React.memo(function ShapedSupportRenderer({
    shapedSupport,
    root,
    isSelected = false,
    selectedId = null,
    dimNonSelected = false,
    isHovered = false,
    baseColor = '#c8752a',
    suppressHover = false,
    isInteractable = true,
}: ShapedSupportRendererProps) {
    const shaftDiameter = shapedSupport.segments[0]?.diameter ?? 1.2;

    // LOD: low detail when not selected/hovered
    const useLowDetail = !isSelected && !isHovered;
    const radialSegments = useLowDetail ? 8 : 24;

    // Visuals
    const color = dimNonSelected && !isSelected ? '#666666' : baseColor;
    const emissive = isHovered && !suppressHover ? HOVER_COLOR : DIM_EMISSIVE;
    const emissiveIntensity = isHovered && !suppressHover ? 0.16 : DIM_EMISSIVE_INTENSITY;

    // Build shaft elements from segments
    const shafts = useMemo(() => {
        const rootTopZ = root.transform.pos.z + root.diskHeight + root.coneHeight;

        return shapedSupport.segments.map((segment, index) => {
            // Compute start/end positions from joints
            let startPos = index === 0
                ? { x: root.transform.pos.x, y: root.transform.pos.y, z: rootTopZ }
                : segment.bottomJoint?.pos ?? { x: 0, y: 0, z: 0 };
            let endPos = segment.topJoint?.pos ?? { x: 0, y: 0, z: 0 };

            return (
                <ShaftRenderer
                    key={segment.id}
                    id={segment.id}
                    start={startPos}
                    end={endPos}
                    diameter={segment.diameter}
                    color={color}
                    emissive={emissive}
                    emissiveIntensity={emissiveIntensity}
                    radialSegments={radialSegments}
                    isParentSelected={isSelected}
                    isInteractable={isInteractable}
                    isSelected={selectedId === segment.id}
                />
            );
        });
    }, [shapedSupport.segments, root, color, emissive, emissiveIntensity, radialSegments, isSelected, isInteractable, selectedId]);

    // Build joint elements (only when selected)
    const joints = useMemo(() => {
        if (!isSelected) return null;
        return shapedSupport.segments
            .filter(seg => seg.topJoint)
            .map(seg => (
                <JointRenderer
                    key={seg.topJoint!.id}
                    joint={seg.topJoint!}
                    color="#888888"
                    emissive={emissive}
                    emissiveIntensity={emissiveIntensity}
                    isParentSelected={isSelected}
                />
            ));
    }, [shapedSupport.segments, isSelected, emissive, emissiveIntensity, radialSegments]);

    return (
        <group>
            {/* Picking group: roots, shafts, shaped contact */}
            <group>
                <RootsRenderer
                    root={root}
                    shaftDiameter={shaftDiameter}
                    color={color}
                    emissive={emissive}
                    emissiveIntensity={emissiveIntensity}
                    radialSegments={radialSegments}
                    sphereSegments={radialSegments}
                />
                {shafts}
                <ShapedContactRenderer
                    shapedContact={shapedSupport.shapedContact}
                    color={color}
                    emissive={emissive}
                    emissiveIntensity={emissiveIntensity}
                />
            </group>

            {/* Joints (separate for interaction) */}
            {joints}
        </group>
    );
});

ShapedSupportRenderer.displayName = 'ShapedSupportRenderer';
