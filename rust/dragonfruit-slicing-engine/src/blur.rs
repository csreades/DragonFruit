use crate::types::SliceJobV3;
use rayon::prelude::*;

fn make_gaussian_kernel(radius: usize, sigma: f32) -> Vec<f32> {
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

    let mut temp = vec![0u8; width * height];

    // Horizontal pass: mask -> temp
    for y in 0..height {
        let row_offset = y * width;
        let src_row = &mask[row_offset..row_offset + width];
        let dest_row = &mut temp[row_offset..row_offset + width];

        for x in 0..width {
            let mut val = 0.0f32;
            for (i, &w) in kernel_x.iter().enumerate() {
                let sample_x = (x as isize + i as isize - radius as isize).clamp(0, width as isize - 1) as usize;
                val += src_row[sample_x] as f32 * w;
            }
            dest_row[x] = val.round().clamp(0.0, 255.0) as u8;
        }
    }

    // Vertical pass: temp -> mask
    for x in 0..width {
        for y in 0..height {
            let mut val = 0.0f32;
            for (i, &w) in kernel_y.iter().enumerate() {
                let sample_y = (y as isize + i as isize - radius as isize).clamp(0, height as isize - 1) as usize;
                val += temp[sample_y * width + x] as f32 * w;
            }
            mask[y * width + x] = val.round().clamp(0.0, 255.0) as u8;
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
) {
    let num_layers = masks.len();
    if num_layers == 0 || width == 0 || height == 0 || radius == 0 || mode == "None" {
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

    // To avoid reading already blurred masks, we keep a sliding window of unblurred masks.
    let mut window: Vec<Vec<u8>> = Vec::with_capacity(2 * radius + 1);
    for i in 0..=(2 * radius) {
        let k = (i as isize - radius as isize).clamp(0, num_layers as isize - 1) as usize;
        window.push(masks[k].clone());
    }

    let mut blended_layer = vec![0u8; width * height];

    for l in 0..num_layers {
        blended_layer.par_chunks_exact_mut(width).enumerate().for_each(|(y, row)| {
            let offset = y * width;
            for x in 0..width {
                let mut val = 0.0f32;
                for (i, &w) in kernel.iter().enumerate() {
                    let pixel_val = window[i][offset + x] as f32;
                    val += pixel_val * w;
                }
                row[x] = val.round().clamp(0.0, 255.0) as u8;
            }
        });

        masks[l].copy_from_slice(&blended_layer);

        // Slide the window for the next layer (l + 1)
        if l + 1 < num_layers {
            window.remove(0);
            let next_layer_idx = ((l + 1 + radius) as isize).clamp(0, num_layers as isize - 1) as usize;
            window.push(masks[next_layer_idx].clone());
        }
    }
}

pub fn apply_spatial_blurs(job: &SliceJobV3, masks: &mut [Vec<u8>]) {
    let width = job.effective_render_width_px() as usize;
    let height = job.source_height_px as usize;

    // 1. XY Blur (Layer-by-Layer)
    if job.blur_mode_xy != "None" && job.blur_radius_xy > 0 {
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
                );
            }
        });
    }

    // 2. Z Blur (Across Layers)
    if job.blur_mode_z != "None" && job.blur_radius_z > 0 {
        apply_z_blur(
            masks,
            width,
            height,
            &job.blur_mode_z,
            job.blur_radius_z,
            job.sigma_z,
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
        apply_xy_blur(&mut mask, 3, 3, "Box", 1, 1.0, 1.0);
        assert_ne!(mask[4], 255);
        assert!(mask[4] > 0);
    }

    #[test]
    fn test_apply_xy_blur_gaussian() {
        let mut mask = vec![0u8; 9];
        mask[4] = 255; // Center pixel in a 3x3 image
        apply_xy_blur(&mut mask, 3, 3, "Gaussian", 1, 1.0, 1.0);
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
        apply_z_blur(&mut masks, 2, 2, "Box", 1, 1.0);
        assert_ne!(masks[1][0], 0);
    }

    #[test]
    fn test_apply_xy_blur_linear() {
        let mut mask = vec![0u8; 9];
        mask[4] = 255; // Center pixel in a 3x3 image
        apply_xy_blur(&mut mask, 3, 3, "Linear", 1, 1.0, 1.0);
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
        apply_z_blur(&mut masks, 2, 2, "Linear", 1, 1.0);
        // Middle layer is blurred by [0.25, 0.5, 0.25], so masks[1][0] should be:
        // 255 * 0.25 + 0 * 0.5 + 255 * 0.25 = 127.5 rounded = 128.
        assert_eq!(masks[1][0], 128);
    }
}
