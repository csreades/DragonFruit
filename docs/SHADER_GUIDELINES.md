# Shader Guidelines

How DragonFruit writes and integrates custom GLSL on top of Three.js
materials. The patterns below are not theoretical — they're how
`OverhangHeatmapMaterial` and `VolumetricHaloMaterial` are built today.
Follow them so new shader work composes with existing pipeline behaviour
(tonemapping, lighting, clipping, instancing).

## Pattern: extend `meshStandardMaterial` via `onBeforeCompile`

Don't reach for a fresh `ShaderMaterial` when you only want to add or
replace a fragment-shader concept. Patching the standard material's
compiled chunks keeps:

- the lighting / shadow contribution Three's pipeline computes for free
- tonemapping and color-space output (sRGB encoding)
- clipping plane support
- standard envMap / roughness / metalness interactions when you still
  want them

The recipe lives in `src/features/shaders/mesh/overhangHeatmap.tsx`
(applied to the model mesh) and `src/features/shaders/mesh/volumetricHalo.tsx`
(applied to overlay halo meshes):

```tsx
// 1. Hold the uniform OBJECTS (not just values) in a useRef so React
//    re-renders don't break the reference Three has bound into the
//    compiled program.
const uniformsRef = React.useRef({
  uFoo: { value: 1.0 },
  uBar: { value: new THREE.Color('#ff8800') },
  // ...
});

// 2. Sync .value on prop change. Mutate, don't replace.
React.useEffect(() => { uniformsRef.current.uFoo.value = foo; }, [foo]);

return (
  <meshStandardMaterial
    transparent
    depthWrite={false}
    toneMapped={false /* if you produce sRGB-final colors yourself */}
    onBeforeCompile={(shader) => {
      shader.uniforms.uFoo = uniformsRef.current.uFoo;
      shader.uniforms.uBar = uniformsRef.current.uBar;
      shader.vertexShader = `varying ...` + shader.vertexShader;
      shader.vertexShader = shader.vertexShader.replace(
        '#include <begin_vertex>',
        `#include <begin_vertex>\n  /* your additions */`,
      );
      shader.fragmentShader = `uniform float uFoo; ...` + shader.fragmentShader;
      // Three r152+ renamed `<output_fragment>` → `<opaque_fragment>`.
      // The legacy name silently no-ops on r152+ — your patch is dropped
      // with no compile error. Always anchor against `<opaque_fragment>`.
      shader.fragmentShader = shader.fragmentShader.replace(
        '#include <opaque_fragment>',
        `#include <opaque_fragment>\n  /* your output */`,
      );
    }}
  />
);
```

## Uniforms hygiene

- `useRef` once, mutate `.value` on prop change. Building a new uniforms
  object each render breaks the binding Three made at compile time.
- If you need per-instance values with `InstancedMesh`, use
  `InstancedBufferAttribute` and a `attribute` declaration in the vertex
  shader. Uniforms are per-mesh, not per-instance — this is a common
  trap with instanced shaders.
- Time-driven values (`uTime`) belong in `useFrame((s) => ...)`, not in
  a useEffect tied to clock state.

## Color stops via OKLCH

When a shader paints a gradient or thresholded scheme, pick stops in
OKLCH on the CPU and pass three pre-baked `THREE.Color` uniforms. Lerp
on the GPU in sRGB space — it's wrong in theory and right in practice
(perceptual error across a short three-stop ramp is below the visual
threshold; OKLCH on the GPU is expensive without TSL).

`src/features/shaders/mesh/haloColorRamp.ts` exposes the helper. Reuse
it; don't roll your own OKLCH math.

## One cognitive knob per perceptual axis

When a shader has many uniforms, the corresponding UI control surface
should NOT have one slider per uniform. Pick the one cognitive axis the
user cares about — "how loudly does this draw attention?" — and map a
single slider through. Hide finer-grained controls behind an "Advanced"
disclosure if power users need them.

The halo shader has ~14 uniforms; the user sees one slider and one
toggle. Pulse on/off respects `prefers-reduced-motion`.

## Render-order discipline for transparent overlays

Three.js's transparent sort is fragile when many transparent meshes
overlap. For overlay layers like halos:

- explicit `renderOrder` per layer (500 for ambient pass, 999 for
  occluded selection pass, 1000 for visible selection pass)
- `depthWrite={false}` so transparent meshes don't poison the depth
  buffer for each other
- cap per-fragment alpha via a `uMaxOpacity` uniform so a stack of
  overlapping halos can never fully obscure the model behind them

## See also

- `OverhangHeatmapMaterial` — the canonical onBeforeCompile precedent.
- `VolumetricHaloMaterial` — the most uniform-heavy patcher in the
  codebase; reference for selection-rim, prefers-reduced-motion, and
  instanced-friendly material instancing.
- `docs/SUPPORT_CODING_GUIDELINES.md` §10 — when introducing a new
  shader-driven visualisation on supports, decouple it from the
  batched / detailed renderer split by consuming `SupportState`
  directly. See `SupportVolumeHalo` for the pattern.
