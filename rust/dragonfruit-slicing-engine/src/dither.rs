//! Floyd-Steinberg energy-based dithering for low-bit-depth display systems.

use crate::rle::{RleRun, RleAccum, emit_row, emit_zero_rows};

/// Precomputed energy tables and target palette for Floyd-Steinberg dithering.
pub struct DitherPaletteV3 {
    /// Maps 8-bit input grayscale directly to physical energy based on the job LUT and gamma:
    /// E = (lut[V] / 255.0) ^ gamma
    pub source_energy: [f32; 256],
    
    /// Target PWM byte values for the low-bit-depth palette levels (e.g. 8 levels for 3-bit).
    pub target_bytes: Vec<u8>,
    
    /// Maps each target palette index to its corresponding physical energy level.
    pub target_energy: Vec<f32>,
}

impl DitherPaletteV3 {
    /// Create and precompute a new dithering palette.
    pub fn new(lut: &[u8; 256], gamma: f64, bit_depth: u32) -> Self {
        let bit_depth = bit_depth.clamp(2, 7);
        let mut source_energy = [0.0f32; 256];
        for i in 0..256 {
            source_energy[i] = ((lut[i] as f64) / 255.0).powf(gamma) as f32;
        }

        let levels = 1 << bit_depth;
        let mut target_bytes = Vec::with_capacity(levels);
        let mut target_energy = Vec::with_capacity(levels);
        for i in 0..levels {
            let val = i as f64 * (255.0 / (levels - 1) as f64);
            let pwm_byte = val.round().clamp(0.0, 255.0) as u8;
            target_bytes.push(pwm_byte);
            target_energy.push(((pwm_byte as f64) / 255.0).powf(gamma) as f32);
        }

        Self {
            source_energy,
            target_bytes,
            target_energy,
        }
    }
}

/// Dither an RLE layer row-by-row with O(width) auxiliary memory.
///
/// Implements sequential Floyd-Steinberg error diffusion in physical energy space,
/// re-encoding back to RLE runs row-by-row.
pub fn dither_rle_layer_with_lut_and_gamma(
    runs: &[RleRun],
    palette: &DitherPaletteV3,
    width: usize,
    height: usize,
) -> Vec<RleRun> {
    if runs.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }

    let mut out = RleAccum::new();
    
    // Auxiliary sliding row error buffers: only O(width) RAM overhead
    let mut err_curr = vec![0.0f32; width];
    let mut err_next = vec![0.0f32; width];

    let mut decoder = crate::engine::RleRowDecoder::new(runs);
    let mut row_pixels = vec![0u8; width];

    for y in 0..height {
        // Decode one row from RLE runs
        decoder.decode_next_row_span(width, 0, &mut row_pixels);

        let is_empty_row = row_pixels.iter().all(|&v| v == 0);
        let has_prior_errors = err_curr.iter().any(|&e| e != 0.0);

        if is_empty_row && !has_prior_errors {
            // Fast path: emit empty row directly
            emit_zero_rows(&mut out, 1, width);
            err_curr.fill(0.0);
            err_next.fill(0.0);
            continue;
        }

        // Apply Floyd-Steinberg error diffusion on the row
        for x in 0..width {
            let val = row_pixels[x];
            let src_energy = palette.source_energy[val as usize];
            let desired_energy = src_energy + err_curr[x];

            // Find closest target palette index
            let mut best_idx = 0;
            let mut min_diff = f32::MAX;
            for (k, &tgt_energy) in palette.target_energy.iter().enumerate() {
                let diff = (desired_energy - tgt_energy).abs();
                if diff < min_diff {
                    min_diff = diff;
                    best_idx = k;
                }
            }

            let dithered_val = palette.target_bytes[best_idx];
            row_pixels[x] = dithered_val;

            // Quantization error in physical energy space
            let quant_error = desired_energy - palette.target_energy[best_idx];

            // Diffuse error to neighbors
            if x + 1 < width {
                err_curr[x + 1] += quant_error * 0.4375; // Right (7/16)
            }
            if y + 1 < height {
                if x > 0 {
                    err_next[x - 1] += quant_error * 0.1875; // Bottom-Left (3/16)
                }
                err_next[x] += quant_error * 0.3125; // Bottom (5/16)
                if x + 1 < width {
                    err_next[x + 1] += quant_error * 0.0625; // Bottom-Right (1/16)
                }
            }
        }

        // Re-encode dithered row to RLE runs
        emit_row(&mut out, &row_pixels);

        // Swap error rows & clear err_next for next loop
        std::mem::swap(&mut err_curr, &mut err_next);
        err_next.fill(0.0);
    }

    out.finish()
}

/// Dither a flat mask sub-image row-by-row with O(width) auxiliary memory.
///
/// Implements sequential Floyd-Steinberg error diffusion in physical energy space
/// directly in the provided flat slice bounds.
pub fn dither_mask_in_bounds(
    mask: &mut [u8],
    row_width: usize,
    row_height: usize,
    palette: &DitherPaletteV3,
) {
    if mask.is_empty() || row_width == 0 || row_height == 0 {
        return;
    }

    // Auxiliary sliding row error buffers: only O(row_width) RAM overhead
    let mut err_curr = vec![0.0f32; row_width];
    let mut err_next = vec![0.0f32; row_width];

    for y in 0..row_height {
        let row_offset = y * row_width;
        
        for x in 0..row_width {
            let idx = row_offset + x;
            let val = mask[idx];
            let src_energy = palette.source_energy[val as usize];
            let desired_energy = src_energy + err_curr[x];

            // Find closest target palette index
            let mut best_idx = 0;
            let mut min_diff = f32::MAX;
            for (k, &tgt_energy) in palette.target_energy.iter().enumerate() {
                let diff = (desired_energy - tgt_energy).abs();
                if diff < min_diff {
                    min_diff = diff;
                    best_idx = k;
                }
            }

            let dithered_val = palette.target_bytes[best_idx];
            mask[idx] = dithered_val;

            // Quantization error in physical energy space
            let quant_error = desired_energy - palette.target_energy[best_idx];

            // Diffuse error to neighbors
            if x + 1 < row_width {
                err_curr[x + 1] += quant_error * 0.4375; // Right (7/16)
            }
            if y + 1 < row_height {
                if x > 0 {
                    err_next[x - 1] += quant_error * 0.1875; // Bottom-Left (3/16)
                }
                err_next[x] += quant_error * 0.3125; // Bottom (5/16)
                if x + 1 < row_width {
                    err_next[x + 1] += quant_error * 0.0625; // Bottom-Right (1/16)
                }
            }
        }

        // Swap error rows & clear err_next for next loop
        std::mem::swap(&mut err_curr, &mut err_next);
        err_next.fill(0.0);
    }
}
