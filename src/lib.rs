mod atlas;
mod axis_detection;
mod image_signals;
mod margin_detection;
mod overlay;
mod periodicity;
mod types;

pub use atlas::{build_atlas, decompose_layers, extract_tiles, quantize_image, LayerBaseMode, LayerDecomposition, TileAtlas};
pub use overlay::draw_overlay;
pub use types::*;

use image::DynamicImage;

struct AxisSignals {
    w: usize,
    h: usize,
    edge_x: Vec<f32>,
    alpha_x: Vec<f32>,
    var_x: Vec<f32>,
    edge_y: Vec<f32>,
    alpha_y: Vec<f32>,
    var_y: Vec<f32>,
}

fn prepare_signals(
    image: &DynamicImage,
    options: &DetectOptions,
) -> anyhow::Result<AxisSignals> {
    if let Some(max) = options.max_stride {
        anyhow::ensure!(
            max >= options.min_stride,
            "max_stride ({}) is below min_stride ({})",
            max,
            options.min_stride
        );
    }

    let prepared = image_signals::PreparedImage::from_dynamic(image);
    let w = prepared.width as usize;
    let h = prepared.height as usize;

    anyhow::ensure!(
        w >= (options.min_stride * 2) as usize && h >= (options.min_stride * 2) as usize,
        "image {}x{} too small for min_stride={}",
        w,
        h,
        options.min_stride
    );

    Ok(AxisSignals {
        w,
        h,
        edge_x: image_signals::edge_energy_x(&prepared),
        alpha_x: image_signals::alpha_occupancy_x(&prepared),
        var_x: image_signals::luma_variance_x(&prepared),
        edge_y: image_signals::edge_energy_y(&prepared),
        alpha_y: image_signals::alpha_occupancy_y(&prepared),
        var_y: image_signals::luma_variance_y(&prepared),
    })
}

fn build_grid(
    xc: &AxisDetection,
    yc: &AxisDetection,
    w: usize,
    h: usize,
    options: &DetectOptions,
) -> Option<GridCandidate> {
    let columns = (w.saturating_sub(xc.offset as usize) / xc.stride as usize) as u32;
    let rows = (h.saturating_sub(yc.offset as usize) / yc.stride as usize) as u32;
    if columns < 2 || rows < 2 {
        return None;
    }
    let mut confidence = (xc.confidence * yc.confidence).sqrt();
    if options.prefer_square && (xc.tile_size == yc.tile_size || xc.tile_size.abs_diff(yc.tile_size) <= 1) {
        confidence = (confidence + 0.03).min(1.0);
    }
    Some(GridCandidate {
        tile_width: xc.tile_size,
        tile_height: yc.tile_size,
        stride_x: xc.stride,
        stride_y: yc.stride,
        offset_x: xc.offset,
        offset_y: yc.offset,
        margin_x: xc.margin,
        margin_y: yc.margin,
        columns,
        rows,
        confidence,
    })
}

pub fn detect_tiling(
    image: &DynamicImage,
    options: &DetectOptions,
) -> anyhow::Result<DetectionResult> {
    let sig = prepare_signals(image, options)?;
    let (w, h) = (sig.w, sig.h);

    let x_candidates = axis_detection::detect_axis(&sig.edge_x, &sig.alpha_x, &sig.var_x, w, options);
    let y_candidates = axis_detection::detect_axis(&sig.edge_y, &sig.alpha_y, &sig.var_y, h, options);

    if x_candidates.is_empty() || y_candidates.is_empty() {
        return Ok(DetectionResult::NotFound {
            reason: "no stride candidates found on one or both axes".to_string(),
            best_confidence: 0.0,
            candidates: Vec::new(),
        });
    }

    let top_x = options.top_candidates.min(x_candidates.len());
    let top_y = options.top_candidates.min(y_candidates.len());

    let mut grid_candidates: Vec<GridCandidate> = Vec::with_capacity(top_x * top_y);

    for xc in &x_candidates[..top_x] {
        for yc in &y_candidates[..top_y] {
            if let Some(g) = build_grid(xc, yc, w, h, options) {
                grid_candidates.push(g);
            }
        }
    }

    grid_candidates.sort_unstable_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    grid_candidates.truncate(options.top_candidates);

    let best = grid_candidates.first().cloned();
    let best_confidence = best.as_ref().map(|b| b.confidence).unwrap_or(0.0);

    if best_confidence >= options.min_confidence {
        let best = best.unwrap();
        let best_x = x_candidates
            .iter()
            .find(|c| c.stride == best.stride_x && c.offset == best.offset_x)
            .cloned()
            .unwrap_or_else(|| x_candidates[0].clone());
        let best_y = y_candidates
            .iter()
            .find(|c| c.stride == best.stride_y && c.offset == best.offset_y)
            .cloned()
            .unwrap_or_else(|| y_candidates[0].clone());

        Ok(DetectionResult::Found(GridDetection {
            detected: true,
            tile_width: best.tile_width,
            tile_height: best.tile_height,
            stride_x: best.stride_x,
            stride_y: best.stride_y,
            offset_x: best.offset_x,
            offset_y: best.offset_y,
            margin_x: best.margin_x,
            margin_y: best.margin_y,
            columns: best.columns,
            rows: best.rows,
            confidence: best.confidence,
            x_axis: best_x,
            y_axis: best_y,
            candidates: grid_candidates,
        }))
    } else {
        Ok(DetectionResult::NotFound {
            reason: format!(
                "best confidence {:.3} below threshold {:.3}",
                best_confidence, options.min_confidence
            ),
            best_confidence,
            candidates: grid_candidates,
        })
    }
}

/// Detect every distinct periodic grid scale in the image, not just the
/// strongest. An image may contain several real grids (e.g. a fine texture, the
/// base tile, and a coarse room layout); each is returned as its own
/// `GridDetection`, sorted by confidence. Harmonic relatives are kept separate
/// so the base tile is not hidden behind a sub-texture or macro structure.
///
/// `max_levels` caps the number of distinct grids returned.
pub fn detect_levels(
    image: &DynamicImage,
    options: &DetectOptions,
    max_levels: usize,
) -> anyhow::Result<Vec<GridDetection>> {
    let sig = prepare_signals(image, options)?;
    let (w, h) = (sig.w, sig.h);

    let xs = axis_detection::axis_levels(&sig.edge_x, &sig.alpha_x, &sig.var_x, w, options);
    let ys = axis_detection::axis_levels(&sig.edge_y, &sig.alpha_y, &sig.var_y, h, options);
    if xs.is_empty() || ys.is_empty() {
        return Ok(Vec::new());
    }

    // Combine axis levels, keep the best-scoring grid per (stride_x, stride_y)
    // scale, then deduplicate near-equal scales.
    let mut grids: Vec<(GridCandidate, AxisDetection, AxisDetection)> = Vec::new();
    for xc in &xs {
        for yc in &ys {
            if let Some(g) = build_grid(xc, yc, w, h, options) {
                grids.push((g, xc.clone(), yc.clone()));
            }
        }
    }
    grids.sort_unstable_by(|a, b| {
        b.0.confidence
            .partial_cmp(&a.0.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Two grids are the same scale if each axis is near-equal or a (tolerant)
    // integer multiple (one grid is the other sampled coarser/finer).
    let same_axis = |a: u32, b: u32| -> bool {
        let lo = a.min(b);
        let hi = a.max(b);
        if lo == 0 {
            return false;
        }
        let tol = 2.0_f32.max(0.06 * hi as f32);
        let rem = (hi % lo) as f32;
        (hi - lo) as f32 <= tol || rem <= tol || (lo as f32 - rem) <= tol
    };

    let mut levels: Vec<GridDetection> = Vec::new();
    for (g, xc, yc) in grids {
        let dup = levels
            .iter()
            .any(|l| same_axis(l.stride_x, g.stride_x) && same_axis(l.stride_y, g.stride_y));
        if dup {
            continue;
        }
        levels.push(GridDetection {
            detected: g.confidence >= options.min_confidence,
            tile_width: g.tile_width,
            tile_height: g.tile_height,
            stride_x: g.stride_x,
            stride_y: g.stride_y,
            offset_x: g.offset_x,
            offset_y: g.offset_y,
            margin_x: g.margin_x,
            margin_y: g.margin_y,
            columns: g.columns,
            rows: g.rows,
            confidence: g.confidence,
            x_axis: xc,
            y_axis: yc,
            candidates: Vec::new(),
        });
        if levels.len() >= max_levels {
            break;
        }
    }

    Ok(levels)
}
