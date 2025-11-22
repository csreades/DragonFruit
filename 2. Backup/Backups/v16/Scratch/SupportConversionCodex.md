# Lychee ↔ AutoSupport Conversion Codex

> Living document that maps Lychee support structures to AutoSupport concepts for future import/export tooling.

---

## 1. Shared Fundamentals

| Concept | Lychee Representation | AutoSupport Representation | Notes |
| --- | --- | --- | --- |
| Units | Millimeters throughout (`settings.supports`, per-instance `tip/base`) | Millimeters (consistent with slicer) | Ensure no implicit unit conversion during import/export. |
| Scene Collections | `*.present.byId` + `*.present.allIds` | Same normalized pattern | Keep IDs stable for round-tripping. |
| Coordinate System | World positions in Lychee scene space (typically Z-up) | Z-up (matching our viewer) | Apply any scene transforms before writing. |

---

## 2. Geometry Presets (Support Styles)

Lychee block: `settings.supports.*` (`supportLight`, `supportMedium`, etc.) and `supportsPresets.presets.spX.settings`.

| Section | Lychee Fields | AutoSupport Target | Conversion Details |
| --- | --- | --- | --- |
| `adaptiveBase` | Bool | `supportPreset.adaptiveBase` | Controls adaptive sizing. |
| `tip` | `type`, `pointDiameter`, `penetration`, `diameter`, `length`, `breakPoint`, `angle` | `supportPreset.tip` | Copy 1:1. For limited UI exposing subset, keep hidden fields stored. |
| `mid` | `type` | `supportPreset.mid` + optional diameter | Lychee lacks explicit diameter; we may store separately or infer from base tip. |
| `baseTip` | Same schema as tip + `isStraight` | `supportPreset.baseTip` | Align `isStraight`. |
| `base` | `type`, `diameter`, `length`, `angle`, `joinDiameter`, `joinLength`, `joinCone` | `supportPreset.base` | Map join fields exactly. |
| `extra` | `tipDiameter2`, `baseDiameter2`, etc. | `supportPreset.extra` | Preserve even if unused initially. |
| `isStraight` | Bool | `supportPreset.isStraight` | Distinguishes overall straight vs angled supports. |

**Favorites and names:** `supportsPresets.favorites` ↔ AutoSupport preset bookmarks; `name` values become user-facing preset labels.

---

## 3. Bracing & Auto-Support Settings

| Lychee Field | AutoSupport Field | Notes |
| --- | --- | --- |
| `bracingPreset.settings.bpX` (pattern, gaps, heights) | `bracingProfiles[id]` | Preserve heights (`firstCathedralHeight`, `secondCathedralHeight`, `zStartPoint`), per-tier settings (`bottom/middle/top`). |
| `supportCreator` | `autoSupport.settings` | Includes `onlyIsland`, `autoBracing`, `interval`, `minBounds`, `autoParenting`, `maxMidLength`, `presetMinima`, `presetAutoSupportCustom.resolutionXY/Z`, `autoLift`. |
| `supportMode` / `supportModeFDM` | `autoSupport.mode` | Document which modes we support. |
| `islandsDetector` presets | `islandScanPresets` | Map `fast/normal/detailed` XY/Z resolutions. |

---

## 4. Support Instances (`supports.present.byId`)

Each Lychee support instance maps to an AutoSupport `SupportInstance`.

| Field | Lychee Key | AutoSupport Key | Notes |
| --- | --- | --- | --- |
| ID | `id` (`"s410"`) | `support.id` | Preserve string IDs or convert to UUID but keep original for export. |
| Tip attachment | `objectIdTip` | `support.objectIdTip` | References object mesh ID. |
| Tip position | `tip { x, y, z }` | `support.tip.position` | Float mm coordinates. |
| Tip normal | `tipNormal { x, y, z }` | `support.tip.normal` | Use normalized vector. `newTipNormal` optional override. |
| Base attachment | `objectIdBase` | `support.objectIdBase` | Plate or object. |
| Base position | `base { x, y, z }` | `support.base.position` | For plate attachments, z ~ 0. |
| Base normal | `baseNormal { x, y, z }` | `support.base.normal` | Often `{0,0,1}` for plate. |
| Geometry settings | `settings` block | `support.settings` | Copy entire structure (tip/base/baseTip/mid/extra/etc.). |
| Hierarchy | `parentBaseId`, `parentTipId`, `parentId[]` | `support.parents.base`, `support.parents.tip`, `support.parents.all` | Supports bracing/stacking. |
| Metadata | `isBaseTip`, `isInFill`, `gridNodeIndex`, `type`, `group`, `mini`, `vertical`, `straight`, `collisionIsAccepted`, `isCollidingWithObject`, `tags`, `updatedAt`, `isVisible` | Store in `support.meta` to keep parity. |

**Coordinate transforms:** when exporting, ensure AutoSupport support positions are transformed into Lychee’s scene coordinates (account for object transforms, plate offsets).

---

## 5. Objects, Plates, and Minima References

| Lychee Block | Purpose | Conversion Strategy |
| --- | --- | --- |
| `objects.present.byId` | Source meshes; includes `supportsBase` arrays referencing support IDs | On import, attach supports to matching objects. On export, fill `supportsBase`. |
| `plates.present.byId` | Build plate geometry & size | Map to AutoSupport plate definition; needed for base projection. |
| `minimas.present.byId` | Detected unsupported islands | Align with our island scan results; may enrich with additional analytics but keep `objectId`, `zPosition`, `area`, `angle`, `hasSupport`. |

---

## 6. Conversion Workflow Notes

1. **Import Lychee → AutoSupport**
   - Parse presets, bracing profiles, auto-support settings first.
   - Load objects/plates to build lookup tables.
   - Convert each support instance into `SupportInstance`, storing untouched geometry fields + metadata.
   - Optional: deduplicate settings to reference presets for UI convenience while retaining per-instance overrides.

2. **AutoSupport authoring → Lychee-compatible export**
   - Ensure every support has complete geometry fields (fill defaults where our UI omits values).
   - Serialize to `supports.present.byId`/`allIds`, referencing existing object IDs.
   - Rebuild `supportsPresets` if we expose custom styles (or embed per-instance settings only).
   - Output bracing + auto-support settings if needed; otherwise keep original values or mark as defaults.

3. **Extensibility**
   - Any AutoSupport-only enhancements (analytics, annotations, deformable bases) should live in separate namespaces so exports remain clean.
   - Version the codex and conversion tooling so future schema changes (e.g., Lychee updates) can be tracked.

---

## 7. Open Questions / TODOs

1. Confirm Lychee’s `type` field semantics (observed `1` = standard resin support). Need mapping list for other types.
2. Determine how Lychee stores bracing instances (if separate from supports) for full round-trip.
3. Validate coordinate transform order when objects are rotated/scaled prior to support placement.
4. Decide whether to infer presets from per-instance settings or always store a local copy.
5. Investigate `gridNodeIndex` usage—whether required for export or optional.

(Keep updating this codex as we uncover more fields or finalize our internal schemas.)
