"use client";

import React, { useMemo } from 'react';
import { Line } from '@react-three/drei';
import { GIZMO_SIZES } from '../constants';
import {
  getSnapTicks,
  buildTierSegmentPoints,
  DEFAULT_SNAP_TICK_CONFIG,
  type SnapTickConfig,
  type TickTier,
} from './snapRotation';

interface SnapTickMarksProps {
  /** Axis ring color (red/green/blue). Stays axis-colored regardless of drag state. */
  color: string;
  /** True when the ring is hovered — ticks fade in. */
  hovered: boolean;
  /** True during an active drag — ticks are strongest. */
  active: boolean;
  /** Multiplies the computed opacity (mirrors the ring's opacityScale). */
  opacityScale?: number;
  /** Tier intervals. Defaults to 45/15/5; #104 supplies persisted config. */
  config?: SnapTickConfig;
}

/** Full (major) tick length as a fraction of the ring radius; tiers scale via lengthMult. */
const TICK_BASE_LENGTH = GIZMO_SIZES.ringMajorRadius * 0.12;

/** Stroke width per tier — higher tiers are thicker. */
const TIER_LINE_WIDTH: Record<TickTier, number> = {
  major: 2.0,
  medium: 1.3,
  minor: 0.7,
};

const TIERS: TickTier[] = ['major', 'medium', 'minor'];

/**
 * SnapTickMarks — static radial tick marks around the rotation ring marking the
 * snap positions on the rotation ring. Rendered in the ring's
 * fixed local frame (NOT the camera-following arc group) so the marks stay at
 * fixed angular positions. Hidden when idle, fades in on hover, strongest during
 * an active drag. One <Line segments> per tier so length and stroke width can
 * differ per tier.
 */
export function SnapTickMarks({
  color,
  hovered,
  active,
  opacityScale = 1,
  config = DEFAULT_SNAP_TICK_CONFIG,
}: SnapTickMarksProps) {
  const segmentsByTier = useMemo(() => {
    const ticks = getSnapTicks(config);
    const radius = GIZMO_SIZES.ringMajorRadius;
    return TIERS.map((tier) => ({
      tier,
      points: buildTierSegmentPoints(ticks, radius, TICK_BASE_LENGTH, tier),
    }));
  }, [config]);

  const opacity = (active ? 0.85 : hovered ? 0.45 : 0) * opacityScale;
  if (opacity <= 0) return null;

  return (
    <group>
      {segmentsByTier.map(({ tier, points }) =>
        points.length > 0 ? (
          <Line
            key={tier}
            points={points}
            segments
            color={color}
            lineWidth={TIER_LINE_WIDTH[tier]}
            transparent
            opacity={opacity}
            depthTest={false}
            toneMapped={false}
          />
        ) : null,
      )}
    </group>
  );
}
