/**
 * useShapedSupportPlacement
 *
 * Two-click placement interaction for shaped supports.
 * 1. First click: set point A on the mesh surface.
 * 2. Mouse move: live preview of contact rectangle stretching to cursor.
 * 3. Second click: set point B and commit the shaped support.
 */

import { useCallback, useRef, useState } from 'react';
import * as THREE from 'three';
import type { Vec3 } from '../../types';
import { addRoot, addShapedSupport } from '../../state';
import { pushHistory } from '@/history/historyStore';
import { SUPPORT_ADD_SHAPED } from '../../history/actionTypes';
import { buildShapedSupportData, type ShapedSupportBuildInput } from './shapedSupportBuilder';
import { calculateSmoothedNormal } from '../../PlacementLogic/PlacementUtils';
import { setShapedPreview } from './shapedPreviewState';

export interface ShapedPlacementPreview {
    /** Point A (first click, locked in) */
    pointA: Vec3;
    normalA: Vec3;
    /** Current cursor point (live, becomes point B on second click) */
    pointB: Vec3;
    normalB: Vec3;
    /** Mesh for surface sampling */
    mesh?: THREE.Mesh;
}

interface ShapedSupportPlacementState {
    /** Whether we are waiting for the second click */
    awaitingSecondClick: boolean;
    /** Preview data when first point is set and cursor is moving */
    preview: ShapedPlacementPreview | null;
}

export function useShapedSupportPlacement() {
    const [placementState, setPlacementState] = useState<ShapedSupportPlacementState>({
        awaitingSecondClick: false,
        preview: null,
    });

    const firstPointRef = useRef<{ pos: Vec3; normal: Vec3; mesh?: THREE.Mesh } | null>(null);

    /**
     * Called on pointer move over the model mesh (THREE.Intersection).
     * When first point is set, updates the live preview with cursor position as point B.
     */
    const onModelHover = useCallback(
        (hit: THREE.Intersection | null) => {
            if (!hit || !firstPointRef.current) return;

            const normal = calculateSmoothedNormal(hit);

            const preview: ShapedPlacementPreview = {
                pointA: firstPointRef.current.pos,
                normalA: firstPointRef.current.normal,
                pointB: { x: hit.point.x, y: hit.point.y, z: hit.point.z },
                normalB: normal,
                mesh: firstPointRef.current.mesh,
            };
            setPlacementState({
                awaitingSecondClick: true,
                preview,
            });
            setShapedPreview(preview);
        },
        [],
    );

    /**
     * Called on click on the model mesh (THREE.Intersection).
     * First click sets point A; second click sets point B and commits the support.
     */
    const onModelClick = useCallback(
        (hit: THREE.Intersection) => {
            const normal = calculateSmoothedNormal(hit);
            const point: Vec3 = { x: hit.point.x, y: hit.point.y, z: hit.point.z };
            const modelId = hit.object?.userData?.modelId || 'unknown';

            if (!firstPointRef.current) {
                // First click — set point A
                const hitMesh = hit.object instanceof THREE.Mesh ? hit.object : undefined;
                firstPointRef.current = { pos: point, normal, mesh: hitMesh };
                setPlacementState({
                    awaitingSecondClick: true,
                    preview: null,
                });
                return;
            }

            // Second click — commit the shaped support
            const buildInput: ShapedSupportBuildInput = {
                pointA: firstPointRef.current.pos,
                normalA: firstPointRef.current.normal,
                pointB: point,
                normalB: normal,
                modelId,
                mesh: firstPointRef.current.mesh,
            };

            const result = buildShapedSupportData(buildInput);

            // Add to state
            addRoot(result.root);
            addShapedSupport(result.shapedSupport);

            // Push history for undo
            pushHistory({
                type: SUPPORT_ADD_SHAPED,
                payload: {
                    root: result.root,
                    shapedSupport: result.shapedSupport,
                },
            });

            // Reset for next placement
            firstPointRef.current = null;
            setPlacementState({
                awaitingSecondClick: false,
                preview: null,
            });
            setShapedPreview(null);
        },
        [],
    );

    /**
     * Cancel the current placement (e.g. on Escape or tool switch).
     */
    const cancelPlacement = useCallback(() => {
        firstPointRef.current = null;
        setPlacementState({
            awaitingSecondClick: false,
            preview: null,
        });
        setShapedPreview(null);
    }, []);

    return {
        placementState,
        onModelHover,
        onModelClick,
        cancelPlacement,
    };
}
