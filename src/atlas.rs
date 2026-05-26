use std::collections::HashMap;

use crate::types::GridDetection;
use image::{DynamicImage, Rgba, RgbaImage};

pub struct TileAtlas {
    pub image: RgbaImage,
    pub unique_count: usize,
    pub total_cells: usize,
    pub atlas_cols: u32,
    pub atlas_rows: u32,
    pub tile_width: u32,
    pub tile_height: u32,
}

fn mean_abs_diff(a: &[u8], b: &[u8]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let sum: u32 = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| x.abs_diff(y) as u32)
        .sum();
    sum as f32 / a.len() as f32
}

/// Extract every tile cell of the detected grid, deduplicate near-identical
/// tiles, and pack the unique set into a single atlas image. This recovers the
/// underlying tileset from a tilemap.
///
/// `tolerance` is the maximum mean absolute per-channel difference (0..255) for
/// two tiles to be considered the same. Use `0` for exact pixel matching (clean
/// PNG sources); a small value (~8) absorbs JPEG compression noise.
pub fn build_atlas(image: &DynamicImage, detection: &GridDetection, tolerance: f32) -> TileAtlas {
    let rgba = image.to_rgba8();
    let (iw, ih) = (rgba.width(), rgba.height());
    let tw = detection.tile_width.max(1);
    let th = detection.tile_height.max(1);

    // Exact path uses a hash map; tolerant path linear-scans representatives.
    let exact = tolerance <= 0.0;
    let mut exact_index: HashMap<Vec<u8>, ()> = HashMap::new();
    let mut tiles: Vec<RgbaImage> = Vec::new();
    let mut total_cells = 0usize;

    for r in 0..detection.rows {
        for c in 0..detection.columns {
            let x0 = detection.offset_x + c * detection.stride_x;
            let y0 = detection.offset_y + r * detection.stride_y;
            if x0 + tw > iw || y0 + th > ih {
                continue;
            }
            total_cells += 1;

            let mut tile = RgbaImage::new(tw, th);
            for ty in 0..th {
                for tx in 0..tw {
                    tile.put_pixel(tx, ty, *rgba.get_pixel(x0 + tx, y0 + ty));
                }
            }

            let is_new = if exact {
                exact_index.insert(tile.as_raw().clone(), ()).is_none()
            } else {
                !tiles
                    .iter()
                    .any(|rep| mean_abs_diff(rep.as_raw(), tile.as_raw()) <= tolerance)
            };
            if is_new {
                tiles.push(tile);
            }
        }
    }

    let n = tiles.len();
    let cols = (n as f32).sqrt().ceil().max(1.0) as u32;
    let rows = (n as u32).div_ceil(cols).max(1);
    const PAD: u32 = 1;
    let aw = cols * (tw + PAD) + PAD;
    let ah = rows * (th + PAD) + PAD;
    let mut atlas = RgbaImage::from_pixel(aw, ah, Rgba([28, 28, 32, 255]));

    for (i, tile) in tiles.iter().enumerate() {
        let cx = i as u32 % cols;
        let cy = i as u32 / cols;
        let ox = PAD + cx * (tw + PAD);
        let oy = PAD + cy * (th + PAD);
        for ty in 0..th {
            for tx in 0..tw {
                atlas.put_pixel(ox + tx, oy + ty, *tile.get_pixel(tx, ty));
            }
        }
    }

    TileAtlas {
        image: atlas,
        unique_count: n,
        total_cells,
        atlas_cols: cols,
        atlas_rows: rows,
        tile_width: tw,
        tile_height: th,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AxisDetection;

    fn axis(stride: u32) -> AxisDetection {
        AxisDetection {
            stride,
            offset: 0,
            tile_size: stride,
            margin: 0,
            count: 0,
            confidence: 1.0,
            periodicity_score: 1.0,
            offset_score: 1.0,
            margin_score: 1.0,
            coverage_score: 1.0,
        }
    }

    fn detection(stride: u32, cols: u32, rows: u32) -> GridDetection {
        GridDetection {
            detected: true,
            tile_width: stride,
            tile_height: stride,
            stride_x: stride,
            stride_y: stride,
            offset_x: 0,
            offset_y: 0,
            margin_x: 0,
            margin_y: 0,
            columns: cols,
            rows,
            confidence: 1.0,
            x_axis: axis(stride),
            y_axis: axis(stride),
            candidates: Vec::new(),
        }
    }

    #[test]
    fn dedup_identical_tiles() {
        // 4 cells of a 4x4 tile, but only 2 distinct colors (checkerboard of
        // solid tiles) -> 2 unique tiles.
        let t = 4u32;
        let cols = 2u32;
        let rows = 2u32;
        let mut img = RgbaImage::new(cols * t, rows * t);
        for r in 0..rows {
            for c in 0..cols {
                let color = if (r + c) % 2 == 0 {
                    Rgba([255, 0, 0, 255])
                } else {
                    Rgba([0, 0, 255, 255])
                };
                for ty in 0..t {
                    for tx in 0..t {
                        img.put_pixel(c * t + tx, r * t + ty, color);
                    }
                }
            }
        }
        let atlas = build_atlas(&DynamicImage::ImageRgba8(img), &detection(t, cols, rows), 0.0);
        assert_eq!(atlas.total_cells, 4);
        assert_eq!(atlas.unique_count, 2, "expected 2 unique tiles");
    }
}
