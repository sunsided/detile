use detile::{detect_tiling, DetectOptions, DetectionResult};
use image::{DynamicImage, Rgba, RgbaImage};

// ---------------------------------------------------------------------------
// Synthetic image helpers
// ---------------------------------------------------------------------------

/// Generate a distinctive tile image.
/// Each tile has a unique base color plus a smooth interior gradient. The
/// gradient gives non-zero variance (needed for margin detection) while
/// producing only weak internal edges, so the strongest edges occur at tile
/// boundaries where the base color jumps.
fn make_tile(w: u32, h: u32, seed: u8) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    let base_r = 40i32 + (seed.wrapping_mul(37) as i32 % 170);
    let base_g = 40i32 + (seed.wrapping_mul(91) as i32 % 170);
    let base_b = 40i32 + (seed.wrapping_mul(53) as i32 % 170);
    for y in 0..h {
        let gy = (y as f32 / h.max(1) as f32 * 36.0) as i32 - 18;
        for x in 0..w {
            let gx = (x as f32 / w.max(1) as f32 * 36.0) as i32 - 18;
            let r = (base_r + gx).clamp(0, 255) as u8;
            let g = (base_g + gy).clamp(0, 255) as u8;
            let b = base_b.clamp(0, 255) as u8;
            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
    img
}

/// Tile with a weak internal 9-px sub-pattern but strong distinct base color
/// (for the harmonic test). Tile-boundary edges (base color jump) must dominate
/// the internal sub-pattern edges so the detector prefers stride=18 over 9.
fn make_tile_with_subpattern(w: u32, h: u32, seed: u8) -> RgbaImage {
    let mut img = make_tile(w, h, seed);
    let sub = 9u32;
    for y in 0..h {
        for x in 0..w {
            if (x / sub + y / sub).is_multiple_of(2) {
                let [r, g, b, a] = img.get_pixel(x, y).0;
                img.put_pixel(x, y, Rgba([r, g.saturating_add(12), b, a]));
            }
        }
    }
    img
}

/// Build a tiled image from a single tile prototype.
/// All columns use the same tile but each gets a unique color rotation to
/// ensure strong edge energy at every tile boundary.
#[allow(clippy::too_many_arguments)]
fn make_tiled_image(
    tile_w: u32,
    tile_h: u32,
    cols: u32,
    rows: u32,
    margin_x: u32,
    margin_y: u32,
    offset_x: u32,
    offset_y: u32,
    margin_rgba: [u8; 4],
) -> RgbaImage {
    let stride_x = tile_w + margin_x;
    let stride_y = tile_h + margin_y;
    let total_w = offset_x + cols * stride_x;
    let total_h = offset_y + rows * stride_y;

    let mut canvas = RgbaImage::new(total_w, total_h);
    // Fill canvas with margin color
    for px in canvas.pixels_mut() {
        *px = Rgba(margin_rgba);
    }

    for row in 0..rows {
        for col in 0..cols {
            let seed = ((col * 7 + row * 13) % 200) as u8;
            let tile = make_tile(tile_w, tile_h, seed);
            let x0 = offset_x + col * stride_x;
            let y0 = offset_y + row * stride_y;
            for ty in 0..tile_h {
                for tx in 0..tile_w {
                    let px = tile.get_pixel(tx, ty);
                    canvas.put_pixel(x0 + tx, y0 + ty, *px);
                }
            }
        }
    }
    canvas
}

/// Build tiled image with transparent gutter.
fn make_tiled_with_alpha_margin(
    tile_w: u32,
    tile_h: u32,
    cols: u32,
    rows: u32,
    margin: u32,
) -> RgbaImage {
    make_tiled_image(
        tile_w,
        tile_h,
        cols,
        rows,
        margin,
        margin,
        0,
        0,
        [0u8, 0, 0, 0], // transparent margin
    )
}

fn as_dynamic(img: RgbaImage) -> DynamicImage {
    DynamicImage::ImageRgba8(img)
}

fn default_opts() -> DetectOptions {
    DetectOptions {
        min_stride: 4,
        min_confidence: 0.55, // slightly relaxed for synthetic tests
        ..DetectOptions::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test1_18x18_no_margin_no_offset() {
    let img = as_dynamic(make_tiled_image(18, 18, 30, 20, 0, 0, 0, 0, [128u8, 128, 128, 255]));
    let result = detect_tiling(&img, &default_opts()).unwrap();
    match result {
        DetectionResult::Found(d) => {
            assert_eq!(d.stride_x, 18, "stride_x={}", d.stride_x);
            assert_eq!(d.stride_y, 18, "stride_y={}", d.stride_y);
            assert_eq!(d.margin_x, 0);
            assert_eq!(d.margin_y, 0);
            assert_eq!(d.offset_x, 0, "offset_x={}", d.offset_x);
            assert_eq!(d.offset_y, 0, "offset_y={}", d.offset_y);
            assert!(d.confidence >= 0.55, "confidence={}", d.confidence);
        }
        DetectionResult::NotFound { best_confidence, .. } => {
            panic!("expected Found, got NotFound (best_confidence={best_confidence})");
        }
    }
}

#[test]
fn test2_18x18_1px_margin_no_offset() {
    let margin_color = [50u8, 50, 50, 255];
    let img = as_dynamic(make_tiled_image(18, 18, 30, 20, 1, 1, 0, 0, margin_color));
    let result = detect_tiling(&img, &default_opts()).unwrap();
    match result {
        DetectionResult::Found(d) => {
            assert_eq!(d.stride_x, 19, "stride_x={}", d.stride_x);
            assert_eq!(d.stride_y, 19, "stride_y={}", d.stride_y);
            assert_eq!(d.offset_x, 0);
            assert_eq!(d.offset_y, 0);
            assert!(d.confidence >= 0.55, "confidence={}", d.confidence);
        }
        DetectionResult::NotFound { best_confidence, .. } => {
            panic!("expected Found, got NotFound (best_confidence={best_confidence})");
        }
    }
}

#[test]
fn test3_18x18_1px_margin_offset_3x7() {
    let margin_color = [40u8, 40, 40, 255];
    let img = as_dynamic(make_tiled_image(18, 18, 28, 18, 1, 1, 3, 7, margin_color));
    let result = detect_tiling(&img, &default_opts()).unwrap();
    match result {
        DetectionResult::Found(d) => {
            assert_eq!(d.stride_x, 19, "stride_x={}", d.stride_x);
            assert_eq!(d.stride_y, 19, "stride_y={}", d.stride_y);
            assert_eq!(d.offset_x, 3, "offset_x={}", d.offset_x);
            assert_eq!(d.offset_y, 7, "offset_y={}", d.offset_y);
            assert!(d.confidence >= 0.55, "confidence={}", d.confidence);
        }
        DetectionResult::NotFound { best_confidence, .. } => {
            panic!("expected Found, got NotFound (best_confidence={best_confidence})");
        }
    }
}

#[test]
fn test4_16x24_rectangular_tiles() {
    let img = as_dynamic(make_tiled_image(16, 24, 25, 15, 0, 0, 0, 0, [100u8, 100, 100, 255]));
    let result = detect_tiling(&img, &default_opts()).unwrap();
    match result {
        DetectionResult::Found(d) => {
            assert_eq!(d.stride_x, 16, "stride_x={}", d.stride_x);
            assert_eq!(d.stride_y, 24, "stride_y={}", d.stride_y);
            assert!(d.confidence >= 0.55, "confidence={}", d.confidence);
        }
        DetectionResult::NotFound { best_confidence, .. } => {
            panic!("expected Found, got NotFound (best_confidence={best_confidence})");
        }
    }
}

#[test]
fn test5_transparent_margins() {
    let img = as_dynamic(make_tiled_with_alpha_margin(16, 16, 25, 20, 2));
    let result = detect_tiling(&img, &default_opts()).unwrap();
    match result {
        DetectionResult::Found(d) => {
            assert!(d.stride_x == 18 || d.stride_x == 16, "stride_x={}", d.stride_x);
            assert!(d.confidence >= 0.55, "confidence={}", d.confidence);
        }
        DetectionResult::NotFound { best_confidence, .. } => {
            panic!("expected Found, got NotFound (best_confidence={best_confidence})");
        }
    }
}

#[test]
fn test6_solid_color_margins() {
    // Bright solid margin - very distinct from tile content
    let margin_color = [255u8, 255, 255, 255];
    let img = as_dynamic(make_tiled_image(18, 18, 25, 18, 2, 2, 0, 0, margin_color));
    let result = detect_tiling(&img, &default_opts()).unwrap();
    match result {
        DetectionResult::Found(d) => {
            assert!(
                d.stride_x == 20 || d.stride_x == 18,
                "stride_x={}",
                d.stride_x
            );
            assert!(d.confidence >= 0.55, "confidence={}", d.confidence);
        }
        DetectionResult::NotFound { best_confidence, .. } => {
            panic!("expected Found, got NotFound (best_confidence={best_confidence})");
        }
    }
}

#[test]
fn test7_harmonic_prefer_18_over_9() {
    // Tiles are 18x18 with a 9px internal sub-pattern.
    // Algorithm should prefer stride=18 over stride=9.
    let tile_w = 18u32;
    let tile_h = 18u32;
    let cols = 30u32;
    let rows = 20u32;
    let total_w = cols * tile_w;
    let total_h = rows * tile_h;

    let mut canvas = RgbaImage::new(total_w, total_h);
    for row in 0..rows {
        for col in 0..cols {
            let seed = ((col * 7 + row * 13) % 200) as u8;
            let tile = make_tile_with_subpattern(tile_w, tile_h, seed);
            let x0 = col * tile_w;
            let y0 = row * tile_h;
            for ty in 0..tile_h {
                for tx in 0..tile_w {
                    canvas.put_pixel(x0 + tx, y0 + ty, *tile.get_pixel(tx, ty));
                }
            }
        }
    }

    let img = as_dynamic(canvas);
    let result = detect_tiling(&img, &default_opts()).unwrap();
    match result {
        DetectionResult::Found(d) => {
            assert!(
                (15..=20).contains(&d.stride_x),
                "stride_x should be near 18, got {}",
                d.stride_x
            );
        }
        DetectionResult::NotFound { best_confidence, .. } => {
            panic!("expected Found, got NotFound (best_confidence={best_confidence})");
        }
    }
}

#[test]
fn test8_random_noise_not_detected() {
    // Deterministic "noise" using a simple LCG
    let w = 400u32;
    let h = 400u32;
    let mut canvas = RgbaImage::new(w, h);
    let mut state = 12345u64;
    for px in canvas.pixels_mut() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let r = (state >> 56) as u8;
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let g = (state >> 56) as u8;
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let b = (state >> 56) as u8;
        *px = Rgba([r, g, b, 255]);
    }

    let img = as_dynamic(canvas);
    let opts = DetectOptions {
        min_confidence: 0.65,
        ..default_opts()
    };
    let result = detect_tiling(&img, &opts).unwrap();
    match result {
        DetectionResult::Found(d) => {
            panic!(
                "expected NotFound for noise image, got Found (confidence={:.3})",
                d.confidence
            );
        }
        DetectionResult::NotFound { .. } => {
            // Correct - noise should not look like a grid
        }
    }
}

#[test]
fn test9_seamless_gradient_not_detected() {
    // Smooth sinusoidal gradient - no hard tile boundaries
    let w = 400u32;
    let h = 400u32;
    let mut canvas = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 / w as f32 * std::f32::consts::TAU * 3.0;
            let fy = y as f32 / h as f32 * std::f32::consts::TAU * 2.0;
            let v = ((fx.sin() + fy.cos() + 2.0) / 4.0 * 255.0) as u8;
            canvas.put_pixel(x, y, Rgba([v, v, v, 255]));
        }
    }

    let img = as_dynamic(canvas);
    let opts = DetectOptions {
        min_confidence: 0.75, // stricter threshold
        ..default_opts()
    };
    let result = detect_tiling(&img, &opts).unwrap();
    match result {
        DetectionResult::Found(d) => {
            // Accept if confidence is very low - smooth gradients can have
            // accidental correlation but should not hit 0.75
            panic!(
                "expected NotFound for seamless gradient, got Found (confidence={:.3})",
                d.confidence
            );
        }
        DetectionResult::NotFound { .. } => {
            // Correct
        }
    }
}
