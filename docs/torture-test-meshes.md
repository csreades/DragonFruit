# Torture-test meshes for the slicer

DragonFruit ships **no** sample meshes (`public/mesh-preview-models/models.json`
is `{"models": []}`) — it's bring-your-own-STL. This is a curated set of
downloadable meshes for **stressing the slicer** (not the printer).

## What actually stresses a slicer

Measured on this repo at 16K/25µm (Saturn 4 Ultra 16K, `.goo`), slice cost is
driven by three things — strut/lattice/fractal meshes maximize all three:

1. **Triangle count** → rasterization cost (lattices are 10⁵–10⁷ triangles).
2. **Disjoint regions per layer** → RLE run count → encode time + peak RAM
   (a toy sphere-lattice here hit ~725 MB RAM / ~328 MB `.goo`).
3. **Sub-pixel-thin struts** → the 16K pixel is 14 µm, so struts under ~100 µm
   explode per-row run counts and AA edge bands.

So the ideal stressors are gyroid/TPMS lattices, Voronoi shells, and Menger
fractals — ranked below.

## Recommended downloads

### Best — a graded set (porosity = strut-thickness sweep)
- **Photopolymer gyroid lattice dataset** — 389 fine-resolution STLs (>70 MB
  each), 7 porosities (55–85%), 6×6×6 mm unit cells. A clean stress axis.
  - Data in Brief: https://www.sciencedirect.com/science/article/pii/S2352340923005061
  - PMC mirror: https://pmc.ncbi.nlm.nih.gov/articles/PMC10439287/
  - (see the "Lattice Mesh Files" folder in its linked data repository)

### Fractals — max per-layer holes
- **Menger Sponges** (Printables, free): https://www.printables.com/model/648813-menger-sponges
- Menger Sponge (Cults): https://cults3d.com/en/3d-model/art/menger-sponge

### Voronoi — high boundary/run complexity
- **Voronoi "Broken Benchy"** (MyMiniFactory): https://www.myminifactory.com/object/3d-print-voronoi-broken-benchy-the-true-and-not-so-jolly-torture-test-38864
- Voronoi 3DBenchy: https://www.3dbenchy.com/voronoi-patterned-3dbenchy/

### Strut / lattice torture models
- Lattice Torture Test (Printables): https://www.printables.com/model/473814-lattice-torture-test
- Gyroid tag (Cults3D): https://cults3d.com/en/tags/gyroid
- Gyroid Lattice (GrabCAD): https://grabcad.com/library/gyroid-lattice-2
- Gyroid models (CGTrader): https://www.cgtrader.com/3d-print-models/gyroid

### Parametric (dial strut fineness to sub-pixel = ultimate stress)
- **MSLattice** or **nTop** (free tier) — generate TPMS/gyroid and thin the
  struts until the slicer chokes; push strut width below the 14 µm pixel to
  stress AA + run count.

## Running them through the slicer

```sh
# CPU, full 16K / 25µm / 4× AA (memory + encode stressor)
dragonfruit-cli slice run lattice.stl -o out.goo \
  --layer-height 0.025 --build-width-mm 211.68 --build-depth-mm 118.37 \
  --source-width-px 15120 --source-height-px 6230 --mirror-x \
  --anti-aliasing 4x --json

# GPU backend (feature = "gpu"); reduce res if VRAM-limited (see src/gpu/README.md)
dragonfruit-cli slice run lattice.stl -o out.goo ... --backend gpu --json
```

Watch `perf.render_wall_ns` (raster), the `.goo` size (run count), and peak RSS.
A porosity sweep over the gyroid set gives a clean "coverage/run-count vs slice
cost" curve.
