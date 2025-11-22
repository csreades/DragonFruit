# Support Joints & Grab Tool - Development Plan

**Phase:** #4 of Support Development Roadmap  
**Date:** November 16, 2025  
**Prerequisites:** ✅ Data model foundation, ✅ Preset architecture, ✅ Placement validation

---

## 1. Overview

Implement multi-segment supports with ball joints and an interactive Grab tool for editing. This enables complex support structures that can bend and adapt to model geometry while maintaining structural integrity.

**Core Features:**
- Ball joints connecting support segments
- Grab tool for moving/rotating/scaling supports
- Joint angle constraints and collision detection
- Visual joint indicators
- Blender-style hotkeys (G for grab, R for rotate, S for scale)

---

## 2. Goals

### Must Have
- ✅ Ball joint geometry rendering
- ✅ At least 1 additional joint per support (tip → joint → base)
- ✅ Grab tool to move support tip position
- ✅ Visual feedback during grab operation
- ✅ Hotkey activation (G key for grab mode)

### Should Have
- Multiple joints per support (configurable count)
- Joint rotation constraints (max angle)
- Axis-locked movement (X/Y/Z keys during grab)
- Rotate tool (R key) for joint rotation
- Scale tool (S key) for diameter adjustment
- Joint collision detection with model

### Could Have
- Auto-joint insertion based on support length
- Joint optimization (minimize angles)
- Joint strength analysis
- Visual joint angle indicators
- Joint presets (stiff vs flexible)
- Smooth joint transitions (bezier curves)

---

## 3. Technical Design

### 3.1 Joint Data Model

**Already in schema:**
```typescript
interface SupportSettings {
  jointDefaults: {
    ballDiameterMm: number;      // Size of ball joint
    maxRotationDeg: number;      // Max angle between segments
    maxSlideMm: number;          // Max linear movement
  };
}

interface SupportInstance {
  joints?: SupportJoint[];       // Array of joints along support
}

interface SupportJoint {
  id: string;
  position: { x: number; y: number; z: number };
  rotation?: { x: number; y: number; z: number };
  ballDiameterMm: number;
  parentSegmentId?: string;
  childSegmentId?: string;
}
```

**Joint Placement:**
- Start with 1 joint at midpoint between tip and base
- Joint divides support into 2 segments: tip→joint, joint→base
- Each segment can have its own diameter/shape

### 3.2 Joint Rendering

**Ball Joint Geometry:**
```typescript
// Sphere at joint position
<mesh position={[joint.x, joint.y, joint.z]}>
  <sphereGeometry args={[joint.ballDiameterMm / 2, 16, 16]} />
  <meshStandardMaterial color="#888888" />
</mesh>
```

**Segment Connection:**
- Tip segment: tip → joint (cone/cylinder)
- Base segment: joint → base (cylinder)
- Segments connect to ball joint surface

### 3.3 Grab Tool System

**Tool Modes:**
```typescript
type EditTool = 'none' | 'grab' | 'rotate' | 'scale';

interface EditState {
  tool: EditTool;
  targetSupportId: string | null;
  targetJointId: string | null;
  startPosition: { x: number; y: number; z: number };
  currentPosition: { x: number; y: number; z: number };
  axisLock: 'none' | 'x' | 'y' | 'z';
}
```

**Grab Tool Flow:**
1. Select support (click on it)
2. Press G key → enter grab mode
3. Move mouse → support tip follows cursor (raycasted to model surface)
4. Press X/Y/Z → lock to axis
5. Click → confirm new position
6. ESC → cancel

**Rotate Tool Flow:**
1. Select support
2. Press R key → enter rotate mode
3. Move mouse → rotate joint angle
4. Click → confirm rotation
5. ESC → cancel

**Scale Tool Flow:**
1. Select support
2. Press S key → enter scale mode
3. Move mouse → adjust diameter
4. Click → confirm scale
5. ESC → cancel

### 3.4 Joint Constraints

**Angle Constraint:**
```typescript
function validateJointAngle(
  segmentA: Vector3,
  segmentB: Vector3,
  maxAngleDeg: number
): boolean {
  const angle = segmentA.angleTo(segmentB) * (180 / Math.PI);
  return angle <= maxAngleDeg;
}
```

**Collision Detection:**
- Check if joint position intersects model
- Check if segments intersect model
- Prevent invalid joint placements

---

## 4. Implementation Tasks

### 4.1 Joint Data & Factory

**File:** `src/supports/types.ts` (extend existing)

```typescript
export interface SupportJoint {
  id: string;
  position: { x: number; y: number; z: number };
  rotation?: { x: number; y: number; z: number };
  ballDiameterMm: number;
  parentSegmentId?: string;
  childSegmentId?: string;
}

export function createDefaultJoint(
  position: { x: number; y: number; z: number },
  settings: SupportSettings
): SupportJoint {
  return {
    id: `joint-${Date.now()}`,
    position,
    ballDiameterMm: settings.jointDefaults.ballDiameterMm,
  };
}
```

**Tasks:**
- [ ] Add SupportJoint interface (already exists, verify)
- [ ] Add createDefaultJoint factory function
- [ ] Add joint validation helpers
- [ ] Update SupportInstance to include joints array

---

### 4.2 Joint Rendering

**File:** `src/supports/SupportRenderer.tsx` (update existing)

Add joint rendering between segments:
```typescript
{support.joints?.map((joint) => (
  <mesh key={joint.id} position={[joint.position.x, joint.position.y, joint.position.z]}>
    <sphereGeometry args={[joint.ballDiameterMm / 2, 16, 16]} />
    <meshStandardMaterial 
      color={isSelected ? '#4488ff' : '#888888'}
      metalness={0.5}
      roughness={0.3}
    />
  </mesh>
))}
```

Update segment rendering to connect to joints instead of straight tip→base.

**Tasks:**
- [ ] Add ball joint sphere rendering
- [ ] Split support into segments (tip→joint, joint→base)
- [ ] Calculate segment positions and rotations
- [ ] Add joint hover/selection highlighting
- [ ] Optimize rendering (use InstancedMesh for multiple joints)

---

### 4.3 Joint Creation

**File:** `src/supports/placement.ts` (update existing)

Add joint insertion when creating support:
```typescript
export function createSupportFromRaycast(
  hit: THREE.Intersection,
  settings: SupportSettings,
  plateZ: number
): SupportInstance | null {
  // ... existing tip/base calculation ...
  
  // Add midpoint joint
  const midpoint = {
    x: (tip.x + base.x) / 2,
    y: (tip.y + base.y) / 2,
    z: (tip.z + base.z) / 2,
  };
  
  const joint = createDefaultJoint(midpoint, settings);
  
  return {
    // ... existing fields ...
    joints: [joint],
  };
}
```

**Tasks:**
- [ ] Add joint creation to placement flow
- [ ] Calculate initial joint position (midpoint)
- [ ] Add joint to support instance
- [ ] Update serialization to include joints

---

### 4.4 Grab Tool Implementation

**File:** `src/supports/tools/GrabTool.ts` (new)

```typescript
export class GrabTool {
  private active: boolean = false;
  private targetSupportId: string | null = null;
  private startPosition: Vector3 | null = null;
  private axisLock: 'none' | 'x' | 'y' | 'z' = 'none';
  
  activate(supportId: string): void;
  deactivate(): void;
  setAxisLock(axis: 'x' | 'y' | 'z'): void;
  updatePosition(newPosition: Vector3): void;
  confirm(): void;
  cancel(): void;
}
```

**Tasks:**
- [ ] Create GrabTool class
- [ ] Implement activate/deactivate
- [ ] Implement axis locking (X/Y/Z keys)
- [ ] Implement position update with raycasting
- [ ] Implement confirm/cancel
- [ ] Add visual feedback (ghost preview)

---

### 4.5 Hotkey System

**File:** `src/app/page.tsx` (update existing)

Add tool hotkeys:
```typescript
useEffect(() => {
  const handleToolHotkey = (e: KeyboardEvent) => {
    if (mode !== 'support') return;
    if (!selectedSupportId) return;
    
    // Ignore if typing in input
    const target = e.target as HTMLElement;
    if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA') return;
    
    const key = e.key.toLowerCase();
    
    if (key === 'g') {
      // Enter grab mode
      setEditTool('grab');
    } else if (key === 'r') {
      // Enter rotate mode
      setEditTool('rotate');
    } else if (key === 's') {
      // Enter scale mode
      setEditTool('scale');
    } else if (key === 'escape') {
      // Cancel tool
      setEditTool('none');
    } else if (key === 'x' || key === 'y' || key === 'z') {
      // Axis lock
      setAxisLock(key as 'x' | 'y' | 'z');
    }
  };
  
  window.addEventListener('keydown', handleToolHotkey);
  return () => window.removeEventListener('keydown', handleToolHotkey);
}, [mode, selectedSupportId]);
```

**Tasks:**
- [ ] Add edit tool state (grab/rotate/scale)
- [ ] Add G/R/S hotkey handlers
- [ ] Add X/Y/Z axis lock handlers
- [ ] Add ESC cancel handler
- [ ] Add visual tool indicator

---

### 4.6 Tool UI Feedback

**File:** `src/supports/ToolIndicator.tsx` (new)

Visual indicator showing active tool and instructions:
```tsx
export function ToolIndicator({ tool, axisLock }: { tool: EditTool; axisLock: string }) {
  if (tool === 'none') return null;
  
  return (
    <div className="absolute top-20 left-1/2 -translate-x-1/2 bg-blue-600 text-white px-4 py-2 rounded-lg shadow-lg">
      <div className="font-semibold">
        {tool === 'grab' && 'Grab Mode'}
        {tool === 'rotate' && 'Rotate Mode'}
        {tool === 'scale' && 'Scale Mode'}
      </div>
      <div className="text-xs mt-1">
        {axisLock !== 'none' ? `Locked to ${axisLock.toUpperCase()} axis` : 'Press X/Y/Z to lock axis'}
        • Click to confirm • ESC to cancel
      </div>
    </div>
  );
}
```

**Tasks:**
- [ ] Create ToolIndicator component
- [ ] Show active tool name
- [ ] Show axis lock status
- [ ] Show instructions (click/ESC)
- [ ] Add to page.tsx

---

## 5. Execution Checklist

> Update as each item is completed

### Phase 4A - Basic Joints (MVP)
1. [ ] Add SupportJoint interface validation
2. [ ] Add createDefaultJoint factory function
3. [ ] Update createSupportFromRaycast to add 1 midpoint joint
4. [ ] Update SupportRenderer to render ball joints
5. [ ] Update SupportRenderer to render segmented support (tip→joint→base)
6. [ ] Update serialization to include joints array
7. [ ] Test joint rendering with all 3 presets

### Phase 4B - Grab Tool
8. [ ] Create GrabTool class with activate/deactivate
9. [ ] Add G hotkey to enter grab mode
10. [ ] Implement tip position update via raycasting
11. [ ] Add visual feedback (ghost preview or highlight)
12. [ ] Add click to confirm, ESC to cancel
13. [ ] Update support position in store
14. [ ] Add to undo/redo history

### Phase 4C - Advanced Grab Features
15. [ ] Add X/Y/Z axis locking
16. [ ] Add ToolIndicator UI component
17. [ ] Add grab mode cursor change
18. [ ] Test grab with validation (prevent invalid positions)

### Phase 4D - Rotate & Scale Tools (Optional)
19. [ ] Implement rotate tool (R key)
20. [ ] Implement scale tool (S key)
21. [ ] Add joint angle constraints
22. [ ] Add joint collision detection

---

## 6. Success Criteria

**Definition of Done (Phase 4A - Basic Joints):**
- ✅ Supports render with 1 ball joint at midpoint
- ✅ Joint appears as sphere between tip and base
- ✅ Support segments connect to joint
- ✅ Joints persist in save/load
- ✅ Works with all 3 presets

**Definition of Done (Phase 4B - Grab Tool):**
- ✅ G key activates grab mode on selected support
- ✅ Mouse movement updates support tip position
- ✅ Click confirms new position
- ✅ ESC cancels grab operation
- ✅ Validation prevents invalid placements
- ✅ Changes added to undo/redo history

**Performance Targets:**
- Joint rendering < 5ms per support
- Grab tool updates < 16ms (60fps)
- No lag during mouse movement

---

## 7. Future Enhancements

**After Phase 4 Complete:**
- Multiple joints per support (2-5 joints)
- Auto-joint insertion based on support length
- Joint optimization (minimize angles for strength)
- Joint strength analysis and warnings
- Smooth joint transitions (bezier curves)
- Joint presets (stiff vs flexible)
- Visual joint angle indicators
- Joint snapping to model surface

---

## 8. Dependencies & Risks

**Dependencies:**
- ✅ Support store with undo/redo (Phase 1) - COMPLETE
- ✅ Support rendering system (Phase 1) - COMPLETE
- ✅ Support selection system (Phase 1) - COMPLETE
- Raycasting system (already exists)
- Three.js geometry utilities

**Risks:**
- Joint rendering performance with many supports (mitigated by InstancedMesh)
- Complex segment rotation calculations (start simple, iterate)
- Grab tool UX complexity (follow Blender patterns)
- Joint angle constraints may be too restrictive (make configurable)

**Mitigation:**
- Start with 1 joint per support
- Use simple sphere geometry for joints
- Follow established Blender hotkey patterns
- Make all constraints configurable in settings

---

## 9. Notes

- Keep joint system simple initially - 1 joint per support is enough for MVP
- Grab tool is highest priority - enables manual support adjustment
- Rotate/Scale tools can be added later if needed
- Joint collision detection can be deferred to later phase
- Consider adding "straighten support" tool to remove joints
- Joint system should work seamlessly with existing validation
- All joint operations should be undoable
