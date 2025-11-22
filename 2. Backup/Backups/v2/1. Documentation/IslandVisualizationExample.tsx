/**
 * Example: Island Volume Visualization Component
 * 
 * This example shows how to use the island tracking system to:
 * 1. Identify all islands in a model
 * 2. Display island statistics
 * 3. Highlight specific islands in 3D
 * 4. Show parent-child relationships
 */

import React, { useMemo } from 'react';
import type { ScanResults } from '@/modules/island';
import {
  getIslandBoundingBox,
  calculateIslandVolume,
  getIslandHierarchy,
  getIslandPixelsByLayer,
} from '@/modules/island';

interface IslandVisualizationProps {
  scanResults: ScanResults;
  layerHeightMm: number;
  selectedIslandId?: number;
  onSelectIsland?: (islandId: number) => void;
}

export function IslandVisualization({
  scanResults,
  layerHeightMm,
  selectedIslandId,
  onSelectIsland,
}: IslandVisualizationProps) {
  // Compute island statistics
  const islandStats = useMemo(() => {
    return scanResults.islands.map(island => {
      const volume = calculateIslandVolume(island, scanResults, layerHeightMm);
      const bbox = getIslandBoundingBox(island, scanResults, layerHeightMm);
      const layerSpan = island.lastLayer - island.firstLayer + 1;
      
      return {
        island,
        volume,
        bbox,
        layerSpan,
      };
    });
  }, [scanResults, layerHeightMm]);

  // Get island hierarchy
  const hierarchy = useMemo(() => {
    return getIslandHierarchy(scanResults);
  }, [scanResults]);

  // Separate active and completed islands
  const activeIslands = islandStats.filter(s => s.island.status === 'active');
  const completedIslands = islandStats.filter(s => s.island.status === 'complete');

  return (
    <div className="island-visualization">
      <h2>Island Analysis</h2>
      
      {/* Summary Stats */}
      <div className="summary">
        <div>Total Islands: {scanResults.islands.length}</div>
        <div>Active: {activeIslands.length}</div>
        <div>Merged: {completedIslands.length}</div>
      </div>

      {/* Active Islands List */}
      <div className="island-list">
        <h3>Active Islands</h3>
        {activeIslands.map(({ island, volume, layerSpan }) => (
          <div
            key={island.id}
            className={`island-item ${selectedIslandId === island.id ? 'selected' : ''}`}
            onClick={() => onSelectIsland?.(island.id)}
          >
            <div className="island-header">
              <strong>Island #{island.id}</strong>
              {island.childIds.length > 0 && (
                <span className="merge-badge">
                  Merged {island.childIds.length} island(s)
                </span>
              )}
            </div>
            <div className="island-details">
              <div>Layers: {island.firstLayer} - {island.lastLayer} ({layerSpan} layers)</div>
              <div>Volume: {volume.toFixed(2)} mm³</div>
              <div>Total Area: {island.totalAreaMm2.toFixed(2)} mm²</div>
            </div>
            
            {/* Show merged children */}
            {island.childIds.length > 0 && (
              <div className="children">
                <small>Absorbed islands: {island.childIds.join(', ')}</small>
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Completed Islands (Merged) */}
      {completedIslands.length > 0 && (
        <div className="island-list">
          <h3>Merged Islands</h3>
          {completedIslands.map(({ island, volume, layerSpan }) => (
            <div
              key={island.id}
              className={`island-item merged ${selectedIslandId === island.id ? 'selected' : ''}`}
              onClick={() => onSelectIsland?.(island.id)}
            >
              <div className="island-header">
                <strong>Island #{island.id}</strong>
                <span className="merge-info">
                  → Merged into #{island.parentId}
                </span>
              </div>
              <div className="island-details">
                <div>Layers: {island.firstLayer} - {island.lastLayer} ({layerSpan} layers)</div>
                <div>Volume: {volume.toFixed(2)} mm³</div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * Example: 3D Island Highlighter
 * 
 * This component shows how to highlight a specific island's volume in the 3D view.
 */
export function useIslandHighlight(
  islandId: number | undefined,
  scanResults: ScanResults | null
) {
  return useMemo(() => {
    if (!islandId || !scanResults) return null;

    const island = scanResults.islands.find(i => i.id === islandId);
    if (!island) return null;

    // Get all pixels belonging to this island
    const pixelsByLayer = getIslandPixelsByLayer(islandId, scanResults);
    
    // Convert to world coordinates for rendering
    const { grid } = scanResults;
    const highlightData: Array<{ layer: number; x: number; z: number }> = [];

    for (const [layer, pixels] of pixelsByLayer) {
      for (const pixelIdx of pixels) {
        const row = Math.floor(pixelIdx / grid.width);
        const col = pixelIdx % grid.width;
        const x = grid.originX + col * grid.px_mm;
        const z = grid.originZ + row * grid.px_mm;
        highlightData.push({ layer, x, z });
      }
    }

    return {
      island,
      highlightData,
    };
  }, [islandId, scanResults]);
}

/**
 * Example: Island Color Mapper
 * 
 * Generates unique colors for each island for visualization.
 */
export function useIslandColors(scanResults: ScanResults | null) {
  return useMemo(() => {
    if (!scanResults) return new Map<number, string>();

    const colorMap = new Map<number, string>();
    const hueStep = 360 / scanResults.islands.length;

    scanResults.islands.forEach((island, index) => {
      const hue = (index * hueStep) % 360;
      const saturation = island.status === 'active' ? 70 : 40;
      const lightness = 50;
      colorMap.set(island.id, `hsl(${hue}, ${saturation}%, ${lightness}%)`);
    });

    return colorMap;
  }, [scanResults]);
}

/**
 * Example CSS for styling
 */
const exampleStyles = `
.island-visualization {
  padding: 1rem;
  max-height: 600px;
  overflow-y: auto;
}

.summary {
  display: flex;
  gap: 1rem;
  margin-bottom: 1rem;
  padding: 0.5rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 4px;
}

.island-list {
  margin-bottom: 1.5rem;
}

.island-item {
  padding: 0.75rem;
  margin-bottom: 0.5rem;
  background: rgba(255, 255, 255, 0.03);
  border-left: 3px solid transparent;
  cursor: pointer;
  transition: all 0.2s;
}

.island-item:hover {
  background: rgba(255, 255, 255, 0.08);
}

.island-item.selected {
  border-left-color: #3b82f6;
  background: rgba(59, 130, 246, 0.1);
}

.island-item.merged {
  opacity: 0.7;
}

.island-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.5rem;
}

.merge-badge {
  font-size: 0.75rem;
  padding: 0.25rem 0.5rem;
  background: rgba(34, 197, 94, 0.2);
  border-radius: 3px;
  color: #22c55e;
}

.merge-info {
  font-size: 0.75rem;
  color: #94a3b8;
}

.island-details {
  font-size: 0.875rem;
  color: #94a3b8;
}

.children {
  margin-top: 0.5rem;
  padding-top: 0.5rem;
  border-top: 1px solid rgba(255, 255, 255, 0.1);
}
`;
