use rayon::prelude::*;

use crate::image_signals::smooth;
use crate::margin_detection::{
    compute_axis_stats, infer_tile_and_margin, local_peak, score_coverage, score_offset,
};
use crate::periodicity::{autocorrelation_candidates, expand_harmonics};
use crate::types::{AxisDetection, DetectOptions};

// Single best-ranked stride per axis: score all candidates, collapse harmonic
// relatives toward the fundamental, sort by confidence.
pub fn detect_axis(
    edge: &[f32],
    alpha: &[f32],
    variance: &[f32],
    axis_len: usize,
    options: &DetectOptions,
) -> Vec<AxisDetection> {
    let mut candidates = score_axis_candidates(edge, alpha, variance, axis_len, options, true);
    refine_harmonics(&mut candidates, edge, axis_len);
    candidates.sort_unstable_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(options.top_candidates);
    candidates
}

// Distinct periodic scales present on an axis (texture / tile / macro layout),
// each kept as its own level. Unlike `detect_axis`, harmonic relatives are NOT
// collapsed, so a 16px tile grid survives alongside a 4px texture and a 40px
// room structure. Near-equal strides (within ~6% or 2px) are merged.
pub fn axis_levels(
    edge: &[f32],
    alpha: &[f32],
    variance: &[f32],
    axis_len: usize,
    options: &DetectOptions,
) -> Vec<AxisDetection> {
    // Score only the autocorrelation peaks (no harmonic expansion), so levels
    // come from genuine periodicities rather than coincidental large lags.
    let mut candidates = score_axis_candidates(edge, alpha, variance, axis_len, options, false);

    // Fold harmonics by walking strides ascending. A stride is folded into a
    // smaller kept stride when it is near-equal or an (tolerant) integer
    // multiple of it - the same grid sampled coarser (e.g. 120 = 6*20, or
    // 202 ~= 10*20). Such a stride is dropped UNLESS its periodicity is notably
    // stronger, in which case it is a real coarser structure and kept as its
    // own level. What remains are genuinely distinct, dominant scales.
    candidates.sort_unstable_by_key(|c| c.stride);
    let mut kept: Vec<AxisDetection> = Vec::new();
    for cand in candidates {
        if cand.periodicity_score < 0.5 {
            continue;
        }
        let mut redundant = false;
        for k in &kept {
            let lo = k.stride.min(cand.stride);
            let hi = k.stride.max(cand.stride);
            if lo == 0 {
                continue;
            }
            let tol = 2.0_f32.max(0.06 * hi as f32);
            let rem = (hi % lo) as f32;
            let near_equal = (hi - lo) as f32 <= tol;
            let near_multiple = rem <= tol || (lo as f32 - rem) <= tol;
            if (near_equal || near_multiple)
                && cand.periodicity_score <= k.periodicity_score + 0.04
            {
                redundant = true;
                break;
            }
        }
        if !redundant {
            kept.push(cand);
        }
    }

    kept.sort_unstable_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    kept
}

fn score_axis_candidates(
    edge: &[f32],
    alpha: &[f32],
    variance: &[f32],
    axis_len: usize,
    options: &DetectOptions,
    expand: bool,
) -> Vec<AxisDetection> {
    let min_stride = options.min_stride as usize;
    let max_stride = options
        .max_stride
        .map(|v| v as usize)
        .unwrap_or_else(|| (axis_len / 2).min(512));

    if min_stride > max_stride || axis_len < min_stride * 2 {
        return Vec::new();
    }

    let smoothed = smooth(edge, 1);
    let stats = compute_axis_stats(edge);

    // Sharpness: real grids have flat tile interiors (low median edge) and
    // sharp boundary spikes (high p95). Smooth gradients have edge energy
    // everywhere, so median is close to p95 -> low sharpness. Used as a gate
    // (smoothstep), not a linear multiplier, so textured-but-real images
    // (e.g. JPEG game maps, sharpness ~0.5-0.7) keep full confidence while
    // smooth gradients (sharpness < ~0.3) are rejected.
    let sharpness = {
        const EPS: f32 = 1e-6;
        let raw = ((stats.p95 - stats.median) / (stats.p95 + EPS)).clamp(0.0, 1.0);
        smoothstep(0.30, 0.55, raw)
    };

    // Reference scale for content asymmetry (used to orient offset phase).
    let var_p95 = compute_axis_stats(variance).p95;

    let top_n = options.top_candidates.max(5);
    let initial = autocorrelation_candidates(&smoothed, min_stride, max_stride, top_n);
    let all_strides: Vec<u32> = if expand {
        expand_harmonics(&initial, min_stride, max_stride)
    } else {
        initial.iter().map(|c| c.stride).collect()
    };

    if all_strides.is_empty() {
        return Vec::new();
    }

    let max_margin = options.max_margin.map(|v| v as usize).unwrap_or(32);

    // Pre-compute centered signal for periodicity scoring
    let mean = smoothed.iter().sum::<f32>() / smoothed.len() as f32;
    let centered: Vec<f32> = smoothed.iter().map(|&v| v - mean).collect();

    let candidates: Vec<AxisDetection> = all_strides
        .par_iter()
        .filter_map(|&stride| {
            let s = stride as usize;
            if s < min_stride || s > max_stride {
                return None;
            }

            // Score every offset: tolerant edge alignment, exact alignment,
            // and content asymmetry (content after seam, margin before).
            struct OffScore {
                offset: usize,
                tol: f32,
                exact: f32,
                combined: f32,
                count: u32,
            }
            let scored: Vec<OffScore> = (0..s)
                .filter_map(|off| {
                    let (tol, count) = score_offset(edge, s, off, axis_len, &stats);
                    if count < 2 {
                        return None;
                    }
                    let exact = exact_alignment(edge, s, off, axis_len);
                    let asym = content_asymmetry(variance, s, off, axis_len);
                    let norm_asym = (asym / (var_p95 + 1e-6)).clamp(-1.0, 1.0);
                    let combined = tol + 0.15 * norm_asym;
                    Some(OffScore {
                        offset: off,
                        tol,
                        exact,
                        combined,
                        count,
                    })
                })
                .collect();

            if scored.is_empty() {
                return None;
            }

            let max_combined = scored
                .iter()
                .map(|s| s.combined)
                .fold(f32::NEG_INFINITY, f32::max);

            // Within a small band of the best combined score, pin the phase by
            // exact seam alignment, then prefer the smallest offset.
            let best = scored
                .iter()
                .filter(|s| s.combined >= max_combined - 0.03)
                .max_by(|a, b| {
                    a.exact
                        .partial_cmp(&b.exact)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(b.offset.cmp(&a.offset))
                })
                .unwrap();
            let (best_offset, offset_score, count) = (best.offset, best.tol, best.count);

            let (coverage_score, _) = score_coverage(axis_len, s, best_offset);

            // Normalized autocorrelation score at this specific stride
            let periodicity_score = {
                let n = centered.len();
                if s < n {
                    let a = &centered[0..n - s];
                    let b = &centered[s..n];
                    let dot: f32 = a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum();
                    let norm_a: f32 = a.iter().map(|&x| x * x).sum();
                    let norm_b: f32 = b.iter().map(|&x| x * x).sum();
                    let denom = (norm_a * norm_b).sqrt();
                    if denom > 1e-10 {
                        (dot / denom).clamp(0.0, 1.0)
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            };

            let margin_det = infer_tile_and_margin(
                alpha,
                variance,
                edge,
                s,
                best_offset,
                axis_len,
                options.allow_margin,
                max_margin,
            );

            // Don't over-penalize a clean grid that genuinely has no margin.
            let margin_score = if !options.allow_margin {
                1.0
            } else if margin_det.margin == 0 {
                0.8
            } else {
                margin_det.score
            };

            let base = 0.40 * periodicity_score
                + 0.30 * offset_score
                + 0.20 * margin_score
                + 0.10 * coverage_score;
            let confidence = (base * sharpness).clamp(0.0, 1.0);

            Some(AxisDetection {
                stride,
                offset: best_offset as u32,
                tile_size: margin_det.tile_size,
                margin: margin_det.margin,
                count,
                confidence,
                periodicity_score,
                offset_score,
                margin_score,
                coverage_score,
            })
        })
        .collect();

    candidates
}

// Mean exact edge value at internal tile boundaries (offset + k*stride, k>=1).
// The first boundary (k=0) is the grid's left/top border which carries no edge,
// so it is skipped to avoid biasing the phase by one pixel.
fn exact_alignment(edge: &[f32], stride: usize, offset: usize, axis_len: usize) -> f32 {
    let mut sum = 0.0_f32;
    let mut n = 0u32;
    let mut k = 0usize;
    let mut pos = offset;
    while pos < axis_len {
        if k >= 1 && pos < edge.len() {
            sum += edge[pos];
            n += 1;
        }
        k += 1;
        pos = match pos.checked_add(stride) {
            Some(v) => v,
            None => break,
        };
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f32
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// Content asymmetry across tile-boundary seams: average of (variance just
// inside the predicted tile) minus (variance just before the seam). Positive
// when high-variance content follows the seam and low-variance margin precedes
// it, i.e. the seam is a true tile START rather than a tile END. Disambiguates
// the two edges that bracket a margin gap.
fn content_asymmetry(variance: &[f32], stride: usize, offset: usize, axis_len: usize) -> f32 {
    let mut sum = 0.0_f32;
    let mut n = 0u32;
    let mut k = 0usize;
    let mut pos = offset;
    while pos < axis_len {
        if k >= 1 {
            let after = variance.get(pos + 1).copied().unwrap_or(0.0);
            let before = if pos >= 1 {
                variance[pos - 1]
            } else {
                0.0
            };
            sum += after - before;
            n += 1;
        }
        k += 1;
        pos = match pos.checked_add(stride) {
            Some(v) => v,
            None => break,
        };
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f32
    }
}

// Disambiguate harmonically-related strides.
//
// 1. Sub-harmonic demotion: if a stride's odd-indexed boundaries are much
//    weaker than its even-indexed ones, the boundaries alternate strong/weak,
//    meaning the true tile is twice this stride. Demote it so its double wins.
//    (e.g. a 9px internal sub-pattern inside 18px tiles -> prefer 18.)
// 2. Prefer the fundamental: penalize a larger stride that is an exact multiple
//    of a near-equal smaller stride, so the smallest period that explains the
//    data wins (kills 2x/3x/6x multiples of the true tile).
fn refine_harmonics(candidates: &mut [AxisDetection], edge: &[f32], axis_len: usize) {
    // Step 1: sub-harmonic demotion
    for c in candidates.iter_mut() {
        let s = c.stride as usize;
        let o = c.offset as usize;
        let (mut sum_even, mut n_even, mut sum_odd, mut n_odd) = (0.0_f32, 0u32, 0.0_f32, 0u32);
        let mut k = 0usize;
        let mut pos = o;
        while pos < axis_len {
            let peak = local_peak(edge, pos, 1);
            if k.is_multiple_of(2) {
                sum_even += peak;
                n_even += 1;
            } else {
                sum_odd += peak;
                n_odd += 1;
            }
            k += 1;
            pos = match pos.checked_add(s) {
                Some(v) => v,
                None => break,
            };
        }
        if n_even >= 2 && n_odd >= 2 {
            let mean_even = sum_even / n_even as f32;
            let mean_odd = sum_odd / n_odd as f32;
            if mean_odd < 0.55 * mean_even {
                c.confidence *= 0.55;
            }
        }
    }

    // Step 2: prefer the smallest fundamental among near-equal multiples
    let snapshot: Vec<(u32, f32)> = candidates
        .iter()
        .map(|c| (c.stride, c.confidence))
        .collect();
    for c in candidates.iter_mut() {
        let dominated = snapshot.iter().any(|&(other_s, other_c)| {
            other_s < c.stride
                && c.stride.is_multiple_of(other_s)
                && other_c >= c.confidence * 0.95
        });
        if dominated {
            c.confidence *= 0.80;
        }
    }
}
