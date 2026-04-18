import type { Vec3, SupportEntity, Segment } from '../../types';
import type { SupportTipProfile } from '../../SupportPrimitives/ContactCone/types';

// ---------------------------------------------------------------------------
// Shaped Contact — the terminal element unique to ShapedSupport
// ---------------------------------------------------------------------------

/**
 * Describes the two user-placed surface points that define the contact footprint.
 */
export interface ShapedContactPoints {
    /** First click position on the model surface */
    pointA: Vec3;
    /** Second click position on the model surface */
    pointB: Vec3;
    /** Surface normal at point A */
    normalA: Vec3;
    /** Surface normal at point B */
    normalB: Vec3;
}

/**
 * ShapedContact: Replaces the standard ContactCone for shaped supports.
 *
 * The contact face is a rounded rectangle whose long axis spans pointA → pointB.
 * The body lofts from that rounded-rect cross-section down to a circular
 * cross-section at the socket joint.
 */
export interface ShapedContact {
    id: string;

    /** Center position of the contact face on the model surface */
    pos: Vec3;
    /** Average surface normal at the contact (used for face orientation) */
    normal: Vec3;
    /** The actual surface normal (for disk-like alignment) */
    surfaceNormal?: Vec3;

    /** The two user-placed points that define the contact footprint */
    points: ShapedContactPoints;

    /** Long-axis length of the rounded rectangle (distance A→B, clamped) */
    lengthMm: number;
    /** Short-axis width (clamped to contact diameter setting) */
    widthMm: number;
    /** Corner chamfer radius */
    chamferRadiusMm: number;

    /** Tip profile inherited from settings (provides body diameter, penetration, etc.) */
    profile: SupportTipProfile;

    /** Height of the loft body from contact face down to socket joint */
    bodyHeightMm: number;

    /** The Joint this shaped contact connects to at its base */
    socketJointId?: string;
}

// ---------------------------------------------------------------------------
// ShapedSupport — full entity
// ---------------------------------------------------------------------------

/**
 * ShapedSupport: Structurally identical to a Trunk from Roots through
 * Shaft/Joint segments, but the terminal contact element is a ShapedContact
 * instead of a standard ContactCone.
 */
export interface ShapedSupport extends SupportEntity {
    rootId: string;
    baseDiameterMm?: number;
    segments: Segment[];
    shapedContact: ShapedContact;
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

export interface ShapedSupportSettings {
    /** Maximum contact diameter (short axis), mm */
    contactDiameterMm: number;
    /** Maximum contact length (long axis), mm */
    maxLengthMm: number;
    /** Corner chamfer radius, mm */
    chamferRadiusMm: number;
    /** Penetration depth into model surface, mm */
    penetrationMm: number;
    /** Loft body height from contact face to socket joint, mm */
    bodyHeightMm: number;
}

/** Default shaped support settings */
export const DEFAULT_SHAPED_SUPPORT_SETTINGS: ShapedSupportSettings = {
    contactDiameterMm: 0.4,
    maxLengthMm: 10.0,
    chamferRadiusMm: 0.15,
    penetrationMm: 0.05,
    bodyHeightMm: 3.0,
};
