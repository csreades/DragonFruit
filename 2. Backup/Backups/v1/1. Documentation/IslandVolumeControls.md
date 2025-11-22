# Island Volume Controls

## Overview

`IslandVolumeControls` is a dedicated control panel for visualizing island volumes with color-by-island functionality. This is separate from `IslandOverlayControls` (which handles soft-brush overlay visualization).

## Location

`src/components/controls/IslandVolumeControls.tsx`

## Features

### 1. **Enable/Disable Toggle**
- Show/hide island volume visualization
- Independent from other overlay modes

### 2. **Summary Statistics**
- Total island count
- Active islands count
- Quick overview at a glance

### 3. **Color Schemes**
Four different coloring modes:

- **Unique per Island**: Each island gets a unique color (default)
- **By Lifecycle**: Color by status (active vs. merged)
- **By Volume**: Heatmap based on total area
- **By Layer Span**: Color by how many layers the island spans

### 4. **Opacity Control**
- Slider to adjust island volume transparency (10-100%)
- Allows seeing the underlying mesh

### 5. **Show Merged Toggle**
- Toggle visibility of completed/merged islands
- Helps focus on active islands only

### 6. **Island Search & Sort**
- **Search**: Filter islands by ID
- **Sort by**:
  - ID (ascending)
  - Volume (descending - largest first)
  - Layers (descending - longest span first)

### 7. **Island List**
Scrollable list showing:
- Island ID
- Status badges (Merged, +N for absorbed islands)
- Layer range (L0–L50)
- Total area in mm²
- Merge relationships (parent/child info)

### 8. **Island Selection**
- Click any island to highlight it in 3D
- Selected island shows with blue border
- Click again or "Clear Selection" to deselect

## Props Interface

```typescript
type IslandVolumeControlsProps = {
  enabled: boolean;                    // Show/hide visualization
  onEnabledChange: (enabled: boolean) => void;
  
  islands: Island[];                   // All islands from scan
  selectedIslandId: number | null;     // Currently selected island
  onSelectIsland: (id: number | null) => void;
  
  colorScheme: ColorScheme;            // 'unique' | 'lifecycle' | 'volume' | 'layers'
  onColorSchemeChange: (scheme: ColorScheme) => void;
  
  showMerged: boolean;                 // Include merged islands
  onShowMergedChange: (show: boolean) => void;
  
  opacity: number;                     // 0.1 to 1.0
  onOpacityChange: (opacity: number) => void;
};
```

## Usage Example

```tsx
import { IslandVolumeControls } from '@/components/controls/IslandVolumeControls';
import { useState } from 'react';

function MyApp() {
  const [volumeEnabled, setVolumeEnabled] = useState(false);
  const [selectedIsland, setSelectedIsland] = useState<number | null>(null);
  const [colorScheme, setColorScheme] = useState<ColorScheme>('unique');
  const [showMerged, setShowMerged] = useState(false);
  const [opacity, setOpacity] = useState(0.7);

  return (
    <div className="sidebar">
      <IslandVolumeControls
        enabled={volumeEnabled}
        onEnabledChange={setVolumeEnabled}
        islands={scanResults?.islands || []}
        selectedIslandId={selectedIsland}
        onSelectIsland={setSelectedIsland}
        colorScheme={colorScheme}
        onColorSchemeChange={setColorScheme}
        showMerged={showMerged}
        onShowMergedChange={setShowMerged}
        opacity={opacity}
        onOpacityChange={setOpacity}
      />
    </div>
  );
}
```

## Integration with Visualization

When the user selects an island or changes settings, you'll need to:

1. **Update mesh colors** based on `colorScheme`
2. **Highlight selected island** using `selectedIslandId`
3. **Apply opacity** to island volumes
4. **Filter visibility** based on `showMerged`

### Example: Applying Colors

```typescript
// Generate color map based on scheme
function getIslandColor(island: Island, scheme: ColorScheme): string {
  switch (scheme) {
    case 'unique':
      // Unique hue per island
      const hue = (island.id * 137.5) % 360;
      return `hsl(${hue}, 70%, 50%)`;
    
    case 'lifecycle':
      return island.status === 'active' ? '#22c55e' : '#f97316';
    
    case 'volume':
      // Heatmap based on area (you'd normalize this)
      const normalized = island.totalAreaMm2 / maxArea;
      return getHeatmapColor(normalized);
    
    case 'layers':
      const span = island.lastLayer - island.firstLayer;
      const spanNorm = span / maxSpan;
      return getHeatmapColor(spanNorm);
  }
}

// Apply to mesh
for (const island of islands) {
  if (!showMerged && island.status === 'complete') continue;
  
  const color = getIslandColor(island, colorScheme);
  const isSelected = island.id === selectedIslandId;
  const finalOpacity = isSelected ? 1.0 : opacity;
  
  paintIslandVolume(island, color, finalOpacity);
}
```

## Visual Design

- **Dark theme**: Matches existing UI (neutral-800 background)
- **Compact layout**: Fits in sidebar alongside other controls
- **Scrollable list**: Handles many islands gracefully
- **Clear visual hierarchy**: Headers, sections, badges
- **Interactive feedback**: Hover states, selection highlighting

## Differences from IslandOverlayControls

| Feature | IslandOverlayControls | IslandVolumeControls |
|---------|----------------------|---------------------|
| Purpose | Soft-brush overlay at island bases | Full 3D volume visualization |
| Brush Size | ✅ Yes | ❌ No |
| Color Schemes | ❌ Single color | ✅ Multiple schemes |
| Island List | ❌ No | ✅ Yes |
| Island Selection | ❌ No | ✅ Yes |
| Search/Sort | ❌ No | ✅ Yes |
| Merge Info | ❌ No | ✅ Yes |
| Taper Control | ✅ Yes | ❌ No |

## Future Enhancements

1. **Export island data**: Download CSV with island statistics
2. **Color customization**: Custom color palettes
3. **Multi-select**: Highlight multiple islands at once
4. **Island comparison**: Side-by-side stats for selected islands
5. **Volume calculation**: Show actual mm³ volume (requires layer height)
6. **Island filtering**: By volume range, layer range, etc.
