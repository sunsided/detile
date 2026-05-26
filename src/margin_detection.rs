use crate::types::MarginDetection;

pub fn local_peak(signal: &[f32], index: usize, radius: usize) -> f32 {
    let n = signal.len();
    let lo = index.saturating_sub(radius);
    let hi = (index + radius + 1).min(n);
    signal[lo..hi]
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max)
}

pub struct AxisStats {
    pub median: f32,
    pub p95: f32,
}

pub fn compute_axis_stats(signal: &[f32]) -> AxisStats {
    if signal.is_empty() {
        return AxisStats {
            median: 0.0,
            p95: 0.0,
        };
    }
    let mut sorted = signal.to_vec();
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let median = if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    };
    let p95_idx = ((n - 1) as f32 * 0.95) as usize;
    let p95 = sorted[p95_idx];
    AxisStats { median, p95 }
}

pub fn score_offset(
    edge_signal: &[f32],
    stride: usize,
    offset: usize,
    axis_len: usize,
    stats: &AxisStats,
) -> (f32, u32) {
    const PEAK_RADIUS: usize = 1;
    const EPSILON: f32 = 1e-6;

    let denom = (stats.p95 - stats.median + EPSILON).max(EPSILON);

    let mut sum_peaks = 0.0_f32;
    let mut count = 0u32;
    let mut pos = offset;
    while pos < axis_len {
        sum_peaks += local_peak(edge_signal, pos, PEAK_RADIUS);
        count += 1;
        pos = match pos.checked_add(stride) {
            Some(v) => v,
            None => break,
        };
    }

    if count < 2 {
        return (0.0, count);
    }

    let mean_peak = sum_peaks / count as f32;
    let score = ((mean_peak - stats.median) / denom).clamp(0.0, 1.0);
    (score, count)
}

pub fn score_coverage(axis_len: usize, stride: usize, offset: usize) -> (f32, u32) {
    if stride == 0 || offset >= axis_len {
        return (0.0, 0);
    }
    let count = ((axis_len - offset) / stride) as u32;
    if count < 2 {
        return (0.0, count);
    }
    let covered = count as usize * stride;
    let leftover = axis_len - offset - covered;
    let coverage = covered as f32 / axis_len as f32;
    let border_penalty = (offset + leftover) as f32 / axis_len as f32;
    let score = (coverage * (1.0 - border_penalty)).clamp(0.0, 1.0);
    (score, count)
}

#[allow(clippy::too_many_arguments)]
pub fn infer_tile_and_margin(
    axis_alpha: &[f32],
    axis_variance: &[f32],
    axis_edge: &[f32],
    stride: usize,
    offset: usize,
    axis_len: usize,
    allow_margin: bool,
    max_margin: usize,
) -> MarginDetection {
    if !allow_margin || stride <= 1 {
        return MarginDetection {
            tile_size: stride as u32,
            margin: 0,
            score: if allow_margin { 0.5 } else { 1.0 },
        };
    }

    let effective_max = max_margin.min(stride / 2).min(32);
    if effective_max == 0 {
        return MarginDetection {
            tile_size: stride as u32,
            margin: 0,
            score: 0.5,
        };
    }

    // Aggregate per-local-index across all full cells
    let mut local_alpha = vec![0.0_f32; stride];
    let mut local_variance = vec![0.0_f32; stride];
    let mut counts = vec![0u32; stride];

    let mut cell_start = offset;
    while cell_start + stride <= axis_len {
        for i in 0..stride {
            let pos = cell_start + i;
            local_alpha[i] += axis_alpha[pos];
            local_variance[i] += axis_variance[pos];
            counts[i] += 1;
        }
        cell_start += stride;
    }

    for i in 0..stride {
        if counts[i] > 0 {
            let c = counts[i] as f32;
            local_alpha[i] /= c;
            local_variance[i] /= c;
        }
    }

    let stride_median_alpha = sorted_median(&local_alpha);
    let stride_median_variance = sorted_median(&local_variance);

    // Position is margin-like when alpha or variance is much lower than stride median
    let is_margin: Vec<bool> = (0..stride)
        .map(|i| {
            let low_alpha = local_alpha[i] < stride_median_alpha * 0.5;
            let low_variance = local_variance[i] < stride_median_variance * 0.3;
            // Also consider edge being very low (quiet band)
            let low_edge = if !axis_edge.is_empty() {
                let pos = offset + i;
                pos < axis_edge.len() && axis_edge[pos] < axis_edge.iter().sum::<f32>() / axis_edge.len() as f32 * 0.2
            } else {
                false
            };
            low_alpha || (low_variance && low_edge)
        })
        .collect();

    let trailing = is_margin.iter().rev().take_while(|&&m| m).count().min(effective_max);
    let leading = is_margin.iter().take_while(|&&m| m).count().min(effective_max);

    let (best_margin, margin_start) = if trailing >= leading {
        // [tile][margin]: margin occupies the last `trailing` positions
        (trailing, stride - trailing)
    } else {
        // [margin][tile]: margin occupies the first `leading` positions
        (leading, 0)
    };

    if best_margin == 0 {
        return MarginDetection {
            tile_size: stride as u32,
            margin: 0,
            score: 0.5,
        };
    }

    let tile_size = stride - best_margin;
    if tile_size == 0 {
        return MarginDetection {
            tile_size: stride as u32,
            margin: 0,
            score: 0.5,
        };
    }

    let hits = (margin_start..margin_start + best_margin)
        .filter(|&i| i < stride && is_margin[i])
        .count();
    let score = (hits as f32 / best_margin as f32).clamp(0.0, 1.0);

    MarginDetection {
        tile_size: tile_size as u32,
        margin: best_margin as u32,
        score,
    }
}

fn sorted_median(vals: &[f32]) -> f32 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut v = vals.to_vec();
    v.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n.is_multiple_of(2) {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    } else {
        v[n / 2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_peak_center() {
        let s = vec![0.0, 0.0, 0.5, 1.0, 0.5, 0.0, 0.0];
        assert!((local_peak(&s, 3, 1) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn local_peak_edge() {
        let s = vec![1.0, 0.5, 0.0, 0.0];
        assert!((local_peak(&s, 0, 2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn coverage_score_perfect() {
        // Exactly fits: stride=10, offset=0, len=100 -> 10 tiles, 0 leftover
        let (score, count) = score_coverage(100, 10, 0);
        assert_eq!(count, 10);
        assert!(score > 0.9, "expected high coverage, got {}", score);
    }

    #[test]
    fn coverage_score_with_offset_and_leftover() {
        // offset=5, stride=10, len=100 -> 9 tiles, leftover=5
        let (score, count) = score_coverage(100, 10, 5);
        assert_eq!(count, 9);
        assert!(score > 0.5);
    }

    #[test]
    fn coverage_rejects_single_tile() {
        let (score, count) = score_coverage(15, 10, 0);
        assert_eq!(count, 1);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn margin_inference_trailing_zeros() {
        let stride = 20;
        let offset = 0;
        let axis_len = 20 * 10;
        // Last 2 positions in stride are transparent
        let alpha: Vec<f32> = (0..axis_len)
            .map(|i| if (i % stride) >= 18 { 0.0 } else { 1.0 })
            .collect();
        let variance: Vec<f32> = alpha.clone();
        let edge: Vec<f32> = vec![0.0; axis_len];

        let det = infer_tile_and_margin(&alpha, &variance, &edge, stride, offset, axis_len, true, 32);
        assert_eq!(det.margin, 2, "expected margin=2, got {}", det.margin);
        assert_eq!(det.tile_size, 18, "expected tile_size=18, got {}", det.tile_size);
        assert!(det.score > 0.5);
    }

    #[test]
    fn margin_inference_no_margin() {
        let stride = 18;
        let axis_len = 18 * 10;
        // Uniform alpha and variance - no margin signal
        let alpha = vec![0.8_f32; axis_len];
        let variance = vec![0.05_f32; axis_len];
        let edge = vec![0.0_f32; axis_len];

        let det = infer_tile_and_margin(&alpha, &variance, &edge, stride, 0, axis_len, true, 32);
        assert_eq!(det.margin, 0);
        assert_eq!(det.tile_size, stride as u32);
    }
}
