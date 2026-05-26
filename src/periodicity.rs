use crate::types::StrideCandidate;

pub fn autocorrelation_candidates(
    signal: &[f32],
    min_stride: usize,
    max_stride: usize,
    top_n: usize,
) -> Vec<StrideCandidate> {
    let n = signal.len();
    if n < min_stride * 2 || min_stride > max_stride {
        return Vec::new();
    }
    let max_s = max_stride.min(n / 2);
    if max_s < min_stride {
        return Vec::new();
    }

    let mean = signal.iter().sum::<f32>() / n as f32;
    let centered: Vec<f32> = signal.iter().map(|&v| v - mean).collect();

    let range = max_s - min_stride + 1;
    let mut scores: Vec<f32> = Vec::with_capacity(range);

    for s in min_stride..=max_s {
        let a = &centered[0..n - s];
        let b = &centered[s..n];
        let dot: f32 = a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|&x| x * x).sum();
        let norm_b: f32 = b.iter().map(|&x| x * x).sum();
        let denom = (norm_a * norm_b).sqrt();
        scores.push(if denom > 1e-10 {
            (dot / denom).clamp(0.0, 1.0)
        } else {
            0.0
        });
    }

    // Peak picking with suppression radius 2
    let mut candidates: Vec<StrideCandidate> = Vec::new();
    let mut suppressed = vec![false; range];

    let mut order: Vec<usize> = (0..range).collect();
    order.sort_unstable_by(|&a, &b| {
        scores[b]
            .partial_cmp(&scores[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for idx in order {
        if suppressed[idx] {
            continue;
        }
        candidates.push(StrideCandidate {
            stride: (min_stride + idx) as u32,
            score: scores[idx],
        });
        let lo = idx.saturating_sub(2);
        let hi = (idx + 3).min(range);
        suppressed[lo..hi].iter_mut().for_each(|s| *s = true);
        if candidates.len() >= top_n {
            break;
        }
    }

    candidates
}

pub fn expand_harmonics(
    candidates: &[StrideCandidate],
    min_stride: usize,
    max_stride: usize,
) -> Vec<u32> {
    let mut expanded: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();

    for c in candidates {
        let s = c.stride as usize;
        for div in [4usize, 3, 2] {
            if s.is_multiple_of(div) {
                let sub = s / div;
                if sub >= min_stride {
                    expanded.insert(sub as u32);
                }
            }
        }
        expanded.insert(c.stride);
        for mul in [2usize, 3, 4] {
            let sup = s * mul;
            if sup <= max_stride {
                expanded.insert(sup as u32);
            }
        }
        for delta in -2i32..=2 {
            let nearby = s as i32 + delta;
            if nearby >= min_stride as i32 && nearby <= max_stride as i32 {
                expanded.insert(nearby as u32);
            }
        }
    }

    expanded.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn periodic_signal(period: usize, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| if i % period == 0 { 1.0_f32 } else { 0.0 })
            .collect()
    }

    #[test]
    fn autocorr_finds_period() {
        // A perfect Dirac comb has score=1.0 at ALL multiples of the period,
        // so we can't assert "best stride == 18". Instead verify that 18 is
        // in the top candidates and has a high score.
        let signal = periodic_signal(18, 18 * 60);
        let candidates = autocorrelation_candidates(&signal, 4, 512, 10);
        assert!(!candidates.is_empty(), "should find at least one candidate");
        let found = candidates.iter().find(|c| c.stride == 18);
        assert!(
            found.is_some(),
            "stride 18 should be in top-10 candidates; got: {:?}",
            candidates.iter().map(|c| c.stride).collect::<Vec<_>>()
        );
        assert!(
            found.unwrap().score > 0.7,
            "score for stride 18 should be high"
        );
    }

    #[test]
    fn autocorr_empty_on_small_signal() {
        let signal = vec![1.0_f32; 5];
        let candidates = autocorrelation_candidates(&signal, 4, 512, 5);
        assert!(candidates.is_empty());
    }

    #[test]
    fn expand_harmonics_includes_multiples() {
        let candidates = vec![StrideCandidate {
            stride: 18,
            score: 0.9,
        }];
        let expanded = expand_harmonics(&candidates, 4, 200);
        assert!(expanded.contains(&9), "should include 18/2=9");
        assert!(expanded.contains(&18), "should include self");
        assert!(expanded.contains(&36), "should include 18*2=36");
        assert!(expanded.contains(&54), "should include 18*3=54");
    }

    #[test]
    fn expand_harmonics_respects_bounds() {
        let candidates = vec![StrideCandidate {
            stride: 18,
            score: 0.9,
        }];
        let expanded = expand_harmonics(&candidates, 10, 30);
        for &s in &expanded {
            assert!((10..=30).contains(&s), "stride {} out of bounds", s);
        }
    }
}
