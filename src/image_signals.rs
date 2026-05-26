use image::DynamicImage;

pub struct PreparedImage {
    pub width: u32,
    pub height: u32,
    pub luma: Vec<f32>,
    pub alpha: Vec<f32>,
}

impl PreparedImage {
    pub fn from_dynamic(img: &DynamicImage) -> Self {
        let rgba = img.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        let n = (width * height) as usize;
        let mut luma = Vec::with_capacity(n);
        let mut alpha = Vec::with_capacity(n);

        for pixel in rgba.pixels() {
            let [r, g, b, a] = pixel.0;
            let l = 0.2126 * (r as f32 / 255.0)
                + 0.7152 * (g as f32 / 255.0)
                + 0.0722 * (b as f32 / 255.0);
            luma.push(l);
            alpha.push(a as f32 / 255.0);
        }

        Self {
            width,
            height,
            luma,
            alpha,
        }
    }

    #[inline]
    fn luma_at(&self, x: usize, y: usize) -> f32 {
        self.luma[y * self.width as usize + x]
    }

    #[inline]
    fn alpha_at(&self, x: usize, y: usize) -> f32 {
        self.alpha[y * self.width as usize + x]
    }
}

pub fn edge_energy_x(img: &PreparedImage) -> Vec<f32> {
    let w = img.width as usize;
    let h = img.height as usize;
    let mut result = Vec::with_capacity(w);
    result.push(0.0_f32);

    for x in 1..w {
        let mut sum = 0.0_f32;
        for y in 0..h {
            let dl = (img.luma_at(x - 1, y) - img.luma_at(x, y)).abs();
            let da = (img.alpha_at(x - 1, y) - img.alpha_at(x, y)).abs();
            sum += dl + 0.5 * da;
        }
        result.push(sum / h as f32);
    }
    result
}

pub fn edge_energy_y(img: &PreparedImage) -> Vec<f32> {
    let w = img.width as usize;
    let h = img.height as usize;
    let mut result = Vec::with_capacity(h);
    result.push(0.0_f32);

    for y in 1..h {
        let mut sum = 0.0_f32;
        for x in 0..w {
            let dl = (img.luma_at(x, y - 1) - img.luma_at(x, y)).abs();
            let da = (img.alpha_at(x, y - 1) - img.alpha_at(x, y)).abs();
            sum += dl + 0.5 * da;
        }
        result.push(sum / w as f32);
    }
    result
}

pub fn alpha_occupancy_x(img: &PreparedImage) -> Vec<f32> {
    let w = img.width as usize;
    let h = img.height as usize;
    let mut result = Vec::with_capacity(w);

    for x in 0..w {
        let sum: f32 = (0..h).map(|y| img.alpha_at(x, y)).sum();
        result.push(sum / h as f32);
    }
    result
}

pub fn alpha_occupancy_y(img: &PreparedImage) -> Vec<f32> {
    let w = img.width as usize;
    let h = img.height as usize;
    let mut result = Vec::with_capacity(h);

    for y in 0..h {
        let sum: f32 = (0..w).map(|x| img.alpha_at(x, y)).sum();
        result.push(sum / w as f32);
    }
    result
}

pub fn luma_variance_x(img: &PreparedImage) -> Vec<f32> {
    let w = img.width as usize;
    let h = img.height as usize;
    let mut result = Vec::with_capacity(w);

    for x in 0..w {
        let mut sum = 0.0_f32;
        let mut sum_sq = 0.0_f32;
        for y in 0..h {
            let v = img.luma_at(x, y);
            sum += v;
            sum_sq += v * v;
        }
        let mean = sum / h as f32;
        result.push((sum_sq / h as f32 - mean * mean).max(0.0));
    }
    result
}

pub fn luma_variance_y(img: &PreparedImage) -> Vec<f32> {
    let w = img.width as usize;
    let h = img.height as usize;
    let mut result = Vec::with_capacity(h);

    for y in 0..h {
        let row_start = y * w;
        let mut sum = 0.0_f32;
        let mut sum_sq = 0.0_f32;
        for x in 0..w {
            let v = img.luma[row_start + x];
            sum += v;
            sum_sq += v * v;
        }
        let mean = sum / w as f32;
        result.push((sum_sq / w as f32 - mean * mean).max(0.0));
    }
    result
}

pub fn smooth(signal: &[f32], radius: usize) -> Vec<f32> {
    let n = signal.len();
    let mut result = Vec::with_capacity(n);

    for i in 0..n {
        let lo = i.saturating_sub(radius);
        let hi = (i + radius + 1).min(n);
        let sum: f32 = signal[lo..hi].iter().sum();
        result.push(sum / (hi - lo) as f32);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_flat_signal() {
        let s = vec![1.0_f32; 10];
        let out = smooth(&s, 2);
        assert_eq!(out.len(), 10);
        for v in &out {
            assert!((v - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn smooth_impulse() {
        let mut s = vec![0.0_f32; 11];
        s[5] = 1.0;
        let out = smooth(&s, 2);
        // Center region: 1/5 = 0.2
        assert!((out[5] - 0.2).abs() < 1e-6);
        // Edges should be 0
        assert!((out[0]).abs() < 1e-6);
        assert!((out[10]).abs() < 1e-6);
    }

    #[test]
    fn smooth_preserves_length() {
        let s: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = smooth(&s, 1);
        assert_eq!(out.len(), s.len());
    }
}
