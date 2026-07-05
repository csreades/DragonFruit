//! Check 2 (geometry mode) — cross-section peel analysis over the full solid.
//!
//! Peel failure is a geometry phenomenon: any thin cross-section carrying a
//! large peeling mass above it can snap — a baked-in strut, a thin wall, or a
//! fragile neck on the part itself. This analyses the full sliced solid (not
//! overhang-only regions), so it works uniformly for parts, imported
//! pre-supported meshes, and native supports.
//!
//! For each per-layer connected region we compute the max cross-section of the
//! mass ABOVE it that routes through it (a top-down overlap DP), then
//!
//!   SF(region) = green * A / (peel * max(A, maxSectionAbove))
//!
//! Regions are unioned across layers into components; the worst (min-SF) region
//! per component is a "neck" and localises to (layer, XY). Streaming: only the
//! previous layer's label grid is held, so memory is O(one layer + graph).

use std::collections::HashSet;

use crate::model::{Connectivity, RleMask};
use crate::rle::{rle_decode_labels, rle_label_components};

#[derive(Clone, Copy)]
pub struct SectionMaterial {
    /// Green strength of the cured resin, MPa (= N/mm²). Conservative (low).
    pub green_mpa: f64,
    /// Effective peel/separation stress, MPa. Conservative (high).
    pub peel_mpa: f64,
}

impl Default for SectionMaterial {
    fn default() -> Self {
        // Shares Check 2's calibrated defaults.
        SectionMaterial { green_mpa: 18.0, peel_mpa: 0.012 }
    }
}

#[derive(Clone)]
pub struct Neck {
    pub layer: u32,
    /// Centroid in grid-relative mm (caller adds the world origin).
    pub cx_mm: f64,
    pub cy_mm: f64,
    pub sf: f64,
    pub area_mm2: f64,
    pub peel_above_mm2: f64,
}

pub struct SectionReport {
    pub component_count: usize,
    pub region_count: usize,
    pub worst_sf: f64,
    pub fail_count: usize,
    pub marginal_count: usize,
    /// One neck per component (the weakest region), sorted worst-first.
    pub necks: Vec<Neck>,
}

fn uf_find(uf: &mut [usize], x: usize) -> usize {
    let mut r = x;
    while uf[r] != r {
        r = uf[r];
    }
    // path compression
    let mut c = x;
    while uf[c] != c {
        let nx = uf[c];
        uf[c] = r;
        c = nx;
    }
    r
}

fn uf_union(uf: &mut [usize], a: usize, b: usize) {
    let ra = uf_find(uf, a);
    let rb = uf_find(uf, b);
    if ra != rb {
        uf[ra] = rb;
    }
}

pub fn analyze_sections(masks: &[RleMask], px_mm: f64, mat: &SectionMaterial) -> SectionReport {
    let px_area = px_mm * px_mm;
    let n = masks.len();

    let mut node_area: Vec<f64> = Vec::new();
    let mut node_cx: Vec<f64> = Vec::new();
    let mut node_cy: Vec<f64> = Vec::new();
    let mut node_layer: Vec<u32> = Vec::new();
    let mut above: Vec<Vec<usize>> = Vec::new(); // node -> nodes directly above it
    let mut uf: Vec<usize> = Vec::new();

    let mut layer_first: Vec<usize> = Vec::with_capacity(n);
    let mut layer_nlabels: Vec<usize> = Vec::with_capacity(n);

    let mut prev_grid: Option<Vec<i32>> = None;
    let mut prev_first: usize = 0;

    for (l, mask) in masks.iter().enumerate() {
        let (labels, comps) = rle_label_components(mask, Connectivity::Four);
        let grid = rle_decode_labels(&labels);
        let maxlabel = comps.iter().map(|c| c.label).max().unwrap_or(0).max(0) as usize;

        let first = node_area.len();
        layer_first.push(first);
        layer_nlabels.push(maxlabel);
        for _ in 0..maxlabel {
            let idx = node_area.len();
            node_area.push(0.0);
            node_cx.push(0.0);
            node_cy.push(0.0);
            node_layer.push(l as u32);
            above.push(Vec::new());
            uf.push(idx);
        }
        for c in &comps {
            if c.label < 1 || c.label as usize > maxlabel {
                continue;
            }
            let idx = first + (c.label as usize - 1);
            let ap = c.area_px.max(0) as f64;
            node_area[idx] = ap * px_area;
            let cnt = c.area_px.max(1) as f64;
            node_cx[idx] = (c.centroid_sum_x / cnt) * px_mm;
            node_cy[idx] = (c.centroid_sum_y / cnt) * px_mm;
        }

        if let Some(pg) = &prev_grid {
            let len = pg.len().min(grid.len());
            let mut seen: HashSet<(i32, i32)> = HashSet::new();
            for i in 0..len {
                let a = pg[i];
                let b = grid[i];
                if a > 0 && b > 0 && seen.insert((a, b)) {
                    let na = prev_first + (a as usize - 1);
                    let nb = first + (b as usize - 1);
                    if na < above.len() && nb < node_area.len() {
                        above[na].push(nb);
                        uf_union(&mut uf, na, nb);
                    }
                }
            }
        }

        prev_grid = Some(grid);
        prev_first = first;
    }

    let total = node_area.len();
    if total == 0 {
        return SectionReport {
            component_count: 0,
            region_count: 0,
            worst_sf: f64::INFINITY,
            fail_count: 0,
            marginal_count: 0,
            necks: Vec::new(),
        };
    }

    // Top-down DP: max_incl[node] = max(area[node], max over above of max_incl).
    let mut max_incl = vec![0.0f64; total];
    for l in (0..n).rev() {
        let start = layer_first[l];
        let end = start + layer_nlabels[l];
        for idx in start..end {
            let mut m = 0.0f64;
            for &nb in &above[idx] {
                if max_incl[nb] > m {
                    m = max_incl[nb];
                }
            }
            max_incl[idx] = node_area[idx].max(m);
        }
    }

    // Per-region SF and per-component worst region.
    let green = mat.green_mpa.max(0.0);
    let peel = mat.peel_mpa.max(1e-12);
    let mut roots = vec![0usize; total];
    for i in 0..total {
        roots[i] = uf_find(&mut uf, i);
    }

    let mut node_sf = vec![f64::INFINITY; total];
    for idx in 0..total {
        let a = node_area[idx];
        let demand = peel * max_incl[idx];
        node_sf[idx] = if a <= 0.0 {
            0.0
        } else if demand <= 0.0 {
            f64::INFINITY
        } else {
            green * a / demand
        };
    }

    // worst region index per component root.
    use std::collections::HashMap;
    let mut comp_worst: HashMap<usize, usize> = HashMap::new();
    for idx in 0..total {
        let root = roots[idx];
        comp_worst
            .entry(root)
            .and_modify(|w| {
                if node_sf[idx] < node_sf[*w] {
                    *w = idx;
                }
            })
            .or_insert(idx);
    }

    let mut necks: Vec<Neck> = comp_worst
        .values()
        .map(|&idx| Neck {
            layer: node_layer[idx],
            cx_mm: node_cx[idx],
            cy_mm: node_cy[idx],
            sf: node_sf[idx],
            area_mm2: node_area[idx],
            peel_above_mm2: max_incl[idx],
        })
        .collect();
    necks.sort_by(|a, b| a.sf.partial_cmp(&b.sf).unwrap_or(std::cmp::Ordering::Equal));

    let fail_count = necks.iter().filter(|nk| nk.sf < 1.0).count();
    let marginal_count = necks.iter().filter(|nk| nk.sf >= 1.0 && nk.sf < 2.0).count();
    let worst_sf = necks.first().map(|nk| nk.sf).unwrap_or(f64::INFINITY);

    SectionReport {
        component_count: comp_worst.len(),
        region_count: total,
        worst_sf,
        fail_count,
        marginal_count,
        necks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rle::rle_encode;

    // Build a mask: a `side`×`side` solid square centred in a `w`×`w` grid.
    fn square_mask(w: i32, side: i32) -> RleMask {
        let mut data = vec![0u8; (w * w) as usize];
        let lo = (w - side) / 2;
        let hi = lo + side;
        for y in lo..hi {
            for x in lo..hi {
                data[(y * w + x) as usize] = 1;
            }
        }
        rle_encode(&data, w, w)
    }

    const M: SectionMaterial = SectionMaterial { green_mpa: 18.0, peel_mpa: 0.012 };

    #[test]
    fn uniform_column_is_safe() {
        let masks: Vec<RleMask> = (0..5).map(|_| square_mask(40, 10)).collect();
        let r = analyze_sections(&masks, 0.1, &M);
        assert_eq!(r.fail_count, 0);
        // SF = green/peel = 1500 everywhere.
        assert!((r.worst_sf - 1500.0).abs() < 1.0, "worst_sf {}", r.worst_sf);
    }

    #[test]
    fn upright_pyramid_is_safe() {
        // side decreasing upward → each layer carries only smaller mass above.
        let sides = [30, 24, 18, 12, 6];
        let masks: Vec<RleMask> = sides.iter().map(|&s| square_mask(40, s)).collect();
        let r = analyze_sections(&masks, 0.1, &M);
        assert_eq!(r.fail_count, 0, "upright pyramid should not fail");
    }

    #[test]
    fn inverted_pyramid_fails_at_the_point() {
        // side increasing upward → tiny base bears the whole mass above.
        // Must be genuinely extreme: fail needs base/top < peel/green ≈ 7e-4.
        // 1px base (0.01mm²) under a ~139mm² top → base/top ≈ 7e-5 → SF ≈ 0.1.
        let sides = [1, 30, 60, 90, 118];
        let masks: Vec<RleMask> = sides.iter().map(|&s| square_mask(120, s)).collect();
        let r = analyze_sections(&masks, 0.1, &M);
        assert!(r.fail_count >= 1, "inverted pyramid must fail (worst_sf {})", r.worst_sf);
        // The worst neck is at the base (layer 0).
        assert_eq!(r.necks[0].layer, 0, "neck at the point (base)");
    }

    #[test]
    fn blob_on_thin_stalk_flags_the_stalk() {
        // thick, thin stalk, thick blob.
        let sides = [30, 30, 2, 30, 30];
        let masks: Vec<RleMask> = sides.iter().map(|&s| square_mask(40, s)).collect();
        let r = analyze_sections(&masks, 0.1, &M);
        assert!(r.necks[0].sf < 1500.0, "stalk neck should be well below a solid column");
        assert_eq!(r.necks[0].layer, 2, "neck localises to the stalk layer");
    }

    #[test]
    fn two_disconnected_columns_are_two_components() {
        // Two separate squares in one grid, all layers → two components.
        let w = 60;
        let make = || {
            let mut data = vec![0u8; (w * w) as usize];
            for y in 20..30 {
                for x in 5..15 {
                    data[(y * w + x) as usize] = 1;
                }
                for x in 45..55 {
                    data[(y * w + x) as usize] = 1;
                }
            }
            rle_encode(&data, w, w)
        };
        let masks: Vec<RleMask> = (0..3).map(|_| make()).collect();
        let r = analyze_sections(&masks, 0.1, &M);
        assert_eq!(r.component_count, 2, "two disconnected columns");
    }
}
