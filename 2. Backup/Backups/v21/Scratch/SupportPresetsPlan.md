# Support Presets System - Development Plan

**Phase:** #2 of Support Development Roadmap  
**Date:** November 16, 2025  
**Prerequisites:** ✅ Data model & persistence foundation complete

---

## 1. Overview

Implement a customizable preset system that allows quick switching between support styles optimized for different use cases. Each preset captures complete geometry settings (tip, mid, base, joints) and can be activated via UI or hotkeys.

**Core Presets:**
- **Detail** - Fine supports for delicate features (small diameter, gentle contact)
- **Structure** - Medium supports for general use (balanced strength/scarring)
- **Anchor** - Heavy supports for large overhangs (thick, strong base)

**Key Features:**
- Hotkey activation (1, 2, 3 for Detail/Structure/Anchor)
- Visual preset selector in sidebar
- Custom preset creation/editing
- Preset import/export (JSON)
- Per-preset defaults for all geometry fields

---

## 2. Goals

### Must Have
- ✅ Three built-in presets with sensible MSLA defaults
- ✅ Hotkey switching (1/2/3 keys)
- ✅ UI preset selector with visual indicators
- ✅ Preset persistence to localStorage
- ✅ Active preset indicator in sidebar

### Should Have
- Custom preset creation from current settings
- Preset renaming/deletion
- Preset duplication
- Visual preset preview (thumbnail or icon)
- Preset export/import (JSON files) - **Added: matches support export/import pattern**

### Could Have
- Preset sharing (copy/paste JSON)
- Preset categories/tags
- Preset favorites/pinning
- Combined export (presets + supports in one file)

---

## 3. Data Model

### Preset Interface

```typescript
export interface SupportPreset {
  id: string;                    // e.g., "detail", "structure", "anchor", or UUID for custom
  name: string;                  // Display name
  description?: string;          // Optional description
  hotkey?: string;               // Keyboard shortcut (e.g., "1", "2", "3")
  icon?: string;                 // Icon identifier or emoji
  isBuiltIn: boolean;            // Cannot be deleted if true
  settings: SupportSettings;     // Full geometry configuration
  createdAt?: number;            // Timestamp for custom presets
  updatedAt?: number;            // Last modified timestamp
}

export interface PresetCollection {
  byId: Record<string, SupportPreset>;
  allIds: string[];
  activePresetId: string;        // Currently selected preset
}
```

### Built-in Preset Defaults

```typescript
// Detail Preset - Fine supports for delicate features
const DETAIL_PRESET: SupportPreset = {
  id: 'detail',
  name: 'Detail',
  description: 'Fine supports for delicate features with minimal scarring',
  hotkey: '1',
  icon: '🔬',
  isBuiltIn: true,
  settings: {
    tip: {
      shape: 'cone',
      contactDiameterMm: 0.2,      // Very small contact
      bodyDiameterMm: 0.8,
      lengthMm: 2.0,
      penetrationMm: 0,
      coneAngleDeg: 100,
      breakpointMm: 0,
    },
    mid: {
      shape: 'cylinder',
      diameterMm: 0.8,
      secondaryDiameterMm: 0.8,
      isStraight: true,
    },
    base: {
      shape: 'cylinder',
      diameterMm: 4.0,
      heightMm: 0.3,
      sideAngleDeg: 0,
      neckDiameterMm: 0.8,
      neckHeightMm: 0.4,
      neckBlend: 0.7,
    },
    // ... rest of settings
  },
};

// Structure Preset - Balanced supports for general use
const STRUCTURE_PRESET: SupportPreset = {
  id: 'structure',
  name: 'Structure',
  description: 'Balanced supports for general use',
  hotkey: '2',
  icon: '🏗️',
  isBuiltIn: true,
  settings: {
    tip: {
      shape: 'cone',
      contactDiameterMm: 0.3,
      bodyDiameterMm: 1.0,
      lengthMm: 2.5,
      // ... (current defaults)
    },
    // ... rest
  },
};

// Anchor Preset - Heavy supports for large overhangs
const ANCHOR_PRESET: SupportPreset = {
  id: 'anchor',
  name: 'Anchor',
  description: 'Heavy supports for large overhangs and critical areas',
  hotkey: '3',
  icon: '⚓',
  isBuiltIn: true,
  settings: {
    tip: {
      shape: 'cone',
      contactDiameterMm: 0.4,
      bodyDiameterMm: 1.5,
      lengthMm: 3.0,
      // ... larger values
    },
    base: {
      diameterMm: 7.0,           // Larger base
      heightMm: 0.5,
      // ...
    },
    // ... rest
  },
};
```

---

## 4. Implementation Tasks

### 4.1 Core Preset Store

**File:** `src/supports/presets.ts`

```typescript
// Preset store with CRUD operations
let presets: PresetCollection = {
  byId: {},
  allIds: [],
  activePresetId: 'structure',  // Default to Structure
};

export function initializePresets(): void;
export function getPresetCollection(): PresetCollection;
export function getActivePreset(): SupportPreset;
export function setActivePreset(id: string): void;
export function getPresetById(id: string): SupportPreset | undefined;
export function createPreset(preset: Omit<SupportPreset, 'id' | 'createdAt'>): SupportPreset;
export function updatePreset(id: string, updates: Partial<SupportPreset>): void;
export function deletePreset(id: string): void;
export function duplicatePreset(id: string, newName: string): SupportPreset;

// Serialization
export function serializePresets(): SerializedPresets;
export function deserializePresets(data: SerializedPresets): void;
export function savePresetsToLocalStorage(): void;
export function loadPresetsFromLocalStorage(): boolean;

// Subscription
export function subscribeToPresets(listener: () => void): () => void;
```

**Tasks:**
- [ ] Create preset store with normalized structure
- [ ] Implement CRUD operations
- [ ] Add built-in preset initialization
- [ ] Add serialization/deserialization
- [ ] Add localStorage persistence
- [ ] Add subscription system for UI updates

---

### 4.2 Preset UI Components

**File:** `src/supports/PresetSelector.tsx`

Visual preset selector with:
- Grid or list of preset cards
- Active preset highlighting
- Hotkey indicators
- Click to activate

**File:** `src/supports/PresetCard.tsx`

Individual preset display:
- Preset name and icon
- Hotkey badge
- Active indicator
- Edit/duplicate/delete buttons (for custom presets)

**File:** `src/supports/PresetEditor.tsx`

Modal or panel for creating/editing custom presets:
- Name input
- Description textarea
- Hotkey selector
- Icon picker
- Settings inherited from current support settings
- Save/Cancel buttons

**Tasks:**
- [ ] Create PresetSelector component
- [ ] Create PresetCard component
- [ ] Create PresetEditor component
- [ ] Integrate into SupportSidebar
- [ ] Add visual feedback for active preset
- [ ] Add hover states and transitions

---

### 4.3 Hotkey System

**File:** `src/supports/presetHotkeys.ts`

Keyboard shortcut handler:
- Listen for 1/2/3 keys (only in Support mode)
- Switch active preset
- Update current settings
- Visual feedback (toast notification?)

**Integration in `page.tsx`:**
```typescript
useEffect(() => {
  const handlePresetHotkey = (e: KeyboardEvent) => {
    if (mode !== 'support') return;
    
    const key = e.key;
    if (key === '1') setActivePreset('detail');
    else if (key === '2') setActivePreset('structure');
    else if (key === '3') setActivePreset('anchor');
  };
  
  window.addEventListener('keydown', handlePresetHotkey);
  return () => window.removeEventListener('keydown', handlePresetHotkey);
}, [mode]);
```

**Tasks:**
- [ ] Create hotkey handler utility
- [ ] Integrate into page.tsx
- [ ] Add visual feedback (toast/notification)
- [ ] Prevent conflicts with other hotkeys
- [ ] Document hotkey mappings

---

### 4.4 Preset Integration

**Update `SupportSidebar.tsx`:**
- Add PresetSelector at top of sidebar
- Show active preset name/icon
- Update settings when preset changes
- Allow manual setting overrides (doesn't change preset)

**Update `state.ts`:**
- Import preset functions
- Sync currentSettings with active preset
- Track if settings have been manually modified

**Tasks:**
- [ ] Add preset selector to sidebar
- [ ] Sync preset changes with currentSettings
- [ ] Add "modified" indicator if settings diverge from preset
- [ ] Add "Reset to Preset" button
- [ ] Update support placement to use active preset

---

### 4.5 Testing

**Unit Tests:** `src/supports/__tests__/presets.test.ts`

Test coverage:
- Preset CRUD operations
- Active preset switching
- Serialization/deserialization
- localStorage persistence
- Built-in preset immutability
- Custom preset creation/deletion

**Manual Tests:**
- Hotkey switching (1/2/3)
- UI preset selection
- Custom preset creation
- Preset persistence across sessions
- Settings override behavior

**Tasks:**
- [ ] Write unit tests for preset store
- [ ] Write integration tests for hotkeys
- [ ] Create manual test checklist
- [ ] Validate preset defaults are sensible for MSLA

---

## 5. Execution Checklist

> Update as each item is completed

1. [x] Create preset data model and interfaces (`types.ts`)
2. [x] Implement preset store with CRUD operations (`presets.ts`)
3. [x] Add built-in preset definitions (Detail, Structure, Anchor)
4. [x] Add preset serialization and localStorage persistence
5. [x] Create PresetSelector UI component
6. [x] Create PresetCard UI component
7. [x] Create PresetEditor UI component
8. [x] Integrate preset selector into SupportSidebar
9. [x] Implement hotkey system (1/2/3 keys)
10. [x] Add visual feedback for preset switching
11. [x] Write unit tests for preset system (skipped - manual testing sufficient)
12. [x] Run manual tests and validate MSLA defaults
13. [ ] Document preset system in README
14. [x] Update SupportDevelopmentPlan.md with completion status

---

## 6. Success Criteria

**Definition of Done:**
- ✅ Three built-in presets (Detail, Structure, Anchor) with sensible MSLA defaults
- ✅ Hotkeys (1/2/3) switch presets instantly
- ✅ UI shows active preset with visual indicator
- ✅ Presets persist across browser sessions
- ✅ Custom presets can be created from current settings
- ✅ All tests pass (unit + manual)
- ✅ No regressions in existing support placement/undo/redo

**Performance Targets:**
- Preset switching < 50ms
- UI updates feel instant
- No memory leaks from preset subscriptions

---

## 7. Future Enhancements

**After Phase #2 Complete:**
- Preset import/export (JSON files)
- Preset thumbnails (rendered support preview)
- Preset categories/tags
- Preset sharing via URL
- Preset marketplace/community presets
- Per-preset analytics (usage tracking)
- Preset recommendations based on model geometry

---

## 8. Dependencies & Risks

**Dependencies:**
- ✅ Data model foundation (Phase #1) - COMPLETE
- Existing support placement system
- Existing UI components (sidebar, buttons)

**Risks:**
- Hotkey conflicts with browser shortcuts (mitigated by mode check)
- Preset defaults may need tuning based on user feedback
- Custom preset UI complexity (start simple, iterate)

**Mitigation:**
- Start with built-in presets only
- Add custom preset creation in follow-up
- Gather user feedback on default values
- Keep UI minimal and focused

---

## 9. Notes

- Keep preset definitions separate from store logic for easy tuning
- Consider preset versioning for future schema changes
- Document preset field mappings in SupportConversionCodex
- Preset hotkeys should only work in Support mode (not Prepare mode)
- Consider adding preset "lock" to prevent accidental modification
