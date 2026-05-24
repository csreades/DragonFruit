import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  snapAngle,
  SNAP_COARSE,
  SNAP_FINE,
  TICK_MINOR,
  getSnapTicks,
  DEFAULT_SNAP_TICK_CONFIG,
  buildTierSegmentPoints,
} from "../snapRotation";

const toDeg = (rad: number) => Math.round((rad * 180) / Math.PI);

describe("snapAngle", () => {
  it("snaps 0 to 0", () => {
    assert.equal(snapAngle(0, SNAP_COARSE), 0);
  });

  it("snaps below midpoint to lower increment (PI/9 → 0 for 45° grid)", () => {
    // PI/9 ≈ 20° < 22.5° midpoint → rounds down to 0
    assert.equal(snapAngle(Math.PI / 9, SNAP_COARSE), 0);
  });

  it("snaps above midpoint to upper increment (PI/3 → PI/4 for 45° grid)", () => {
    // PI/3 ≈ 60° > 22.5° midpoint → rounds up to PI/4 (45°)
    assert.equal(snapAngle(Math.PI / 3, SNAP_COARSE), Math.PI / 4);
  });

  it("snaps negative angles correctly (-PI/3 → -PI/4 for 45° grid)", () => {
    assert.equal(snapAngle(-Math.PI / 3, SNAP_COARSE), -Math.PI / 4);
  });

  it("snaps full rotation (2*PI → 2*PI for 45° grid)", () => {
    const result = snapAngle(2 * Math.PI, SNAP_COARSE);
    assert.ok(
      Math.abs(result - 2 * Math.PI) < 1e-10,
      `Expected ~2*PI, got ${result}`,
    );
  });

  it("snaps PI exactly for fine grid (PI/12)", () => {
    const result = snapAngle(Math.PI, SNAP_FINE);
    assert.ok(
      Math.abs(result - Math.PI) < 1e-10,
      `Expected ~PI, got ${result}`,
    );
  });

  it("snaps to fine grid increments (PI/6 → PI/6 for 15° grid)", () => {
    const result = snapAngle(Math.PI / 6, SNAP_FINE);
    assert.ok(
      Math.abs(result - Math.PI / 6) < 1e-10,
      `Expected ~PI/6, got ${result}`,
    );
  });
});

describe("snap constants", () => {
  it("SNAP_COARSE is 45 degrees in radians", () => {
    assert.ok(
      Math.abs(SNAP_COARSE - Math.PI / 4) < 1e-10,
      `Expected PI/4, got ${SNAP_COARSE}`,
    );
  });

  it("SNAP_FINE is 15 degrees in radians", () => {
    assert.ok(
      Math.abs(SNAP_FINE - Math.PI / 12) < 1e-10,
      `Expected PI/12, got ${SNAP_FINE}`,
    );
  });
});

describe("transition tolerance", () => {
  it("coarse-to-fine transition from aligned position produces no jump", () => {
    // lastSnapped = PI/4 (45°), rawAccumulated resets to PI/4
    // snapAngle(PI/4, SNAP_FINE) should equal PI/4 (since PI/4 = 3*PI/12)
    const lastSnapped = Math.PI / 4;
    const result = snapAngle(lastSnapped, SNAP_FINE);
    assert.ok(
      Math.abs(result - lastSnapped) < 1e-10,
      `Expected no jump, got delta ${result - lastSnapped}`,
    );
  });

  it("transition quantization error is within half-increment", () => {
    // Worst case: value exactly at midpoint between two fine grid lines
    const halfFine = SNAP_FINE / 2;
    const testAngle = SNAP_FINE * 2.5; // exactly between 2*SNAP_FINE and 3*SNAP_FINE
    const result = snapAngle(testAngle, SNAP_FINE);
    const error = Math.abs(result - testAngle);
    assert.ok(
      error <= halfFine + 1e-10,
      `Quantization error ${error} exceeds half-increment ${halfFine}`,
    );
  });
});

describe("getSnapTicks (default 45/15/5 config)", () => {
  const ticks = getSnapTicks();

  it("produces 72 ticks (360 / 5deg)", () => {
    assert.equal(ticks.length, 72);
  });

  it("classifies into 8 major / 16 medium / 48 minor", () => {
    const counts = ticks.reduce(
      (acc, t) => {
        acc[t.tier] += 1;
        return acc;
      },
      { major: 0, medium: 0, minor: 0 } as Record<string, number>,
    );
    assert.deepEqual(counts, { major: 8, medium: 16, minor: 48 });
  });

  it("assigns each angular position to exactly one tier (no duplicate angles)", () => {
    const seen = new Set(ticks.map((t) => toDeg(t.angleRad)));
    assert.equal(seen.size, ticks.length);
  });

  it("major angles are multiples of 45deg", () => {
    for (const t of ticks.filter((t) => t.tier === "major")) {
      assert.equal(toDeg(t.angleRad) % 45, 0, `major at ${toDeg(t.angleRad)} not multiple of 45`);
    }
  });

  it("medium angles are multiples of 15 but not 45", () => {
    for (const t of ticks.filter((t) => t.tier === "medium")) {
      const deg = toDeg(t.angleRad);
      assert.equal(deg % 15, 0, `medium ${deg} not multiple of 15`);
      assert.notEqual(deg % 45, 0, `medium ${deg} should not also be major`);
    }
  });

  it("minor angles are multiples of 5 but not 15", () => {
    for (const t of ticks.filter((t) => t.tier === "minor")) {
      const deg = toDeg(t.angleRad);
      assert.equal(deg % 5, 0, `minor ${deg} not multiple of 5`);
      assert.notEqual(deg % 15, 0, `minor ${deg} should not also be medium/major`);
    }
  });

  it("length multipliers are 1.0 / 0.6 / 0.3 by tier", () => {
    for (const t of ticks) {
      const expected = t.tier === "major" ? 1.0 : t.tier === "medium" ? 0.6 : 0.3;
      assert.equal(t.lengthMult, expected, `tier ${t.tier} lengthMult ${t.lengthMult}`);
    }
  });

  it("all angles fall in [0, 2PI)", () => {
    for (const t of ticks) {
      assert.ok(
        t.angleRad >= 0 && t.angleRad < 2 * Math.PI,
        `angle ${t.angleRad} out of [0, 2PI)`,
      );
    }
  });

  it("TICK_MINOR equals 5 degrees in radians", () => {
    assert.ok(Math.abs(TICK_MINOR - Math.PI / 36) < 1e-12, `Expected PI/36, got ${TICK_MINOR}`);
  });

  it("passing the explicit default config equals the no-arg call", () => {
    assert.deepEqual(getSnapTicks(DEFAULT_SNAP_TICK_CONFIG), ticks);
  });
});

describe("buildTierSegmentPoints", () => {
  const ticks = getSnapTicks();
  const R = 4.8;
  const baseLength = 0.6;

  it("emits two points (outer, inner) per tick of the requested tier", () => {
    assert.equal(buildTierSegmentPoints(ticks, R, baseLength, "major").length, 8 * 2);
    assert.equal(buildTierSegmentPoints(ticks, R, baseLength, "medium").length, 16 * 2);
    assert.equal(buildTierSegmentPoints(ticks, R, baseLength, "minor").length, 48 * 2);
  });

  it("outer point lies on the ring radius; inner is shortened by baseLength*lengthMult", () => {
    const major = buildTierSegmentPoints(ticks, R, baseLength, "major");
    const outer = major[0];
    const inner = major[1];
    assert.ok(Math.abs(Math.hypot(outer[0], outer[1]) - R) < 1e-9, `outer radius ${Math.hypot(outer[0], outer[1])}`);
    // major lengthMult is 1.0 → inner radius = R - baseLength
    assert.ok(
      Math.abs(Math.hypot(inner[0], inner[1]) - (R - baseLength)) < 1e-9,
      `inner radius ${Math.hypot(inner[0], inner[1])}`,
    );
  });

  it("minor ticks are shorter than major ticks (lengthMult 0.3 vs 1.0)", () => {
    const major = buildTierSegmentPoints(ticks, R, baseLength, "major");
    const minor = buildTierSegmentPoints(ticks, R, baseLength, "minor");
    const majorLen = R - Math.hypot(major[1][0], major[1][1]);
    const minorLen = R - Math.hypot(minor[1][0], minor[1][1]);
    assert.ok(minorLen < majorLen, `minor ${minorLen} should be < major ${majorLen}`);
  });
});
