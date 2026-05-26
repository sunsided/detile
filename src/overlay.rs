use crate::types::GridDetection;
use image::{DynamicImage, Rgba, RgbaImage};

pub fn draw_overlay(img: &DynamicImage, detection: &GridDetection) -> DynamicImage {
    let mut out = img.to_rgba8();
    let w = out.width();
    let h = out.height();

    let tile_color = Rgba([255u8, 50, 50, 220]);
    let margin_color = Rgba([255u8, 165, 0, 160]);
    let offset_color = Rgba([50u8, 255, 50, 255]);

    // Vertical lines at tile boundaries
    let mut x = detection.offset_x;
    while x < w {
        draw_vline(&mut out, x, 0, h, tile_color);
        if detection.margin_x > 0 {
            let mx = x + detection.tile_width;
            if mx < w {
                draw_vline(&mut out, mx, 0, h, margin_color);
            }
        }
        x = match x.checked_add(detection.stride_x) {
            Some(v) => v,
            None => break,
        };
    }

    // Horizontal lines at tile boundaries
    let mut y = detection.offset_y;
    while y < h {
        draw_hline(&mut out, 0, w, y, tile_color);
        if detection.margin_y > 0 {
            let my = y + detection.tile_height;
            if my < h {
                draw_hline(&mut out, 0, w, my, margin_color);
            }
        }
        y = match y.checked_add(detection.stride_y) {
            Some(v) => v,
            None => break,
        };
    }

    // Cross marker at the first tile origin
    let ox = detection.offset_x;
    let oy = detection.offset_y;
    for d in 0..7u32 {
        let px = ox.saturating_add(d).saturating_sub(3);
        if px < w && oy < h {
            out.put_pixel(px, oy, offset_color);
        }
        let py = oy.saturating_add(d).saturating_sub(3);
        if ox < w && py < h {
            out.put_pixel(ox, py, offset_color);
        }
    }

    DynamicImage::ImageRgba8(out)
}

fn draw_vline(img: &mut RgbaImage, x: u32, y0: u32, y1: u32, color: Rgba<u8>) {
    if x >= img.width() {
        return;
    }
    for y in y0..y1.min(img.height()) {
        img.put_pixel(x, y, color);
    }
}

fn draw_hline(img: &mut RgbaImage, x0: u32, x1: u32, y: u32, color: Rgba<u8>) {
    if y >= img.height() {
        return;
    }
    for x in x0..x1.min(img.width()) {
        img.put_pixel(x, y, color);
    }
}
