use crate::types::SliceJobV3;
use rayon::prelude::*;

pub fn find_active_bounds(mask: &[u8], width: usize, height: usize) -> Option<(usize, usize, usize, usize)> {
    if mask.is_empty() || width == 0 || height == 0 {
        return None;
    }

    // Find min_y
    let mut min_y = None;
    for y in 0..height {
        let row = &mask[y * width..(y + 1) * width];
        if row.iter().any(|&p| p > 0) {
            min_y = Some(y);
            break;
        }
    }

    let Some(start_y) = min_y else {
        return None;
    };

    // Find max_y
    let mut max_y = start_y;
    for y in (start_y + 1..height).rev() {
        let row = &mask[y * width..(y + 1) * width];
        if row.iter().any(|&p| p > 0) {
            max_y = y;
            break;
        }
    }

    // Find min_x and max_x within [start_y, max_y]
    let mut min_x = width;
    let mut max_x = 0;
    for y in start_y..=max_y {
        let row = &mask[y * width..(y + 1) * width];
        if let Some(first) = row.iter().position(|&p| p > 0) {
            if first < min_x {
                min_x = first;
            }
            if let Some(last) = row.iter().rposition(|&p| p > 0) {
                if last > max_x {
                    max_x = last;
                }
            }
        }
    }

    if min_x <= max_x {
        Some((min_x, max_x, start_y, max_y))
    } else {
        None
    }
}

pub fn make_gaussian_kernel(radius: usize, sigma: f32) -> Vec<f32> {
    if radius == 0 {
        return vec![1.0];
    }
    let size = 2 * radius + 1;
    let mut kernel = vec![0.0f32; size];
    let mut sum = 0.0f32;
    let sigma2 = 2.0 * sigma * sigma;
    for i in 0..size {
        let d = (i as isize - radius as isize) as f32;
        let w = (-d * d / sigma2).exp();
        kernel[i] = w;
        sum += w;
    }
    if sum > 0.0 {
        for w in &mut kernel {
            *w /= sum;
        }
    }
    kernel
}

pub fn apply_xy_blur(
    mask: &mut [u8],
    width: usize,
    height: usize,
    mode: &str,
    radius: u32,
    sigma_x: f32,
    sigma_y: f32,
    roi: (usize, usize, usize, usize),
) {
    if mode == "None" || radius == 0 || width == 0 || height == 0 {
        return;
    }
    let radius = radius.clamp(1, 6) as usize;

    let kernel_x = if mode == "Gaussian" {
        make_gaussian_kernel(radius, sigma_x)
    } else if mode == "Linear" || mode == "Tent" {
        let size = 2 * radius + 1;
        let mut k = vec![0.0f32; size];
        let denom = ((radius + 1) * (radius + 1)) as f32;
        for i in 0..size {
            let dist = (i as isize - radius as isize).abs() as usize;
            k[i] = ((radius + 1 - dist) as f32) / denom;
        }
        k
    } else {
        vec![1.0 / (2 * radius + 1) as f32; 2 * radius + 1]
    };

    let kernel_y = if mode == "Gaussian" {
        make_gaussian_kernel(radius, sigma_y)
    } else if mode == "Linear" || mode == "Tent" {
        let size = 2 * radius + 1;
        let mut k = vec![0.0f32; size];
        let denom = ((radius + 1) * (radius + 1)) as f32;
        for i in 0..size {
            let dist = (i as isize - radius as isize).abs() as usize;
            k[i] = ((radius + 1 - dist) as f32) / denom;
        }
        k
    } else {
        vec![1.0 / (2 * radius + 1) as f32; 2 * radius + 1]
    };

    let (min_x, max_x, min_y, max_y) = roi;
    let h_roi = max_y - min_y + 1;

    let mut temp = vec![0.0f32; width * h_roi];

    // Horizontal pass: mask -> temp (inside ROI)
    for y in min_y..=max_y {
        let row_offset = y * width;
        let src_row = &mask[row_offset..row_offset + width];
        let dest_row = &mut temp[(y - min_y) * width..(y - min_y) * width + width];

        for x in min_x..=max_x {
            let mut val = 0.0f32;
            for (i, &w) in kernel_x.iter().enumerate() {
                let sample_x = (x as isize + i as isize - radius as isize).clamp(0, width as isize - 1) as usize;
                val += src_row[sample_x] as f32 * w;
            }
            dest_row[x] = val;
        }
    }

    // Vertical pass: temp -> mask (inside ROI)
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            let mut val = 0.0f32;
            for (i, &w) in kernel_y.iter().enumerate() {
                let sample_y = (y as isize + i as isize - radius as isize).clamp(min_y as isize, max_y as isize) as usize;
                val += temp[(sample_y - min_y) * width + x] * w;
            }
            mask[y * width + x] = val.round().clamp(0.0, 255.0) as u8;
        }
    }
}

pub fn apply_z_blur_subrange(
    masks: &mut [Vec<u8>],
    width: usize,
    height: usize,
    mode: &str,
    radius: u32,
    sigma_z: f32,
    roi: (usize, usize, usize, usize),
    start_main_idx: usize,
    end_main_idx: usize,
) {
    let num_layers = masks.len();
    if num_layers == 0 || width == 0 || height == 0 || radius == 0 || mode == "None" {
        return;
    }
    if start_main_idx >= num_layers || end_main_idx >= num_layers || start_main_idx > end_main_idx {
        return;
    }
    let radius = radius.clamp(1, 6) as usize;
    let kernel = if mode == "Gaussian" {
        make_gaussian_kernel(radius, sigma_z)
    } else if mode == "Linear" || mode == "Tent" {
        let size = 2 * radius + 1;
        let mut k = vec![0.0f32; size];
        let denom = ((radius + 1) * (radius + 1)) as f32;
        for i in 0..size {
            let dist = (i as isize - radius as isize).abs() as usize;
            k[i] = ((radius + 1 - dist) as f32) / denom;
        }
        k
    } else {
        vec![1.0 / (2 * radius + 1) as f32; 2 * radius + 1]
    };

    let (min_x, max_x, min_y, max_y) = roi;
    let w_roi = max_x - min_x + 1;
    let h_roi = max_y - min_y + 1;

    let window_size = 2 * radius + 1;
    // Pre-allocate the circular buffer pool: h_roi * w_roi elements per buffer
    let mut circular_buffers = vec![vec![0u8; w_roi * h_roi]; window_size];
    
    // Load initial layers [start_main_idx - radius..=start_main_idx + radius] (clamped)
    for i in 0..window_size {
        let k = (start_main_idx as isize + i as isize - radius as isize).clamp(0, num_layers as isize - 1) as usize;
        let dest_buf = &mut circular_buffers[i];
        let src_mask = &masks[k];
        for y in min_y..=max_y {
            let src_row = &src_mask[y * width + min_x..=y * width + max_x];
            let dest_row = &mut dest_buf[(y - min_y) * w_roi..(y - min_y) * w_roi + w_roi];
            dest_row.copy_from_slice(src_row);
        }
    }

    let mut blended_layer = vec![0u8; w_roi * h_roi];
    let mut write_idx = 0usize;

    let num_threads = rayon::current_num_threads();
    let rows_per_thread = (h_roi + num_threads - 1) / num_threads;
    let chunk_size = (rows_per_thread * w_roi).max(w_roi);

    for l in start_main_idx..=end_main_idx {
        blended_layer
            .par_chunks_mut(chunk_size)
            .enumerate()
            .for_each(|(chunk_idx, chunk)| {
                let start_row = chunk_idx * rows_per_thread;
                for (local_y_offset, row) in chunk.chunks_exact_mut(w_roi).enumerate() {
                    let local_y = start_row + local_y_offset;
                    let offset = local_y * w_roi;
                    for local_x in 0..w_roi {
                        let mut val = 0.0f32;
                        for (i, &w) in kernel.iter().enumerate() {
                            let circ_idx = (write_idx + i) % window_size;
                            let pixel_val = circular_buffers[circ_idx][offset + local_x] as f32;
                            val += pixel_val * w;
                        }
                        row[local_x] = val.round().clamp(0.0, 255.0) as u8;
                    }
                }
            });

        // Write convolved ROI back to masks[l] directly
        let dest_mask = &mut masks[l];
        for y in min_y..=max_y {
            let src_row = &blended_layer[(y - min_y) * w_roi..(y - min_y) * w_roi + w_roi];
            let dest_row = &mut dest_mask[y * width + min_x..=y * width + max_x];
            dest_row.copy_from_slice(src_row);
        }

        // Slide the window: overwrite the oldest index (write_idx) with layer (l + 1 + radius)
        if l < end_main_idx {
            let next_layer_idx = ((l + 1 + radius) as isize).clamp(0, num_layers as isize - 1) as usize;
            let dest_buf = &mut circular_buffers[write_idx];
            let src_mask = &masks[next_layer_idx];
            for y in min_y..=max_y {
                let src_row = &src_mask[y * width + min_x..=y * width + max_x];
                let dest_row = &mut dest_buf[(y - min_y) * w_roi..(y - min_y) * w_roi + w_roi];
                dest_row.copy_from_slice(src_row);
            }
            write_idx = (write_idx + 1) % window_size;
        }
    }
}

pub fn apply_z_blur(
    masks: &mut [Vec<u8>],
    width: usize,
    height: usize,
    mode: &str,
    radius: u32,
    sigma_z: f32,
    roi: (usize, usize, usize, usize),
) {
    let num_layers = masks.len();
    if num_layers == 0 {
        return;
    }
    apply_z_blur_subrange(masks, width, height, mode, radius, sigma_z, roi, 0, num_layers - 1);
}

pub fn apply_spatial_blurs(job: &SliceJobV3, masks: &mut [Vec<u8>]) {
    let width = job.effective_render_width_px() as usize;
    let height = job.source_height_px as usize;

    let mut active_bounds: Option<(usize, usize, usize, usize)> = None;
    for mask in masks.iter() {
        if !mask.is_empty() {
            if let Some(bounds) = find_active_bounds(mask, width, height) {
                if let Some(ref mut curr) = active_bounds {
                    curr.0 = curr.0.min(bounds.0);
                    curr.1 = curr.1.max(bounds.1);
                    curr.2 = curr.2.min(bounds.2);
                    curr.3 = curr.3.max(bounds.3);
                } else {
                    active_bounds = Some(bounds);
                }
            }
        }
    }

    let Some(bounds) = active_bounds else {
        return;
    };

    // 1. XY Blur (Layer-by-Layer)
    if job.blur_mode_xy != "None" && job.blur_radius_xy > 0 {
        let xy_radius = job.blur_radius_xy as usize;
        let padded_roi = (
            bounds.0.saturating_sub(xy_radius),
            (bounds.1 + xy_radius).min(width.saturating_sub(1)),
            bounds.2.saturating_sub(xy_radius),
            (bounds.3 + xy_radius).min(height.saturating_sub(1)),
        );

        masks.par_iter_mut().for_each(|mask| {
            if !mask.is_empty() {
                apply_xy_blur(
                    mask,
                    width,
                    height,
                    &job.blur_mode_xy,
                    job.blur_radius_xy,
                    job.sigma_x,
                    job.sigma_y,
                    padded_roi,
                );
            }
        });
    }

    // 2. Z Blur (Across Layers)
    if job.enable_z_perturbation && job.blur_mode_z != "None" && job.blur_radius_z > 0 {
        apply_z_blur(
            masks,
            width,
            height,
            &job.blur_mode_z,
            job.blur_radius_z,
            job.sigma_z,
            bounds,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_gaussian_kernel() {
        let kernel = make_gaussian_kernel(1, 1.0);
        assert_eq!(kernel.len(), 3);
        let sum: f32 = kernel.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(kernel[1] > kernel[0]); // Center is higher
        assert!((kernel[0] - kernel[2]).abs() < 1e-5); // Symmetric
    }

    #[test]
    fn test_apply_xy_blur_box() {
        let mut mask = vec![0u8; 9];
        mask[4] = 255; // Center pixel in a 3x3 image
        let bounds = find_active_bounds(&mask, 3, 3).unwrap();
        let padded = (
            bounds.0.saturating_sub(1),
            bounds.1 + 1,
            bounds.2.saturating_sub(1),
            bounds.3 + 1,
        );
        apply_xy_blur(&mut mask, 3, 3, "Box", 1, 1.0, 1.0, padded);
        assert_ne!(mask[4], 255);
        assert!(mask[4] > 0);
    }

    #[test]
    fn test_apply_xy_blur_gaussian() {
        let mut mask = vec![0u8; 9];
        mask[4] = 255; // Center pixel in a 3x3 image
        let bounds = find_active_bounds(&mask, 3, 3).unwrap();
        let padded = (
            bounds.0.saturating_sub(1),
            bounds.1 + 1,
            bounds.2.saturating_sub(1),
            bounds.3 + 1,
        );
        apply_xy_blur(&mut mask, 3, 3, "Gaussian", 1, 1.0, 1.0, padded);
        assert_ne!(mask[4], 255);
        assert!(mask[4] > 0);
    }

    #[test]
    fn test_apply_z_blur() {
        let mut masks = vec![
            vec![255; 4],
            vec![0; 4],
            vec![255; 4],
        ];
        apply_z_blur(&mut masks, 2, 2, "Box", 1, 1.0, (0, 1, 0, 1));
        assert_ne!(masks[1][0], 0);
    }

    #[test]
    fn test_apply_xy_blur_linear() {
        let mut mask = vec![0u8; 9];
        mask[4] = 255; // Center pixel in a 3x3 image
        let bounds = find_active_bounds(&mask, 3, 3).unwrap();
        let padded = (
            bounds.0.saturating_sub(1),
            bounds.1 + 1,
            bounds.2.saturating_sub(1),
            bounds.3 + 1,
        );
        apply_xy_blur(&mut mask, 3, 3, "Linear", 1, 1.0, 1.0, padded);
        assert_ne!(mask[4], 255);
        assert!(mask[4] > 0);
    }

    #[test]
    fn test_apply_z_blur_linear() {
        let mut masks = vec![
            vec![255; 4],
            vec![0; 4],
            vec![255; 4],
        ];
        apply_z_blur(&mut masks, 2, 2, "Linear", 1, 1.0, (0, 1, 0, 1));
        // Middle layer is blurred by [0.25, 0.5, 0.25], so masks[1][0] should be:
        // 255 * 0.25 + 0 * 0.5 + 255 * 0.25 = 127.5 rounded = 128.
        assert_eq!(masks[1][0], 128);
    }

    #[test]
    fn test_apply_z_blur_direct_write_gradients() {
        let mut masks = vec![
            vec![0; 4],
            vec![255; 4],
            vec![0; 4],
        ];
        apply_z_blur(&mut masks, 2, 2, "Linear", 1, 1.0, (0, 1, 0, 1));
        // Under direct-write convolve, the middle layer's 255 is averaged with 0s and correctly falls to 128:
        assert_eq!(masks[1][0], 128);
        assert_eq!(masks[1][1], 128);
        assert_eq!(masks[1][2], 128);
        assert_eq!(masks[1][3], 128);

        // And neighboring layers correctly receive the convolved blurred Z-gradient (64)
        assert_eq!(masks[0][0], 64);
        assert_eq!(masks[2][0], 64);
    }
}
