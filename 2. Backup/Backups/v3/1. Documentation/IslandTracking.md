# Island Tracking System

## Overview

The island tracking system implements cross-layer ID propagation and parent-child relationships for islands detected during STL slicing. This enables you to identify and visualize the complete 3D volumes associated with each island.

## Architecture

### Core Components

1. **IslandTracker** (`islandTracker.ts`)
   - Manages island ID propagation across layers
   - Detects merges and assigns parent-child relationships
   - Tracks island lifecycle (active/complete status)
   - Maintains per-layer area statistics

2. **ScanOrchestrator** (`ScanOrchestrator.ts`)
   - Coordinates parallel worker-based island detection
   - Processes layers sequentially for ID tracking
   - Returns complete scan results with island metadata

3. **Island Volume Utilities** (`islandVolume.ts`)
   - Query functions for island pixels, bounding boxes, hierarchies
   - Volume calculations
   - Layer-based island queries

## Data Structures

### Island Type
```typescript
interface Island {
  id: number;                        // Unique island ID
  firstLayer: number;                // Layer where island first appeared
  lastLayer: number;                 // Last layer (finalized or merged)
  status: 'active' | 'complete';     // Active or completed (merged)
  totalAreaMm2: number;              // Total cross-sectional area
  perLayerAreaMm2: Map<number, number>; // Area at each layer
  parentId: number | null;           // Parent island ID if merged
  childIds: number[];                // Child island IDs that merged into this
}
```

### ScanResults Type
```typescript
interface ScanResults {
  grid: GridRef;                     // Pixel grid metadata
  layers: ScanLayerResult[];         // Per-layer results
  firstHit: Int16Array;              // First layer each pixel was an island
  lastHit: Int16Array;               // Last layer each pixel was an island
  baseFootprint: Uint8Array;         // Union of all island pixels
  baseLabels: Int32Array;            // Base footprint component labels
  compBase: Int16Array;              // Base layer per component
  compTop: Int16Array;               // Top layer per component
  islands: Island[];                 // All tracked islands
  islandLabelsPerLayer: Int32Array[]; // Island IDs per pixel per layer
}
```

## How It Works

### 1. Island Detection (Per Layer)
Workers detect unsupported regions at each layer by comparing to the dilated previous layer.

### 2. ID Propagation (Sequential)
After workers complete, the `IslandTracker` processes layers sequentially:

- **Layer 0**: All components are new islands, assigned unique IDs
- **Layer L > 0**: For each component:
  - Find overlapping island IDs from previous layer
  - **No overlap**: New island (assign new ID)
  - **Single overlap**: Continuation (inherit ID)
  - **Multiple overlaps**: Merge (largest becomes parent, others marked complete)

### 3. Parent-Child Tracking
When islands merge:
- The island with the largest area at the previous layer becomes the **parent**
- Other islands become **children** and are marked `status: 'complete'`
- Children record their `parentId`
- Parent records child IDs in `childIds` array

## Usage Examples

### Basic Island Scan
```typescript
import { runIslandScan } from '@/modules/island';

const scanResults = await runIslandScan(
  { geometry, bbox },
  layerHeightMm,
  {
    px_mm: 0.10,
    support_buffer_mm: 0.6,
    connectivity: 4,
  },
  (done, total) => console.log(`Progress: ${done}/${total}`)
);

// Access all islands
console.log(`Found ${scanResults.islands.length} islands`);
```

### Query Island Volumes
```typescript
import { 
  getIslandPixelsByLayer,
  getIslandBoundingBox,
  calculateIslandVolume,
} from '@/modules/island';

// Get pixels for a specific island
const pixelsByLayer = getIslandPixelsByLayer(islandId, scanResults);

// Get 3D bounding box
const bbox = getIslandBoundingBox(island, scanResults, layerHeightMm);

// Calculate volume
const volumeMm3 = calculateIslandVolume(island, scanResults, layerHeightMm);
```

### Explore Island Hierarchy
```typescript
import { 
  getIslandHierarchy,
  getIslandDescendants,
} from '@/modules/island';

// Get parent-child relationships
const hierarchy = getIslandHierarchy(scanResults);

// Find all descendants of an island
const descendants = getIslandDescendants(islandId, scanResults);
```

### Query Islands at Layer
```typescript
import { 
  getIslandsAtLayer,
  getIslandIdAtPixel,
} from '@/modules/island';

// Get all active islands at a layer
const activeIslands = getIslandsAtLayer(layerIdx, scanResults);

// Get island ID at specific pixel
const islandId = getIslandIdAtPixel(layerIdx, pixelIdx, scanResults);
```

## Visualization Applications

### 1. Color by Island ID
Use `islandLabelsPerLayer` to assign unique colors to each island's volume:
```typescript
for (let layer = 0; layer < scanResults.islandLabelsPerLayer.length; layer++) {
  const labels = scanResults.islandLabelsPerLayer[layer];
  for (let i = 0; i < labels.length; i++) {
    const islandId = labels[i];
    if (islandId > 0) {
      // Assign color based on islandId
      const color = getColorForIsland(islandId);
      paintPixel(layer, i, color);
    }
  }
}
```

### 2. Highlight Island Families
Show parent islands and their merged children with related colors:
```typescript
const hierarchy = getIslandHierarchy(scanResults);
for (const [parentId, children] of hierarchy) {
  const baseColor = getColorForIsland(parentId);
  // Paint parent with base color
  // Paint children with variations of base color
}
```

### 3. Island Lifecycle Animation
Animate islands appearing, growing, and merging:
```typescript
for (const island of scanResults.islands) {
  // Highlight from firstLayer to lastLayer
  // Show merge event if parentId !== null
}
```

## Performance Considerations

- **Worker Parallelism**: Layer detection uses Web Workers (CPU concurrency)
- **Sequential Tracking**: ID propagation is sequential (required for correctness)
- **Memory**: `islandLabelsPerLayer` stores Int32Array per layer
- **Optimization**: Use `getIslandPixelsByLayer` caching if querying same island repeatedly

## Design Alignment

This implementation follows the design specification in `IslandScannerDesign.md`:
- ✅ Stable island IDs across layers (lines 9, 19)
- ✅ Parent-child merge rules (lines 10, 20, 51)
- ✅ Island lifecycle tracking (lines 21, 31-32)
- ✅ Per-layer area statistics (line 52)
- ✅ 4-connectivity (default, line 26)

## Future Enhancements

1. **8-connectivity support**: Already in design, needs testing
2. **Island splitting detection**: Track when islands split into multiple components
3. **Minimum area filtering**: Optional threshold for tiny islands
4. **GPU acceleration**: For very high-resolution scans
5. **Incremental updates**: Update only changed layers when parameters adjust
