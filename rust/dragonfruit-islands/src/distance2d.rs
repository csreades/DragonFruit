//! Anisotropic 2-D Euclidean distance transform (Check 1 — resin escape).
//!
//! For a per-layer solid mask, computes for every solid pixel the straight-line
//! distance to the nearest *non-solid* (empty) pixel — the resin's lateral
//! escape-path length. The MAX over the layer is the inscribed-circle radius of
//! the most landlocked region: the Check-1 risk scalar.
//!
//! Exact separable Felzenszwalb–Huttenlocher transform (two 1-D passes:
//! columns then rows), with **per-axis physical weights** `(dx_um, dy_um)` so a
//! 14 µm × 19 µm pixel grid yields true microns, not isotropic pixel counts.
//!
//! Scope (see docs/design/preflight/check-1-resin-escape.md): this is the
//! *core numeric*. It measures worst **in-plane lateral** escape only — it is
//! blind to sealed 3-D cavities. The caller is responsible for cropping to the
//! solid bounding box (this transform is O(w·h) in the region it is given).

/// Result of a layer distance transform.
#[derive(Debug, Clone)]
pub struct EscapeField {
    pub width: usize,
    pub height: usize,
    /// Per-pixel escape distance in microns (0 for non-solid pixels).
    pub dist_um: Vec<f32>,
    /// Worst escape distance over the layer (microns).
    pub max_um: f32,
    /// Location of that maximum (x, y) in pixel coords.
    pub argmax: (usize, usize),
}

// A finite sentinel that is larger than any achievable squared distance in a
// real layer, so the transform stays in clean finite f64 arithmetic (no
// INF/NaN edge cases in the parabola intersections). Real squared distances are
// at most (dx·w)² + (dy·h)²; for a full 16K plate that is ~5e10 µm², so 1e18 is
// unreachable yet never overflows f64.
const SEED_MISS: f64 = 1.0e18;

/// 1-D exact squared distance transform along one axis with sample spacing `s`
/// (microns per pixel): `out[q] = min_p ( (s·(q−p))² + f[p] )`.
///
/// `f` holds 0 at seed (empty) samples and a large value at non-seed (solid)
/// samples on the first pass; on the second pass it holds the first pass's
/// squared distances. All values are finite.
fn dt_1d(f: &[f64], s: f64, out: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let s2 = s * s;
    // v: parabola centres of the lower envelope; z: their boundaries.
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;

    // Intersection abscissa of the parabolas centred at p and q (p < q).
    let intersect = |q: usize, p: usize| -> f64 {
        let qf = q as f64;
        let pf = p as f64;
        ((qf * qf - pf * pf) + (f[q] - f[p]) / s2) / (2.0 * (qf - pf))
    };

    for q in 1..n {
        let mut s_int = intersect(q, v[k]);
        // z[0] = −∞ guards k from underflowing (finite s_int is never ≤ −∞).
        while s_int <= z[k] {
            k -= 1;
            s_int = intersect(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s_int;
        z[k + 1] = f64::INFINITY;
    }

    let mut k2 = 0usize;
    for q in 0..n {
        while z[k2 + 1] < q as f64 {
            k2 += 1;
        }
        let dq = q as f64 - v[k2] as f64;
        out[q] = s2 * dq * dq + f[v[k2]];
    }
}

/// Exact anisotropic 2-D EDT of `solid` (row-major, `true` = solid).
///
/// `dx_um` / `dy_um` are the physical pixel pitches. Returns escape distance in
/// microns per pixel plus the layer MAX and its location. The region beyond the
/// mask is treated as empty (a solid pixel at the border escapes immediately),
/// so callers pad the solid bbox by ≥1 empty pixel before calling.
pub fn distance_transform(
    solid: &[bool],
    width: usize,
    height: usize,
    dx_um: f64,
    dy_um: f64,
) -> EscapeField {
    assert_eq!(solid.len(), width * height, "mask len must be width*height");
    if width == 0 || height == 0 {
        return EscapeField { width, height, dist_um: Vec::new(), max_um: 0.0, argmax: (0, 0) };
    }

    // f: 0 at empty (seed), SEED_MISS at solid.
    let mut f: Vec<f64> = solid.iter().map(|&s| if s { SEED_MISS } else { 0.0 }).collect();

    // Pass 1 — columns (y axis, spacing dy).
    let mut col_in = vec![0.0f64; height];
    let mut col_out = vec![0.0f64; height];
    let mut g = vec![0.0f64; width * height];
    for x in 0..width {
        for y in 0..height {
            col_in[y] = f[y * width + x];
        }
        dt_1d(&col_in, dy_um, &mut col_out);
        for y in 0..height {
            g[y * width + x] = col_out[y];
        }
    }

    // Pass 2 — rows (x axis, spacing dx). g becomes squared µm distance.
    let mut row_out = vec![0.0f64; width];
    let mut max_um = 0.0f32;
    let mut argmax = (0usize, 0usize);
    let mut dist_um = vec![0.0f32; width * height];
    for y in 0..height {
        let row = &g[y * width..(y + 1) * width];
        dt_1d(row, dx_um, &mut row_out);
        for x in 0..width {
            let d = row_out[x].max(0.0).sqrt() as f32;
            dist_um[y * width + x] = d;
            if d > max_um {
                max_um = d;
                argmax = (x, y);
            }
        }
    }

    // reuse f's allocation intent; nothing else references it now
    f.clear();

    EscapeField { width, height, dist_um, max_um, argmax }
}

/// Local maxima of the distance field (candidate drain-hole locations): pixels
/// strictly greater than their 8 neighbours and ≥ `frac` × the layer max.
/// Returned as `(x, y, dist_um)`, strongest first.
pub fn local_maxima(field: &EscapeField, frac: f32) -> Vec<(usize, usize, f32)> {
    let (w, h) = (field.width, field.height);
    let thresh = field.max_um * frac;
    let mut peaks = Vec::new();
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let c = field.dist_um[y * w + x];
            if c < thresh || c <= 0.0 {
                continue;
            }
            let mut is_peak = true;
            'nb: for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = (x as i32 + dx) as usize;
                    let ny = (y as i32 + dy) as usize;
                    if field.dist_um[ny * w + nx] > c {
                        is_peak = false;
                        break 'nb;
                    }
                }
            }
            if is_peak {
                peaks.push((x, y, c));
            }
        }
    }
    peaks.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O(solid·empty) reference — the ground truth the fast transform must match.
    fn brute_force(solid: &[bool], w: usize, h: usize, dx: f64, dy: f64) -> Vec<f32> {
        let empties: Vec<(i64, i64)> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| !solid[y * w + x])
            .map(|(x, y)| (x as i64, y as i64))
            .collect();
        let mut out = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                if !solid[y * w + x] {
                    continue;
                }
                let mut best = f64::INFINITY;
                for &(ex, ey) in &empties {
                    let ddx = dx * (x as i64 - ex) as f64;
                    let ddy = dy * (y as i64 - ey) as f64;
                    best = best.min(ddx * ddx + ddy * ddy);
                }
                out[y * w + x] = best.sqrt() as f32;
            }
        }
        out
    }

    fn assert_matches_brute(solid: &[bool], w: usize, h: usize, dx: f64, dy: f64) {
        let field = distance_transform(solid, w, h, dx, dy);
        let bf = brute_force(solid, w, h, dx, dy);
        for i in 0..w * h {
            let diff = (field.dist_um[i] - bf[i]).abs();
            assert!(
                diff < 1e-3,
                "pixel {i} ({},{}): edt={} brute={} (dx={dx} dy={dy})",
                i % w,
                i / w,
                field.dist_um[i],
                bf[i]
            );
        }
    }

    // Build a solid disk of radius r (pixels) centred in a (2r+3)² grid.
    fn disk(r: i32) -> (Vec<bool>, usize) {
        let w = (2 * r + 3) as usize;
        let c = (w / 2) as i32;
        let mut m = vec![false; w * w];
        for y in 0..w as i32 {
            for x in 0..w as i32 {
                let dx = x - c;
                let dy = y - c;
                if dx * dx + dy * dy <= r * r {
                    m[(y as usize) * w + x as usize] = true;
                }
            }
        }
        (m, w)
    }

    #[test]
    fn thin_rect_escapes_easily() {
        // 2-wide × 20-tall solid strip in a padded grid: every interior pixel is
        // ~1 px from an edge, so max escape ≈ half a pixel pitch — LOW risk.
        let (w, h) = (6usize, 24usize);
        let mut m = vec![false; w * h];
        for y in 2..22 {
            for x in 2..4 {
                m[y * w + x] = true;
            }
        }
        let field = distance_transform(&m, w, h, 14.0, 19.0);
        assert!(field.max_um <= 14.0 + 1e-3, "thin strip max_um={} should be ~1px", field.max_um);
        assert_matches_brute(&m, w, h, 14.0, 19.0);
    }

    #[test]
    fn disk_max_is_radius_isotropic() {
        // Isotropic 10 µm pixels; a disk radius R traps resin at its centre at
        // distance ≈ R·pitch. Nearest empty is one ring outside the last solid
        // ring, so expect ~ (r+1)·pitch, matched exactly by brute force.
        let r = 8;
        let (m, w) = disk(r);
        let field = distance_transform(&m, w, w, 10.0, 10.0);
        let bf = brute_force(&m, w, w, 10.0, 10.0);
        let bf_max = bf.iter().cloned().fold(0.0f32, f32::max);
        assert!((field.max_um - bf_max).abs() < 1e-3);
        // Centre is the most landlocked point → argmax at the disk centre.
        assert_eq!(field.argmax, (w / 2, w / 2));
        assert!(field.max_um >= (r as f32) * 10.0, "disk max {} < radius", field.max_um);
    }

    #[test]
    fn annulus_max_is_wall_half_thickness() {
        // A ring: resin escapes to BOTH the inner hole and the outer edge, so
        // the worst point sits mid-wall at ≈ half the wall thickness — far less
        // than the outer radius. This is the "add a drain hole" win made numeric.
        let (ro, ri) = (12i32, 7i32);
        let w = (2 * ro + 3) as usize;
        let c = (w / 2) as i32;
        let mut m = vec![false; w * w];
        for y in 0..w as i32 {
            for x in 0..w as i32 {
                let d2 = (x - c) * (x - c) + (y - c) * (y - c);
                if d2 <= ro * ro && d2 >= ri * ri {
                    m[(y as usize) * w + x as usize] = true;
                }
            }
        }
        let field = distance_transform(&m, w, w, 10.0, 10.0);
        let wall = (ro - ri) as f32 * 10.0;
        // Mid-wall distance ≈ wall/2; must be well under the solid-disk figure (ro).
        assert!(
            field.max_um <= wall * 0.6 + 1e-3,
            "annulus max {} should be ~half-wall ({}), not the radius",
            field.max_um,
            wall / 2.0
        );
        assert_matches_brute(&m, w, w, 10.0, 10.0);
    }

    #[test]
    fn anisotropic_weights_are_per_axis() {
        // A single empty pixel gap: distance across it must use the correct axis
        // pitch. Solid everywhere except one empty column vs one empty row.
        let (w, h) = (9usize, 9usize);
        let mut m = vec![true; w * h];
        m[4 * w + 4] = false; // one empty pixel at centre
        let field = distance_transform(&m, w, h, 14.0, 19.0);
        // The pixel directly left of the hole is 1 x-step away → 14 µm; the pixel
        // directly above is 1 y-step away → 19 µm. Verify the axes are distinct.
        assert!((field.dist_um[4 * w + 3] - 14.0).abs() < 1e-3, "x-neighbour should be dx");
        assert!((field.dist_um[3 * w + 4] - 19.0).abs() < 1e-3, "y-neighbour should be dy");
        assert_matches_brute(&m, w, h, 14.0, 19.0);
    }

    #[test]
    fn matches_brute_force_on_random_masks() {
        // Deterministic pseudo-random masks — exhaustive per-pixel equality.
        let (w, h) = (17usize, 13usize);
        let mut state: u64 = 0x9e3779b97f4a7c15;
        for _ in 0..8 {
            let mut m = vec![false; w * h];
            let mut any_empty = false;
            for c in m.iter_mut() {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *c = (state & 3) != 0; // ~75% solid
                if !*c {
                    any_empty = true;
                }
            }
            if !any_empty {
                m[0] = false;
            }
            assert_matches_brute(&m, w, h, 14.0, 19.0);
        }
    }
}
