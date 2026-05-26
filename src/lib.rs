mod axis_detection;
mod image_signals;
mod margin_detection;
mod overlay;
mod periodicity;
mod types;

pub use overlay::draw_overlay;
pub use types::*;

use image::DynamicImage;

pub fn detect_tiling(
    image: &DynamicImage,
    options: &DetectOptions,
) -> anyhow::Result<DetectionResult> {
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

    let edge_x = image_signals::edge_energy_x(&prepared);
    let alpha_x = image_signals::alpha_occupancy_x(&prepared);
    let var_x = image_signals::luma_variance_x(&prepared);

    let edge_y = image_signals::edge_energy_y(&prepared);
    let alpha_y = image_signals::alpha_occupancy_y(&prepared);
    let var_y = image_signals::luma_variance_y(&prepared);

    let x_candidates = axis_detection::detect_axis(&edge_x, &alpha_x, &var_x, w, options);
    let y_candidates = axis_detection::detect_axis(&edge_y, &alpha_y, &var_y, h, options);

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
            let columns = ((w.saturating_sub(xc.offset as usize)) / xc.stride as usize) as u32;
            let rows = ((h.saturating_sub(yc.offset as usize)) / yc.stride as usize) as u32;

            if columns < 2 || rows < 2 {
                continue;
            }

            let mut confidence = (xc.confidence * yc.confidence).sqrt();

            if options.prefer_square {
                let tw = xc.tile_size;
                let th = yc.tile_size;
                if tw == th || tw.abs_diff(th) <= 1 {
                    confidence = (confidence + 0.03).min(1.0);
                }
            }

            grid_candidates.push(GridCandidate {
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
            });
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
