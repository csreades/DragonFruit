import { useState, useCallback, useMemo } from 'react';
import * as THREE from 'three';
import { runIslandScan, runScanlineScan, type ScanResults } from './ScanOrchestrator';
import { runIslandScanNative } from './nativeIslandScan';
import { computeIslandMarkers, type IslandMarker } from './islandOverlayLogic';
import type { GeometryWithBounds } from '@/hooks/useStlGeometry';
import { quaternionFromGlobalEuler } from '@/utils/rotation';

interface TransformState {
  position: THREE.Vector3;
  rotation: THREE.Euler;
  scale: THREE.Vector3;
}

interface IslandManagerProps {
  geom: GeometryWithBounds | null;
  transform: TransformState;
  layerHeightMm: number; // Passed from slicing manager
}

export function useIslandManager({ geom, transform, layerHeightMm }: IslandManagerProps) {
  // Scanning State
  const [scanning, setScanning] = useState<boolean>(false);
  const [scanProgress, setScanProgress] = useState<{ done: number; total: number } | null>(null);
  const [scanData, setScanData] = useState<ScanResults | null>(null);
  const [scanBBox, setScanBBox] = useState<THREE.Box3 | null>(null);

  // Scan Parameters
  const [pxMm, setPxMm] = useState<number>(0.10);
  const [supportBufMm, setSupportBufMm] = useState<number>(0.6);
  const [connectivity, setConnectivity] = useState<4 | 8>(4);
  const [minIslandAreaMm2, setMinIslandAreaMm2] = useState<number>(0);
  const [minOverlapPx, setMinOverlapPx] = useState<number>(4);
  const [overlapNeighborhoodPx, setOverlapNeighborhoodPx] = useState<number>(1);
  const [useSurfaceContiguity, setUseSurfaceContiguity] = useState<boolean>(false);

  // Native (Rust) toggle
  const [useNativeScan, setUseNativeScan] = useState<boolean>(true);

  // UI State
  const [scanCardExpanded, setScanCardExpanded] = useState<boolean>(true);

  // Overlay State. enabled + support coverage default ON so users see the
  // feature without having to discover the toggles.
  const [overlayEnabled, setOverlayEnabled] = useState<boolean>(true);
  // Default blue picked for visibility against the typical pink/magenta
  // model colour and against the orange supports.
  const [overlayColor, setOverlayColor] = useState<string>('#0433FF');
  const [overlayOpacity, setOverlayOpacity] = useState<number>(1.0);
  const [showIslandIdLabels, setShowIslandIdLabels] = useState<boolean>(false);
  const [overlayTaper, setOverlayTaper] = useState<number>(0.60);
  // Halo shader state (primary cognitive surface).
  const [overlayHaloIntensity, setOverlayHaloIntensity] = useState<number>(0.7);
  const [overlayHaloPulseEnabled, setOverlayHaloPulseEnabled] = useState<boolean>(true);
  const [showSupportVolumeHalo, setShowSupportVolumeHalo] = useState<boolean>(true);
  const [supportVolumeHaloIntensity, setSupportVolumeHaloIntensity] = useState<number>(0.7);

  // Island column highlight (SoftClayMaterial shader path).
  const [showIslands, setShowIslands] = useState<boolean>(true);
  // Cyan + yellow chosen to contrast against typical pink/red resin
  // models (Mag, 2026-05-18). Warm colours muddy against warm models.
  const [islandColor, setIslandColor] = useState<string>('#00E5FF');
  const [islandIntensity, setIslandIntensity] = useState<number>(0.85);
  const [islandRadiusFactor, setIslandRadiusFactor] = useState<number>(3.0);
  const [islandColumnHeight, setIslandColumnHeight] = useState<number>(6.0);

  // Overhang highlight (only fires near an island marker — problem zones).
  const [showOverhang, setShowOverhang] = useState<boolean>(true);
  const [overhangColor, setOverhangColor] = useState<string>('#FFEB3B');
  const [overhangAngleDeg, setOverhangAngleDeg] = useState<number>(45);
  const [overhangIntensity, setOverhangIntensity] = useState<number>(0.7);
  const [overhangProximityMm, setOverhangProximityMm] = useState<number>(8.0);

  // Selection State
  const [selectedIslandId, setSelectedIslandId] = useState<number | null>(null);
  const [showMerged, setShowMerged] = useState<boolean>(false);

  // Voxel State
  const [voxelEnabled, setVoxelEnabled] = useState<boolean>(false);
  const [voxelColorScheme, setVoxelColorScheme] = useState<'unique' | 'lifecycle' | 'height'>('unique');
  const [voxelOpacity, setVoxelOpacity] = useState<number>(1.0);
  const [voxelShowMerged, setVoxelShowMerged] = useState<boolean>(true);
  const [voxelShowTerritory, setVoxelShowTerritory] = useState<boolean>(false);

  // Helper to prepare geometry for scanning
  const prepareTransformedGeom = useCallback(() => {
    if (!geom) return null;

    const transformedGeom = geom.geometry.clone();

    // Center offset (negate to move geometry)
    // Note: geom.bbox is world-aligned but geom.geometry might need centering if it wasn't baked
    // In page.tsx: 
    // const bbox = geom.geometry.boundingBox ...
    // const centerOffset = bbox.getCenter(...)
    // transformedGeom.translate(-centerOffset.x...)

    const bbox = geom.geometry.boundingBox ?? new THREE.Box3().setFromBufferAttribute(
      geom.geometry.getAttribute('position') as THREE.BufferAttribute
    );
    const centerOffset = bbox.getCenter(new THREE.Vector3());

    transformedGeom.translate(-centerOffset.x, -centerOffset.y, -centerOffset.z);

    const quaternion = quaternionFromGlobalEuler(transform.rotation);
    const matrix = new THREE.Matrix4().compose(
      new THREE.Vector3(transform.position.x, transform.position.y, transform.position.z),
      quaternion,
      new THREE.Vector3(transform.scale.x, transform.scale.y, transform.scale.z)
    );
    transformedGeom.applyMatrix4(matrix);
    transformedGeom.computeBoundingBox();

    return transformedGeom;
  }, [geom, transform]);

  const onRunIslandScan = useCallback(async () => {
    if (!geom) return;
    const transformedGeom = prepareTransformedGeom();
    if (!transformedGeom) return;

    setScanning(true);
    const transformedBBox = transformedGeom.boundingBox!;
    setScanBBox(transformedBBox);

    setScanProgress({ done: 0, total: Math.max(0, Math.ceil((transformedBBox.max.z - transformedBBox.min.z) / layerHeightMm)) });

    try {
      const res = await runIslandScan(
        { geometry: transformedGeom, bbox: transformedBBox },
        layerHeightMm,
        {
          px_mm: pxMm,
          support_buffer_mm: supportBufMm,
          connectivity,
          min_island_area_mm2: minIslandAreaMm2,
          min_overlap_px: minOverlapPx,
          overlap_neighborhood_px: overlapNeighborhoodPx,
          useSurfaceContiguity,
        },
        (done, total) => setScanProgress({ done, total })
      );
      setScanData(res);
    } finally {
      setScanning(false);
    }
  }, [geom, prepareTransformedGeom, layerHeightMm, pxMm, supportBufMm, connectivity, minIslandAreaMm2, minOverlapPx, overlapNeighborhoodPx, useSurfaceContiguity]);

  const onRunScanlineScan = useCallback(async () => {
    if (!geom) return;
    const transformedGeom = prepareTransformedGeom();
    if (!transformedGeom) return;

    setScanning(true);
    const transformedBBox = transformedGeom.boundingBox!;
    setScanBBox(transformedBBox);

    setScanProgress({ done: 0, total: Math.max(0, Math.ceil((transformedBBox.max.z - transformedBBox.min.z) / layerHeightMm)) });

    const startTime = performance.now();
    try {
      const res = await runScanlineScan(
        { geometry: transformedGeom, bbox: transformedBBox },
        layerHeightMm,
        {
          px_mm: pxMm,
          support_buffer_mm: supportBufMm,
          connectivity,
          min_island_area_mm2: minIslandAreaMm2,
          min_overlap_px: minOverlapPx,
          overlap_neighborhood_px: overlapNeighborhoodPx,
          useSurfaceContiguity,
        },
        (done, total) => setScanProgress({ done, total })
      );
      const endTime = performance.now();
      console.log(`Scanline Scan took ${(endTime - startTime).toFixed(2)}ms`);
      setScanData(res);
    } finally {
      setScanning(false);
    }
  }, [geom, prepareTransformedGeom, layerHeightMm, pxMm, supportBufMm, connectivity, minIslandAreaMm2, minOverlapPx, overlapNeighborhoodPx, useSurfaceContiguity]);

  const onRunNativeIslandScan = useCallback(async () => {
    if (!geom) return;
    const transformedGeom = prepareTransformedGeom();
    if (!transformedGeom) return;

    setScanning(true);
    const transformedBBox = transformedGeom.boundingBox!;
    setScanBBox(transformedBBox);

    const numLayers = Math.max(0, Math.ceil((transformedBBox.max.z - transformedBBox.min.z) / layerHeightMm));
    setScanProgress({ done: 0, total: numLayers });

    const startTime = performance.now();
    try {
      const res = await runIslandScanNative(
        { geometry: transformedGeom, bbox: transformedBBox },
        layerHeightMm,
        {
          px_mm: pxMm,
          support_buffer_mm: supportBufMm,
          connectivity,
          min_island_area_mm2: minIslandAreaMm2,
          min_overlap_px: minOverlapPx,
          overlap_neighborhood_px: overlapNeighborhoodPx,
        },
        (done, total) => setScanProgress({ done, total })
      );
      const endTime = performance.now();
      console.log(`Native Island Scan took ${(endTime - startTime).toFixed(2)}ms`);
      setScanData(res);
    } finally {
      setScanning(false);
    }
  }, [geom, prepareTransformedGeom, layerHeightMm, pxMm, supportBufMm, connectivity, minIslandAreaMm2, minOverlapPx, overlapNeighborhoodPx]);

  // Compute markers
  const islandMarkers = useMemo<IslandMarker[]>(() => {
    if (!scanData || !scanBBox) return [];
    return computeIslandMarkers(scanData, scanBBox, layerHeightMm, overlayTaper);
  }, [scanData, scanBBox, layerHeightMm, overlayTaper]);

  // Clear scan data (e.g. on rotation)
  const clearScanData = useCallback(() => {
    setScanData(null);
    setOverlayEnabled(false);
    setVoxelEnabled(false);
    setSelectedIslandId(null);
  }, []);

  return {
    scanning,
    scanProgress,
    scanData,
    setScanData,
    scanBBox,
    pxMm, setPxMm,
    supportBufMm, setSupportBufMm,
    connectivity, setConnectivity,
    minIslandAreaMm2, setMinIslandAreaMm2,
    minOverlapPx, setMinOverlapPx,
    overlapNeighborhoodPx, setOverlapNeighborhoodPx,
    useSurfaceContiguity, setUseSurfaceContiguity,
    scanCardExpanded, setScanCardExpanded,
    overlayEnabled, setOverlayEnabled,
    overlayColor, setOverlayColor,
    overlayOpacity, setOverlayOpacity,
    showIslandIdLabels, setShowIslandIdLabels,
    overlayHaloIntensity, setOverlayHaloIntensity,
    overlayHaloPulseEnabled, setOverlayHaloPulseEnabled,
    showSupportVolumeHalo, setShowSupportVolumeHalo,
    supportVolumeHaloIntensity, setSupportVolumeHaloIntensity,
    showIslands, setShowIslands,
    islandColor, setIslandColor,
    islandIntensity, setIslandIntensity,
    islandRadiusFactor, setIslandRadiusFactor,
    islandColumnHeight, setIslandColumnHeight,
    showOverhang, setShowOverhang,
    overhangColor, setOverhangColor,
    overhangAngleDeg, setOverhangAngleDeg,
    overhangIntensity, setOverhangIntensity,
    overhangProximityMm, setOverhangProximityMm,
    selectedIslandId, setSelectedIslandId,
    showMerged, setShowMerged,
    voxelEnabled, setVoxelEnabled,
    voxelColorScheme, setVoxelColorScheme,
    voxelOpacity, setVoxelOpacity,
    voxelShowMerged, setVoxelShowMerged,
    voxelShowTerritory, setVoxelShowTerritory,
    islandMarkers,
    onRunIslandScan,
    onRunScanlineScan,
    onRunNativeIslandScan,
    useNativeScan, setUseNativeScan,
    clearScanData
  };
}
