/**
 * Shaped Support Builder
 *
 * Builds a ShapedSupport entity from two user-placed surface points.
 * The body is a simple vertical trunk (no pathfinding) from roots to a
 * socket joint, topped by a ShapedContact element.
 */

import * as THREE from 'three';
import type { Vec3, Roots, Segment, Joint } from '../../types';
import { getJointDiameter } from '../../constants';
import { getSettings } from '../../Settings';
import type { ShapedSupport, ShapedContact, ShapedContactPoints, ShapedSupportSettings } from './types';
import { DEFAULT_SHAPED_SUPPORT_SETTINGS } from './types';

function uuidv4() {
    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function (c) {
        const r = (Math.random() * 16) | 0,
            v = c === 'x' ? r : (r & 0x3) | 0x8;
        return v.toString(16);
    });
}

export interface ShapedSupportBuildInput {
    /** First point on model surface */
    pointA: Vec3;
    /** Surface normal at point A */
    normalA: Vec3;
    /** Second point on model surface */
    pointB: Vec3;
    /** Surface normal at point B */
    normalB: Vec3;
    /** Model this support belongs to */
    modelId: string;
    /** Optional overrides for shaped support settings */
    shapedSettings?: Partial<ShapedSupportSettings>;
}

export interface ShapedSupportBuildResult {
    root: Roots;
    shapedSupport: ShapedSupport;
}

export function buildShapedSupportData(input: ShapedSupportBuildInput): ShapedSupportBuildResult {
    const { pointA, normalA, pointB, normalB, modelId, shapedSettings } = input;
    const settings = getSettings();

    const shaped: ShapedSupportSettings = {
        ...DEFAULT_SHAPED_SUPPORT_SETTINGS,
        ...settings.shaped,
        ...shapedSettings,
    };

    // Shaft/roots from global settings
    const shaftDiameter = settings.shaft.diameterMm;
    const rootsDiameter = settings.roots.diameterMm;
    const diskHeight = settings.roots.diskHeightMm;
    const coneHeight = settings.roots.coneHeightMm;
    const jointDiameter = getJointDiameter(shaftDiameter);

    // --- Contact face computation ---
    // Midpoint of A and B is the contact center
    const contactCenter: Vec3 = {
        x: (pointA.x + pointB.x) / 2,
        y: (pointA.y + pointB.y) / 2,
        z: (pointA.z + pointB.z) / 2,
    };

    // Average surface normal
    const avgNormal = new THREE.Vector3(
        (normalA.x + normalB.x) / 2,
        (normalA.y + normalB.y) / 2,
        (normalA.z + normalB.z) / 2,
    );
    if (avgNormal.lengthSq() < 0.0001) avgNormal.set(0, 0, -1);
    avgNormal.normalize();

    // Long axis length = distance between A and B, clamped to max
    const dx = pointB.x - pointA.x;
    const dy = pointB.y - pointA.y;
    const dz = pointB.z - pointA.z;
    const rawLength = Math.sqrt(dx * dx + dy * dy + dz * dz);
    const contactLength = Math.min(rawLength, shaped.maxLengthMm);

    // Short axis = contact diameter setting
    const contactWidth = shaped.contactDiameterMm;

    // --- Socket position (bottom of shaped contact body) ---
    // The shaped contact loft extends along the average normal direction
    const socketPos: Vec3 = {
        x: contactCenter.x + avgNormal.x * shaped.bodyHeightMm,
        y: contactCenter.y + avgNormal.y * shaped.bodyHeightMm,
        z: contactCenter.z + avgNormal.z * shaped.bodyHeightMm,
    };

    // --- Roots base position (directly below socket on XY plane, Z=0) ---
    const basePos: Vec3 = { x: socketPos.x, y: socketPos.y, z: 0 };
    const rootsTopZ = diskHeight + coneHeight;

    // --- Build joints and segments (simple vertical trunk) ---
    const socketJointId = uuidv4();
    const socketJoint: Joint = {
        id: socketJointId,
        pos: socketPos,
        diameter: jointDiameter,
    };

    // Single mid-joint for a straight support (same pattern as trunk)
    const midZ = (rootsTopZ + socketPos.z) / 2;
    const midJoint: Joint = {
        id: uuidv4(),
        pos: { x: basePos.x, y: basePos.y, z: midZ },
        diameter: jointDiameter,
    };

    const segments: Segment[] = [
        {
            id: uuidv4(),
            diameter: shaftDiameter,
            topJoint: midJoint,
        },
        {
            id: uuidv4(),
            diameter: shaftDiameter,
            bottomJoint: midJoint,
            topJoint: socketJoint,
        },
    ];

    // --- Build Root ---
    const rootId = uuidv4();
    const root: Roots = {
        id: rootId,
        modelId,
        transform: {
            pos: basePos,
            rot: { x: 0, y: 0, z: 0, w: 1 },
        },
        diameter: rootsDiameter,
        diskHeight,
        coneHeight,
    };

    // --- Build ShapedContact ---
    const shapedContact: ShapedContact = {
        id: uuidv4(),
        pos: contactCenter,
        normal: { x: avgNormal.x, y: avgNormal.y, z: avgNormal.z },
        surfaceNormal: { x: avgNormal.x, y: avgNormal.y, z: avgNormal.z },
        points: {
            pointA,
            pointB,
            normalA,
            normalB,
        },
        lengthMm: contactLength,
        widthMm: contactWidth,
        chamferRadiusMm: shaped.chamferRadiusMm,
        profile: {
            type: 'disk',
            contactDiameterMm: contactWidth,
            bodyDiameterMm: settings.tip.bodyDiameterMm,
            lengthMm: shaped.bodyHeightMm,
            penetrationMm: shaped.penetrationMm,
            diskThicknessMm: settings.tip.diskThicknessMm ?? 0.1,
            maxStandoffMm: settings.tip.maxStandoffMm ?? 1.5,
            standoffAngleThreshold: settings.tip.standoffAngleThreshold ?? Math.PI / 4,
        },
        bodyHeightMm: shaped.bodyHeightMm,
        socketJointId,
    };

    // --- Build ShapedSupport ---
    const shapedSupport: ShapedSupport = {
        id: uuidv4(),
        modelId,
        rootId,
        baseDiameterMm: shaftDiameter,
        segments,
        shapedContact,
    };

    return { root, shapedSupport };
}
