use std::collections::HashMap;

use crate::types::GridDetection;
use color_quant::NeuQuant;
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

pub enum LayerBaseMode {
    /// Most frequent exact RGBA value per pixel (best for palette/pixel art)
    Mode,
    /// Channel-wise median per pixel (best for lossy/noisy sources)
    Median,
}

pub struct LayerDecomposition {
    /// One W×H base tile per cluster; single element when `n_bases == 1`
    pub bases: Vec<RgbaImage>,
    /// Cluster index for each valid cell (row-major, length == `total_cells`)
    pub base_assignments: Vec<u32>,
    /// Full grid layout (rows×cols slots): pixels that differ from the cell's
    /// assigned base by more than `detail_threshold` are kept; others transparent
    pub detail_atlas: RgbaImage,
    pub tile_width: u32,
    pub tile_height: u32,
    pub columns: u32,
    pub rows: u32,
    pub total_cells: u32,
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

fn sum_abs_diff(a: &[u8], b: &[u8]) -> u32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| x.abs_diff(y) as u32)
        .sum()
}

/// Quantize an image to at most `n_colors` palette entries using NeuQuant.
/// Returns the image unchanged when `n_colors` is 0 or the image is too small
/// to build a meaningful palette.
pub fn quantize_image(image: &DynamicImage, n_colors: u32) -> DynamicImage {
    if n_colors == 0 {
        return image.clone();
    }
    let rgba = image.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let colors = (n_colors as usize).clamp(2, 256);
    // NeuQuant needs at least `colors` training samples
    if rgba.as_raw().len() < colors * 4 {
        return DynamicImage::ImageRgba8(rgba);
    }
    let nq = NeuQuant::new(10, colors, rgba.as_raw());
    let palette = nq.color_map_rgba();
    let out: Vec<u8> = rgba
        .pixels()
        .flat_map(|p| {
            let i = nq.index_of(&p.0) * 4;
            [palette[i], palette[i + 1], palette[i + 2], palette[i + 3]]
        })
        .collect();
    DynamicImage::ImageRgba8(
        RgbaImage::from_raw(w, h, out).expect("output buffer size matches image dimensions"),
    )
}

fn copy_tile(rgba: &RgbaImage, x0: u32, y0: u32, tw: u32, th: u32) -> RgbaImage {
    let mut tile = RgbaImage::new(tw, th);
    for ty in 0..th {
        for tx in 0..tw {
            tile.put_pixel(tx, ty, *rgba.get_pixel(x0 + tx, y0 + ty));
        }
    }
    tile
}

/// Extract all in-bounds grid cells as (row, col, tile) triples.
fn extract_all_tiles(rgba: &RgbaImage, detection: &GridDetection) -> Vec<(u32, u32, RgbaImage)> {
    let (iw, ih) = (rgba.width(), rgba.height());
    let tw = detection.tile_width.max(1);
    let th = detection.tile_height.max(1);
    let mut cells =
        Vec::with_capacity(detection.rows as usize * detection.columns as usize);

    for r in 0..detection.rows {
        for c in 0..detection.columns {
            let x0 = detection.offset_x + c * detection.stride_x;
            let y0 = detection.offset_y + r * detection.stride_y;
            if x0 + tw > iw || y0 + th > ih {
                continue;
            }
            cells.push((r, c, copy_tile(rgba, x0, y0, tw, th)));
        }
    }
    cells
}

/// Return the most frequent value in a sorted slice.
fn mode_of_sorted(sorted: &[u32]) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let mut best_val = sorted[0];
    let mut best_run = 1u32;
    let mut cur_val = sorted[0];
    let mut cur_run = 1u32;
    for &v in &sorted[1..] {
        if v == cur_val {
            cur_run += 1;
            if cur_run > best_run {
                best_run = cur_run;
                best_val = cur_val;
            }
        } else {
            cur_val = v;
            cur_run = 1;
        }
    }
    best_val
}

/// Per-pixel mode (or median) of a set of tiles. Returns a single W×H base image.
fn compute_base_tile(tiles: &[&RgbaImage], tw: u32, th: u32, mode: &LayerBaseMode) -> RgbaImage {
    if tiles.is_empty() {
        return RgbaImage::new(tw, th);
    }
    let n = tiles.len();
    let mut base = RgbaImage::new(tw, th);

    match mode {
        LayerBaseMode::Mode => {
            let mut values: Vec<u32> = Vec::with_capacity(n);
            for py in 0..th {
                for px in 0..tw {
                    values.clear();
                    for tile in tiles {
                        values.push(u32::from_le_bytes(tile.get_pixel(px, py).0));
                    }
                    values.sort_unstable();
                    base.put_pixel(px, py, Rgba(mode_of_sorted(&values).to_le_bytes()));
                }
            }
        }
        LayerBaseMode::Median => {
            let mid = n / 2;
            let mut ch_vals: Vec<u8> = Vec::with_capacity(n);
            for py in 0..th {
                for px in 0..tw {
                    let mut out = [0u8; 4];
                    for ch in 0..4usize {
                        ch_vals.clear();
                        for tile in tiles {
                            ch_vals.push(tile.get_pixel(px, py).0[ch]);
                        }
                        ch_vals.sort_unstable();
                        out[ch] = ch_vals[mid];
                    }
                    base.put_pixel(px, py, Rgba(out));
                }
            }
        }
    }
    base
}

/// Per-pixel channel mean of a set of tiles — cheap centroid for k-means iterations.
fn compute_mean_tile(tiles: &[&RgbaImage], tw: u32, th: u32) -> RgbaImage {
    if tiles.is_empty() {
        return RgbaImage::new(tw, th);
    }
    let n = tiles.len() as f32;
    let mut base = RgbaImage::new(tw, th);
    for py in 0..th {
        for px in 0..tw {
            let mut sums = [0f32; 4];
            for tile in tiles {
                let p = tile.get_pixel(px, py);
                for ch in 0..4 {
                    sums[ch] += p.0[ch] as f32;
                }
            }
            base.put_pixel(
                px,
                py,
                Rgba([
                    (sums[0] / n) as u8,
                    (sums[1] / n) as u8,
                    (sums[2] / n) as u8,
                    (sums[3] / n) as u8,
                ]),
            );
        }
    }
    base
}

/// Pick `k` centroid indices using greedy farthest-first (O(k × N × tile_bytes)).
fn init_centroids(cells: &[(u32, u32, RgbaImage)], k: usize) -> Vec<usize> {
    let k = k.min(cells.len());
    if k == 0 {
        return Vec::new();
    }
    let n = cells.len();
    let mut min_dist: Vec<u32> = (0..n)
        .map(|i| sum_abs_diff(cells[i].2.as_raw(), cells[0].2.as_raw()))
        .collect();
    let mut chosen = vec![0usize];

    while chosen.len() < k {
        let next = min_dist
            .iter()
            .enumerate()
            .max_by_key(|&(_, d)| d)
            .map(|(i, _)| i)
            .unwrap_or(0);
        for i in 0..n {
            let d = sum_abs_diff(cells[i].2.as_raw(), cells[next].2.as_raw());
            if d < min_dist[i] {
                min_dist[i] = d;
            }
        }
        chosen.push(next);
    }
    chosen
}

/// K-means clustering on tile images.
///
/// Uses mean centroids during iteration (fast) and mode/median only for the
/// final cluster representatives (accurate). Runs at most 20 iterations.
///
/// Returns `(bases, assignments)` where `assignments[i]` is the cluster index
/// (0..k) for `cells[i]`.
fn kmeans_tiles(
    cells: &[(u32, u32, RgbaImage)],
    k: usize,
    tw: u32,
    th: u32,
    mode: &LayerBaseMode,
) -> (Vec<RgbaImage>, Vec<usize>) {
    let k = k.min(cells.len()).max(1);
    let n = cells.len();

    let init_idx = init_centroids(cells, k);
    // Mean centroids for fast distance comparisons during iteration
    let mut centroids: Vec<RgbaImage> = init_idx.iter().map(|&i| cells[i].2.clone()).collect();
    let mut assignments = vec![0usize; n];

    for _ in 0..20 {
        let mut changed = false;

        for (i, (_, _, tile)) in cells.iter().enumerate() {
            let nearest = centroids
                .iter()
                .enumerate()
                .min_by_key(|(_, c)| sum_abs_diff(tile.as_raw(), c.as_raw()))
                .map(|(j, _)| j)
                .unwrap_or(0);
            if assignments[i] != nearest {
                assignments[i] = nearest;
                changed = true;
            }
        }

        if !changed {
            break;
        }

        for ci in 0..k {
            let cluster: Vec<&RgbaImage> = cells
                .iter()
                .zip(&assignments)
                .filter(|&(_, a)| *a == ci)
                .map(|((_, _, t), _)| t)
                .collect();
            if !cluster.is_empty() {
                centroids[ci] = compute_mean_tile(&cluster, tw, th);
            }
        }
    }

    // Final pass: accurate mode/median centroids
    let bases: Vec<RgbaImage> = (0..k)
        .map(|ci| {
            let cluster: Vec<&RgbaImage> = cells
                .iter()
                .zip(&assignments)
                .filter(|&(_, a)| *a == ci)
                .map(|((_, _, t), _)| t)
                .collect();
            if cluster.is_empty() {
                centroids[ci].clone()
            } else {
                compute_base_tile(&cluster, tw, th, mode)
            }
        })
        .collect();

    (bases, assignments)
}

/// Extract every tile cell of the detected grid as a flat `Vec<RgbaImage>`,
/// in row-major order. Out-of-bounds cells are silently skipped. Use
/// `build_atlas` to get a deduplicated, packed tileset instead.
pub fn extract_tiles(image: &DynamicImage, detection: &GridDetection) -> Vec<RgbaImage> {
    let rgba = image.to_rgba8();
    extract_all_tiles(&rgba, detection)
        .into_iter()
        .map(|(_, _, tile)| tile)
        .collect()
}

/// Split the detected grid into `n_bases` base layers and a per-cell detail layer.
///
/// When `n_bases == 1` the single base is the per-pixel mode/median across all
/// cells (same as before). When `n_bases > 1` k-means clustering groups cells by
/// visual similarity; each cluster gets its own base image. This handles maps like
/// Vertigo where ground, street, and grass are all distinct underlying bases.
///
/// The **detail atlas** is a full-grid image (one slot per cell). Pixels that
/// differ from the cell's assigned base by more than `detail_threshold` mean
/// per-channel are kept; all others are transparent.
pub fn decompose_layers(
    image: &DynamicImage,
    detection: &GridDetection,
    mode: LayerBaseMode,
    detail_threshold: f32,
    n_colors: u32,
    n_bases: u32,
) -> LayerDecomposition {
    let working;
    let rgba = if n_colors > 0 {
        working = quantize_image(image, n_colors);
        working.to_rgba8()
    } else {
        image.to_rgba8()
    };
    let tw = detection.tile_width.max(1);
    let th = detection.tile_height.max(1);
    let cols = detection.columns;
    let rows = detection.rows;

    let cells = extract_all_tiles(&rgba, detection);
    let total_cells = cells.len() as u32;
    let k = (n_bases as usize).max(1);

    let (bases, assignments) = if k == 1 || cells.is_empty() {
        let refs: Vec<&RgbaImage> = cells.iter().map(|(_, _, t)| t).collect();
        (vec![compute_base_tile(&refs, tw, th, &mode)], vec![0usize; cells.len()])
    } else {
        kmeans_tiles(&cells, k, tw, th, &mode)
    };

    const PAD: u32 = 1;
    let aw = cols * (tw + PAD) + PAD;
    let ah = rows * (th + PAD) + PAD;
    let mut detail_atlas = RgbaImage::from_pixel(aw, ah, Rgba([28, 28, 32, 255]));

    for (j, (r, c, tile)) in cells.iter().enumerate() {
        let base = &bases[assignments[j]];
        let ox = PAD + c * (tw + PAD);
        let oy = PAD + r * (th + PAD);
        for ty in 0..th {
            for tx in 0..tw {
                let tp = tile.get_pixel(tx, ty);
                let bp = base.get_pixel(tx, ty);
                let pixel = if mean_abs_diff(&tp.0, &bp.0) > detail_threshold {
                    *tp
                } else {
                    Rgba([0, 0, 0, 0])
                };
                detail_atlas.put_pixel(ox + tx, oy + ty, pixel);
            }
        }
    }

    LayerDecomposition {
        bases,
        base_assignments: assignments.iter().map(|&a| a as u32).collect(),
        detail_atlas,
        tile_width: tw,
        tile_height: th,
        columns: cols,
        rows,
        total_cells,
    }
}

/// Extract every tile cell of the detected grid, deduplicate near-identical
/// tiles, and pack the unique set into a single atlas image. This recovers the
/// underlying tileset from a tilemap.
///
/// `tolerance` is the maximum mean absolute per-channel difference (0..255) for
/// two tiles to be considered the same. Use `0` for exact pixel matching (clean
/// PNG sources); a small value (~8) absorbs JPEG compression noise.
pub fn build_atlas(
    image: &DynamicImage,
    detection: &GridDetection,
    tolerance: f32,
    n_colors: u32,
) -> TileAtlas {
    let working;
    let rgba = if n_colors > 0 {
        working = quantize_image(image, n_colors);
        working.to_rgba8()
    } else {
        image.to_rgba8()
    };
    let (iw, ih) = (rgba.width(), rgba.height());
    let tw = detection.tile_width.max(1);
    let th = detection.tile_height.max(1);

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

            let tile = copy_tile(&rgba, x0, y0, tw, th);

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
    let atlas_cols = (n as f32).sqrt().ceil().max(1.0) as u32;
    let atlas_rows = (n as u32).div_ceil(atlas_cols).max(1);
    const PAD: u32 = 1;
    let aw = atlas_cols * (tw + PAD) + PAD;
    let ah = atlas_rows * (th + PAD) + PAD;
    let mut atlas = RgbaImage::from_pixel(aw, ah, Rgba([28, 28, 32, 255]));

    for (i, tile) in tiles.iter().enumerate() {
        let cx = i as u32 % atlas_cols;
        let cy = i as u32 / atlas_cols;
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
        atlas_cols,
        atlas_rows,
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

    fn solid_grid(
        tile: u32,
        cols: u32,
        rows: u32,
        color_fn: impl Fn(u32, u32) -> Rgba<u8>,
    ) -> RgbaImage {
        let mut img = RgbaImage::new(cols * tile, rows * tile);
        for r in 0..rows {
            for c in 0..cols {
                let color = color_fn(r, c);
                for ty in 0..tile {
                    for tx in 0..tile {
                        img.put_pixel(c * tile + tx, r * tile + ty, color);
                    }
                }
            }
        }
        img
    }

    #[test]
    fn dedup_identical_tiles() {
        let t = 4u32;
        let cols = 2u32;
        let rows = 2u32;
        let img = solid_grid(t, cols, rows, |r, c| {
            if (r + c) % 2 == 0 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        let atlas =
            build_atlas(&DynamicImage::ImageRgba8(img), &detection(t, cols, rows), 0.0, 0);
        assert_eq!(atlas.total_cells, 4);
        assert_eq!(atlas.unique_count, 2, "expected 2 unique tiles");
    }

    #[test]
    fn extract_tiles_count() {
        let t = 4u32;
        let cols = 3u32;
        let rows = 2u32;
        let img = RgbaImage::new(cols * t, rows * t);
        let tiles = extract_tiles(&DynamicImage::ImageRgba8(img), &detection(t, cols, rows));
        assert_eq!(tiles.len(), (cols * rows) as usize);
    }

    #[test]
    fn decompose_layers_base_is_mode() {
        let t = 4u32;
        let red = Rgba([255u8, 0, 0, 255]);
        let blue = Rgba([0u8, 0, 255, 255]);
        let img = solid_grid(t, 2, 2, |r, c| if r == 1 && c == 1 { blue } else { red });
        let decomp = decompose_layers(
            &DynamicImage::ImageRgba8(img),
            &detection(t, 2, 2),
            LayerBaseMode::Mode,
            15.0,
            0,
            1,
        );
        for py in 0..t {
            for px in 0..t {
                assert_eq!(*decomp.bases[0].get_pixel(px, py), red, "base should be red");
            }
        }
        const PAD: u32 = 1;
        let ox = PAD + 1 * (t + PAD);
        let oy = PAD + 1 * (t + PAD);
        assert_eq!(*decomp.detail_atlas.get_pixel(ox, oy), blue);
    }

    #[test]
    fn decompose_layers_identical_tiles_transparent_detail() {
        let t = 4u32;
        let color = Rgba([100u8, 150, 200, 255]);
        let img = solid_grid(t, 2, 2, |_, _| color);
        let decomp = decompose_layers(
            &DynamicImage::ImageRgba8(img),
            &detection(t, 2, 2),
            LayerBaseMode::Mode,
            0.0,
            0,
            1,
        );
        const PAD: u32 = 1;
        for r in 0..2u32 {
            for c in 0..2u32 {
                let ox = PAD + c * (t + PAD);
                let oy = PAD + r * (t + PAD);
                for ty in 0..t {
                    for tx in 0..t {
                        let p = decomp.detail_atlas.get_pixel(ox + tx, oy + ty);
                        assert_eq!(
                            p.0[3],
                            0,
                            "detail pixel should be transparent when matching base"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn decompose_layers_detail_pixel_preserves_color() {
        let t = 4u32;
        let red = Rgba([255u8, 0, 0, 255]);
        let green = Rgba([0u8, 255, 0, 255]);
        let mut img = solid_grid(t, 2, 1, |_, _| red);
        img.put_pixel(t + 2, 2, green);
        let decomp = decompose_layers(
            &DynamicImage::ImageRgba8(img),
            &detection(t, 2, 1),
            LayerBaseMode::Mode,
            15.0,
            0,
            1,
        );
        const PAD: u32 = 1;
        let ox = PAD + 1 * (t + PAD);
        let oy = PAD;
        assert_eq!(*decomp.detail_atlas.get_pixel(ox + 2, oy + 2), green);
        assert_eq!(decomp.detail_atlas.get_pixel(ox, oy).0[3], 0);
    }

    #[test]
    fn decompose_layers_median_matches_majority() {
        let t = 2u32;
        let dark = Rgba([100u8, 100, 100, 255]);
        let bright = Rgba([200u8, 200, 200, 255]);
        let img = solid_grid(t, 2, 2, |r, c| if r == 1 && c == 1 { bright } else { dark });
        let decomp = decompose_layers(
            &DynamicImage::ImageRgba8(img),
            &detection(t, 2, 2),
            LayerBaseMode::Median,
            15.0,
            0,
            1,
        );
        assert_eq!(*decomp.bases[0].get_pixel(0, 0), dark, "median base should be dark");
    }

    #[test]
    fn decompose_layers_two_bases_separates_colors() {
        // 2 red tiles (top row) + 2 blue tiles (bottom row) → 2 bases should find
        // red and blue as separate bases, not blend them.
        let t = 4u32;
        let red = Rgba([255u8, 0, 0, 255]);
        let blue = Rgba([0u8, 0, 255, 255]);
        let img = solid_grid(t, 2, 2, |r, _| if r == 0 { red } else { blue });
        let decomp = decompose_layers(
            &DynamicImage::ImageRgba8(img),
            &detection(t, 2, 2),
            LayerBaseMode::Mode,
            15.0,
            0,
            2,
        );
        assert_eq!(decomp.bases.len(), 2);
        // Each base should be either pure red or pure blue
        let b0 = *decomp.bases[0].get_pixel(0, 0);
        let b1 = *decomp.bases[1].get_pixel(0, 0);
        let colors: std::collections::HashSet<[u8; 4]> = [b0.0, b1.0].into();
        assert!(colors.contains(&red.0), "one base should be red");
        assert!(colors.contains(&blue.0), "one base should be blue");
        // All detail slots should be fully transparent (tiles match their base exactly)
        const PAD: u32 = 1;
        for r in 0..2u32 {
            for c in 0..2u32 {
                let ox = PAD + c * (t + PAD);
                let oy = PAD + r * (t + PAD);
                for ty in 0..t {
                    for tx in 0..t {
                        assert_eq!(
                            decomp.detail_atlas.get_pixel(ox + tx, oy + ty).0[3],
                            0,
                            "detail should be transparent when tile = base"
                        );
                    }
                }
            }
        }
    }
}
