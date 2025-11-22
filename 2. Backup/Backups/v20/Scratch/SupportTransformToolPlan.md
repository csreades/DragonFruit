# Support Transform Tool - Detailed Design & Brainstorm

**Focus:** Interactive support editing with Blender-style transform tools  
**Date:** November 16, 2025  
**Status:** Design & Brainstorming Phase

---

## 1. Core Concept

A unified transform system for editing placed supports, inspired by Blender's workflow:
- **Select** a support by clicking on it
- **Activate tool** with hotkey (G/R/S)
- **Transform** by moving mouse
- **Constrain** to axis with X/Y/Z keys
- **Confirm** with click or Enter
- **Cancel** with ESC or right-click

---

## 2. Transform Modes

### Mode 1: Grab (Move) - G Key
**What it does:** Move the support tip to a new position on the model surface

**Behavior:**
- Mouse movement raycasts to model surface
- Support tip follows cursor (snapped to surface)
- Support base stays on build plate
- Support shaft/joints adjust automatically
- Preview shows new position in real-time

**Use cases:**
- Adjust support placement after initial drop
- Move support to better contact point
- Reposition after model rotation

### Mode 2: Rotate - R Key
**What it does:** Rotate the support around its base point

**Behavior:**
- Mouse movement rotates support in 3D
- Base point stays fixed
- Tip moves in arc around base
- Can constrain to specific axis (X/Y/Z)

**Use cases:**
- Angle support for better strength
- Avoid collisions with other supports
- Optimize support direction

### Mode 3: Scale - S Key  
**What it does:** Adjust support diameter (thickness)

**Behavior:**
- Mouse movement scales diameter
- Affects tip, shaft, and base proportionally
- Or: affects only selected segment
- Maintains support position

**Use cases:**
- Make support thicker for heavy areas
- Make support thinner to reduce scarring
- Quick diameter adjustment without sidebar

---

## 3. Key Design Questions

### Q1: What exactly are we transforming?

**Option A: Transform the entire support**
- Move tip position (most common)
- Tip stays on model surface (raycasted)
- Base stays on build plate
- Joints adjust automatically

**Option B: Transform individual components**
- Select tip, joint, or base individually
- Move selected component independently
- Other components adjust to maintain connection
- More complex but more flexible

**Option C: Transform segments**
- Select a segment (tip→joint or joint→base)
- Adjust that segment's angle/length
- Adjacent segments adjust

**Recommendation:** Start with **Option A** (whole support), add Option B later if needed.

---

### Q2: How does the tip stay on the model surface?

**Option A: Continuous raycasting**
- Every mouse move triggers raycast
- Tip snaps to nearest surface point
- Smooth but potentially expensive

**Option B: Raycast on mouse move with throttling**
- Raycast every 16ms (60fps max)
- Smooth enough, better performance
- Use requestAnimationFrame

**Option C: Raycast only on click**
- Show ghost preview during move
- Raycast when user clicks to confirm
- Fast but less intuitive

**Recommendation:** **Option B** - throttled raycasting for smooth feedback without performance hit.

---

### Q3: How do we handle axis constraints?

**Option A: Global axes (world space)**
- X = left/right
- Y = forward/back  
- Z = up/down
- Simple, predictable

**Option B: Local axes (support-relative)**
- X/Y = perpendicular to support
- Z = along support direction
- More intuitive for rotation

**Option C: View-relative axes**
- X/Y = screen space
- Z = toward/away from camera
- Easiest for user to visualize

**Recommendation:** **Option A** for grab (global), **Option B** for rotate (local).

---

### Q4: What's the visual feedback during transform?

**Option A: Ghost preview**
- Original support stays visible (dimmed)
- Ghost support shows new position (bright/highlighted)
- Clear before/after comparison

**Option B: Direct manipulation**
- Support moves in real-time
- No ghost, just the moving support
- Simpler but harder to compare

**Option C: Gizmo/handles**
- 3D arrows/rings for X/Y/Z
- Click and drag handles
- Like Blender/Unity/Unreal

**Recommendation:** **Option A** (ghost) for grab, **Option C** (gizmo) for rotate/scale if we add those.

---

### Q5: How do joints behave during transform?

**Option A: Joints move proportionally**
- If support has 1 joint at midpoint
- Joint stays at midpoint during transform
- Simple, predictable

**Option B: Joints stay fixed in space**
- Joints don't move
- Segments bend/stretch
- More realistic but complex

**Option C: Joints optimize automatically**
- Joints reposition to minimize angles
- AI-assisted placement
- Complex but powerful

**Recommendation:** **Option A** for MVP, consider Option C later.

---

### Q6: What happens if the new position is invalid?

**Option A: Block the transform**
- Preview turns red
- Click does nothing
- User must find valid position

**Option B: Snap to nearest valid position**
- Auto-adjust to closest valid spot
- Always succeeds
- May surprise user

**Option C: Allow but warn**
- Let user place invalid support
- Show warning indicator
- User can fix later

**Recommendation:** **Option A** - consistent with current validation system.

---

## 4. Proposed Implementation Flow

### Grab Tool (G Key) - Detailed Flow

**1. Activation**
```
User clicks support → support selected (highlighted)
User presses G → grab mode activated
UI shows: "Grab Mode - Move mouse to reposition • Click to confirm • ESC to cancel"
Cursor changes to crosshair or move icon
```

**2. Mouse Movement**
```
On mouse move:
  1. Raycast from mouse to model
  2. If hit:
     - Calculate new tip position
     - Validate new position (spacing check)
     - Update ghost preview
     - Color: green if valid, red if invalid
  3. If no hit:
     - Hide ghost preview
     - Show "No surface" indicator
```

**3. Axis Constraint (Optional)**
```
User presses X/Y/Z:
  - Lock movement to that axis
  - UI updates: "Locked to X axis"
  - Mouse movement only affects X coordinate
  - Press same key again to unlock
```

**4. Confirmation**
```
User clicks (or presses Enter):
  - If position is valid:
    - Update support in store
    - Add to undo history
    - Exit grab mode
    - Show success (brief green flash)
  - If position is invalid:
    - Show error toast
    - Stay in grab mode
    - User can try again or ESC
```

**5. Cancellation**
```
User presses ESC (or right-clicks):
  - Discard changes
  - Exit grab mode
  - Support returns to original position
  - No undo entry created
```

---

## 5. Technical Architecture

### State Management

```typescript
// Transform tool state
interface TransformState {
  mode: 'none' | 'grab' | 'rotate' | 'scale';
  targetSupportId: string | null;
  originalSupport: SupportInstance | null; // For cancel/undo
  previewSupport: SupportInstance | null;  // For ghost preview
  axisLock: 'none' | 'x' | 'y' | 'z';
  isValid: boolean; // Validation result
}
```

### Hook Structure

```typescript
function useTransformTool() {
  const [state, setState] = useState<TransformState>(initialState);
  
  const activate = (mode: 'grab' | 'rotate' | 'scale', supportId: string) => {
    // Store original support
    // Enter transform mode
    // Set up event listeners
  };
  
  const updatePreview = (mousePosition: Vector2) => {
    // Raycast to model
    // Calculate new position
    // Validate
    // Update preview
  };
  
  const setAxisLock = (axis: 'x' | 'y' | 'z') => {
    // Toggle axis lock
  };
  
  const confirm = () => {
    // Validate final position
    // Update support in store
    // Add to undo history
    // Exit mode
  };
  
  const cancel = () => {
    // Discard changes
    // Exit mode
  };
  
  return { state, activate, updatePreview, setAxisLock, confirm, cancel };
}
```

### Event Handling

```typescript
// In page.tsx or dedicated component
useEffect(() => {
  if (transformState.mode === 'none') return;
  
  const handleMouseMove = (e: MouseEvent) => {
    // Convert to normalized device coordinates
    // Update preview
  };
  
  const handleClick = (e: MouseEvent) => {
    if (e.button === 0) transformTool.confirm();
    if (e.button === 2) transformTool.cancel();
  };
  
  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') transformTool.cancel();
    if (e.key === 'Enter') transformTool.confirm();
    if (e.key === 'x') transformTool.setAxisLock('x');
    if (e.key === 'y') transformTool.setAxisLock('y');
    if (e.key === 'z') transformTool.setAxisLock('z');
  };
  
  window.addEventListener('mousemove', handleMouseMove);
  window.addEventListener('click', handleClick);
  window.addEventListener('keydown', handleKeyDown);
  
  return () => {
    // Cleanup
  };
}, [transformState.mode]);
```

---

## 6. Visual Design

### Ghost Preview
```typescript
// Render both original (dimmed) and preview (highlighted)
{transformState.mode !== 'none' && transformState.originalSupport && (
  <SupportRenderer
    support={transformState.originalSupport}
    opacity={0.3}
    color="#666666"
  />
)}

{transformState.previewSupport && (
  <SupportRenderer
    support={transformState.previewSupport}
    opacity={0.8}
    color={transformState.isValid ? '#00ff00' : '#ff0000'}
    isGhost={true}
  />
)}
```

### UI Indicator
```typescript
{transformState.mode !== 'none' && (
  <div className="absolute top-20 left-1/2 -translate-x-1/2 bg-blue-600 text-white px-4 py-2 rounded-lg">
    <div className="font-semibold">
      {transformState.mode === 'grab' && 'Grab Mode'}
      {transformState.mode === 'rotate' && 'Rotate Mode'}
      {transformState.mode === 'scale' && 'Scale Mode'}
    </div>
    <div className="text-xs">
      {transformState.axisLock !== 'none' 
        ? `Locked to ${transformState.axisLock.toUpperCase()} axis`
        : 'Press X/Y/Z to lock axis'
      }
      • Click to confirm • ESC to cancel
    </div>
  </div>
)}
```

---

## 7. Open Questions for Discussion

1. **Should grab mode work on any part of the support, or only the tip?**
   - Option A: Click anywhere on support, always moves tip
   - Option B: Click tip to move tip, click base to move base, etc.

2. **Should we show a grid or snap points during grab?**
   - Could help with precise placement
   - Might clutter the view

3. **Should axis lock be sticky (persists) or per-operation?**
   - Sticky: Once you press X, stays locked until you press X again
   - Per-op: Resets when you confirm/cancel

4. **Should we allow grab mode while placing a new support?**
   - Could enable "place and adjust" workflow
   - Might be confusing

5. **Should validation be real-time or only on confirm?**
   - Real-time: Preview turns red immediately
   - On confirm: Only check when user clicks

6. **Should we support multi-select and transform multiple supports at once?**
   - Powerful but complex
   - Defer to later phase?

7. **Should the camera orbit be disabled during transform?**
   - Prevents accidental camera movement
   - But might be annoying if user wants to orbit

8. **Should we show distance/angle measurements during transform?**
   - Helpful for precision
   - Might clutter UI

---

## 8. Recommended Starting Point

**Phase 4A: Basic Grab Tool (Minimum Viable)**

1. Select support by clicking
2. Press G to enter grab mode
3. Mouse movement raycasts to model, updates tip position
4. Preview shows new position (green/red based on validation)
5. Click to confirm, ESC to cancel
6. No axis locking (add later)
7. No rotate/scale (add later)
8. No multi-select (add later)

**This gives us:**
- Core transform infrastructure
- Real-time preview system
- Validation integration
- Undo/redo integration
- Foundation for more complex tools

**Estimated complexity:** Medium
**Estimated time:** 2-3 hours implementation + testing

---

## 9. Next Steps

Before implementing, we should decide on:
1. ✅ Transform target (whole support vs components) → **Whole support**
2. ✅ Raycast strategy (continuous vs throttled) → **Throttled**
3. ✅ Visual feedback (ghost vs direct) → **Ghost preview**
4. ✅ Invalid position handling (block vs snap vs warn) → **Block**
5. ⚠️ Axis locking behavior (sticky vs per-op) → **Need decision**
6. ⚠️ Camera behavior during transform → **Need decision**
7. ⚠️ Click target (anywhere vs specific parts) → **Need decision**

Once these are decided, we can proceed with implementation!
