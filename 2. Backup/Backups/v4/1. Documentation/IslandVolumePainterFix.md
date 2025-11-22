# Island Volume Painter Fix

## Problem

The original `IslandVolumePainter` was only painting the bottom layer of islands instead of the full 3D volumes.

### Root Cause

The flood-fill approach had a fundamental flaw:

1. **Seeded only at base layer**: Only triangles at `island.firstLayer` were identified as seeds
2. **Flood-fill through mesh connectivity**: Used adjacency map to propagate through connected triangles
3. **Ignored vertical extent**: Didn't check if triangles actually belonged to the island across its full layer range

**Why it failed:**
- Flood-fill propagates through mesh topology, not through the island's actual 3D volume
- If there were gaps or disconnected regions in the mesh, the flood wouldn't reach them
- Triangles between layers weren't properly detected

## Solution

Replaced flood-fill with **direct triangle-to-island matching** across the full vertical extent:

### New Algorithm

For each triangle:

1. **Calculate triangle's Y range** (min/max Y of vertices in world space)
2. **Convert to layer range** (triMinLayer to triMaxLayer)
3. **Sample 4 points**: centroid + 3 vertices in XZ plane
4. **Check all layers** the triangle spans (layerStart to layerEnd)
5. **Count overlap**: For each layer, check how many sample points fall within island pixels
6. **Assign to best match**: Triangle belongs to island with highest overlap score

### Key Improvements

- **Full vertical coverage**: Checks every layer the triangle spans, not just the base
- **No flood-fill dependency**: Each triangle is independently evaluated
- **Handles gaps**: Works even with disconnected mesh regions
- **Respects island hierarchy**: Only processes active islands (or merged if enabled)

### Code Changes

**Removed:**
- `buildAdjacencyMap()` function (no longer needed)
- Seed triangle collection loop
- Flood-fill queue processing

**Added:**
- Triangle Y range calculation
- Layer range intersection with island bounds
- Multi-layer sample point checking
- Overlap scoring system

## Performance Considerations

The new approach is O(T × I × L × S) where:
- T = number of triangles
- I = number of islands
- L = average layers per triangle (typically 1-3)
- S = sample points per triangle (4)

This is more expensive than flood-fill but necessary for correctness. For typical models:
- ~10K triangles
- ~10-50 islands
- ~1-3 layers per triangle
- = ~1-6M checks (fast enough for real-time)

## Testing

To verify the fix works:

1. Load an STL with unsupported overhangs
2. Run island scan
3. Enable island volume visualization
4. Check that **entire volumes** are colored, not just base layers
5. Try different color schemes (unique, lifecycle, volume, layers)
6. Test with island selection to highlight specific volumes

## Future Optimizations

If performance becomes an issue:

1. **Spatial indexing**: Build octree/BVH for faster triangle-to-island queries
2. **Reduce sampling**: Use fewer sample points (e.g., centroid only)
3. **Layer skipping**: Sample every Nth layer for large triangles
4. **Parallel processing**: Use Web Workers for triangle assignment
5. **Caching**: Store triangle-island assignments between color scheme changes
