/**
 * Check 2 (geometry mode) — the section node graph.
 *
 * Generalises sectionPeelProfile.ts from a single vertical mass to the branching
 * topology of a real part: the connected-component graph the island pipeline
 * already builds (per-slice islands linked by parent/child). Each node is a
 * vertically-tracked connected feature carrying a cross-section-area profile;
 * edges point UP (toward the top of the print, away from the plate).
 *
 * A neck (thin section) must resist the worst single peeling layer of EVERYTHING
 * above it — its own upper layers AND every branch that passes through it:
 *
 *   SF(layer i in node N) = green·A[i] / (peel · max( suffixMax(A, i),
 *                                                     maxSectionAbove(N) ))
 *
 * Node SF = min over its layers; a low-SF neck localises to a (layer, node).
 * This is "build the node model with min/max sections, then walk the max–mins."
 *
 * Pure graph arithmetic — unit-tested against synthetic topologies. Building the
 * nodes from the native island graph is the next (native-touching) step.
 */
import {
  DEFAULT_SECTION_MATERIAL,
  sectionBand,
  type SectionMaterial,
} from './sectionPeelProfile';

export interface SectionNode {
  id: string;
  /** Cross-section area per layer over this node's life, ordered plate→top. */
  areaByLayerMm2: number[];
  /** Global layer index of areaByLayerMm2[0] (to locate necks in the print). */
  baseLayer: number;
  /** Nodes directly ABOVE this one (toward the top / away from the plate). */
  childIds: string[];
}

export interface NodeSection {
  id: string;
  minSectionMm2: number;
  maxSectionMm2: number;
}

export interface NodeVerdict {
  id: string;
  sf: number;
  band: 'fail' | 'marginal' | 'ok';
  /** Global layer index of the governing neck. */
  worstLayer: number;
  neckAreaMm2: number;
  peelAreaAboveMm2: number;
}

export interface GraphVerdict {
  perNode: NodeVerdict[];
  worst: NodeVerdict | null;
  failCount: number;
  marginalCount: number;
  nodeCount: number;
}

/** Step 1: min/max cross-section per node. */
export function nodeSections(nodes: SectionNode[]): NodeSection[] {
  return nodes.map((n) => {
    let mn = Infinity;
    let mx = 0;
    for (const a of n.areaByLayerMm2) {
      const v = Math.max(0, a);
      if (v < mn) mn = v;
      if (v > mx) mx = v;
    }
    if (!Number.isFinite(mn)) mn = 0;
    return { id: n.id, minSectionMm2: mn, maxSectionMm2: mx };
  });
}

/**
 * For every node, the max cross-section of everything STRICTLY above it (across
 * all branches). Memoised DFS over childIds; cycle-guarded.
 */
function maxSectionAboveByNode(nodes: SectionNode[]): Map<string, number> {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const externalMemo = new Map<string, number>();

  const ownMax = (n: SectionNode): number => {
    let mx = 0;
    for (const a of n.areaByLayerMm2) mx = Math.max(mx, Math.max(0, a));
    return mx;
  };

  const maxExternal = (id: string): number => {
    const cached = externalMemo.get(id);
    if (cached !== undefined) return cached;
    externalMemo.set(id, 0); // cycle guard
    const n = byId.get(id);
    let m = 0;
    if (n) {
      for (const cid of n.childIds) {
        const child = byId.get(cid);
        if (!child) continue;
        // max section of the child and everything above it
        m = Math.max(m, ownMax(child), maxExternal(cid));
      }
    }
    externalMemo.set(id, m);
    return m;
  };

  const out = new Map<string, number>();
  for (const n of nodes) out.set(n.id, maxExternal(n.id));
  return out;
}

/** Step 2: per-node safety factor over the graph. */
export function analyzeSectionGraph(
  nodes: SectionNode[],
  mat: SectionMaterial = DEFAULT_SECTION_MATERIAL,
): GraphVerdict {
  const green = Math.max(0, mat.greenStrengthMPa);
  const peel = Math.max(1e-12, mat.sigmaPeelMPa);
  const maxAbove = maxSectionAboveByNode(nodes);

  const perNode: NodeVerdict[] = nodes.map((n) => {
    const areas = n.areaByLayerMm2;
    const ext = maxAbove.get(n.id) ?? 0;
    const len = areas.length;
    if (len === 0) {
      return { id: n.id, sf: 0, band: 'fail', worstLayer: n.baseLayer, neckAreaMm2: 0, peelAreaAboveMm2: ext };
    }
    // Suffix-max within the node, seeded by the external mass above it.
    let running = ext;
    let worstSf = Infinity;
    let worstIdx = 0;
    let worstDemand = 0;
    for (let i = len - 1; i >= 0; i--) {
      const a = Math.max(0, areas[i]);
      running = Math.max(running, a);
      const demand = peel * running;
      const sf = a <= 0 ? 0 : demand <= 0 ? Infinity : (green * a) / demand;
      if (sf <= worstSf) {
        worstSf = sf;
        worstIdx = i;
        worstDemand = running;
      }
    }
    return {
      id: n.id,
      sf: worstSf,
      band: sectionBand(worstSf),
      worstLayer: n.baseLayer + worstIdx,
      neckAreaMm2: Math.max(0, areas[worstIdx]),
      peelAreaAboveMm2: worstDemand,
    };
  });

  perNode.sort((a, b) => a.sf - b.sf);
  return {
    perNode,
    worst: perNode.length > 0 ? perNode[0] : null,
    failCount: perNode.filter((v) => v.band === 'fail').length,
    marginalCount: perNode.filter((v) => v.band === 'marginal').length,
    nodeCount: nodes.length,
  };
}
