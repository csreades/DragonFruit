import assert from 'node:assert/strict';
import test from 'node:test';

import {
    getResolvedChainMetrics,
    isResolvedChainReplacementBetter,
} from '../PlacementLogic/Pathfinding/SmartPlacementV2';
import type { Vec3 } from '../types';

const socketPos: Vec3 = { x: 0, y: 0, z: 12 };
const rootTopTarget: Vec3 = { x: 3, y: 0, z: 2 };

test('resolved-chain comparison prefers a more vertical upper span over fewer joints', () => {
    const routed = getResolvedChainMetrics(
        socketPos,
        [
            { x: 0.2, y: 0, z: 8.5 },
            { x: 2.2, y: 0, z: 5 },
        ],
        rootTopTarget,
    );
    const straight = getResolvedChainMetrics(socketPos, [], rootTopTarget);

    assert.equal(isResolvedChainReplacementBetter(straight, routed), false);
    assert.equal(isResolvedChainReplacementBetter(routed, straight), true);
});

test('resolved-chain comparison still allows fewer joints when upper-span quality is not worsened', () => {
    const current = getResolvedChainMetrics(
        socketPos,
        [{ x: 0.1, y: 0, z: 7 }],
        { x: 0.2, y: 0, z: 2 },
    );
    const straighter = getResolvedChainMetrics(socketPos, [], { x: 0.15, y: 0, z: 2 });

    assert.equal(isResolvedChainReplacementBetter(straighter, current), true);
});
