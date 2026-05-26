use anyhow::Context;
use eframe::egui;

use crate::cli::UiArgs;
use detile::{build_atlas, detect_levels, draw_overlay, DetectOptions, GridDetection};

pub fn run(args: &UiArgs) -> anyhow::Result<()> {
    let img = image::open(&args.image)
        .with_context(|| format!("failed to open image: {}", args.image.display()))?;

    let opts = DetectOptions {
        min_stride: args.min_stride,
        max_stride: args.max_stride,
        max_margin: args.max_margin,
        top_candidates: 16,
        prefer_square: args.prefer_square,
        allow_margin: !args.no_margin,
        min_confidence: 0.0, // list every configuration for inspection
    };

    let levels = detect_levels(&img, &opts, args.max_levels)?;
    anyhow::ensure!(!levels.is_empty(), "no grid configurations detected");

    let edited = levels[0].clone();
    let app = ViewerApp {
        img_w: img.width(),
        img_h: img.height(),
        source: img,
        levels,
        selected: 0,
        last_selected: usize::MAX,
        edited,
        view: View::Overlay,
        zoom: 4.0,
        tolerance: 8.0,
        cache: None,
    };

    let native = eframe::NativeOptions::default();
    eframe::run_native("detile viewer", native, Box::new(|_cc| Ok(Box::new(app))))
        .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;
    Ok(())
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Overlay,
    Atlas,
}

#[derive(PartialEq, Clone, Copy)]
struct CacheKey {
    offset_x: u32,
    offset_y: u32,
    stride_x: u32,
    stride_y: u32,
    tile_width: u32,
    tile_height: u32,
    tol_key: i32,
}

struct AtlasGeom {
    cols: u32,
    tw: u32,
    th: u32,
    pad: u32,
    unique: usize,
    total: usize,
}

struct Cache {
    key: CacheKey,
    overlay: egui::TextureHandle,
    atlas: egui::TextureHandle,
    geom: AtlasGeom,
}

struct ViewerApp {
    source: image::DynamicImage,
    img_w: u32,
    img_h: u32,
    levels: Vec<GridDetection>,
    selected: usize,
    last_selected: usize,
    edited: GridDetection,
    view: View,
    zoom: f32,
    tolerance: f32,
    cache: Option<Cache>,
}

fn to_texture(ctx: &egui::Context, name: &str, img: &image::RgbaImage) -> egui::TextureHandle {
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    ctx.load_texture(name, color, egui::TextureOptions::NEAREST)
}

fn level_label(d: &GridDetection) -> String {
    format!(
        "{}x{}  stride {}x{}  off {}x{}  conf {:.2}",
        d.tile_width, d.tile_height, d.stride_x, d.stride_y, d.offset_x, d.offset_y, d.confidence
    )
}

impl ViewerApp {
    // Clamp edited params to valid ranges and re-derive margin + grid counts.
    fn normalize_edited(&mut self) {
        let e = &mut self.edited;
        e.stride_x = e.stride_x.max(1);
        e.stride_y = e.stride_y.max(1);
        e.tile_width = e.tile_width.clamp(1, e.stride_x);
        e.tile_height = e.tile_height.clamp(1, e.stride_y);
        e.offset_x = e.offset_x.min(self.img_w.saturating_sub(1));
        e.offset_y = e.offset_y.min(self.img_h.saturating_sub(1));
        e.margin_x = e.stride_x - e.tile_width;
        e.margin_y = e.stride_y - e.tile_height;
        e.columns = self.img_w.saturating_sub(e.offset_x) / e.stride_x;
        e.rows = self.img_h.saturating_sub(e.offset_y) / e.stride_y;
    }

    fn current_key(&self) -> CacheKey {
        let e = &self.edited;
        CacheKey {
            offset_x: e.offset_x,
            offset_y: e.offset_y,
            stride_x: e.stride_x,
            stride_y: e.stride_y,
            tile_width: e.tile_width,
            tile_height: e.tile_height,
            tol_key: (self.tolerance * 10.0).round() as i32,
        }
    }

    fn ensure_cache(&mut self, ctx: &egui::Context) {
        let key = self.current_key();
        if self.cache.as_ref().map(|c| c.key) == Some(key) {
            return;
        }
        let overlay_img = draw_overlay(&self.source, &self.edited).to_rgba8();
        let atlas = build_atlas(&self.source, &self.edited, self.tolerance);
        self.cache = Some(Cache {
            key,
            overlay: to_texture(ctx, "overlay", &overlay_img),
            atlas: to_texture(ctx, "atlas", &atlas.image),
            geom: AtlasGeom {
                cols: atlas.atlas_cols,
                tw: atlas.tile_width,
                th: atlas.tile_height,
                pad: 1,
                unique: atlas.unique_count,
                total: atlas.total_cells,
            },
        });
    }
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let n = self.levels.len();
        // When a text field / DragValue has focus, leave Tab, arrows and digit
        // keys to it; only the dedicated F1/F2 keys switch views unconditionally.
        let editing = ctx.memory(|m| m.focused().is_some());
        ctx.input(|i| {
            if i.key_pressed(egui::Key::F1) {
                self.view = View::Overlay;
            }
            if i.key_pressed(egui::Key::F2) {
                self.view = View::Atlas;
            }
            if !editing {
                if i.key_pressed(egui::Key::Num1) {
                    self.view = View::Overlay;
                }
                if i.key_pressed(egui::Key::Num2) {
                    self.view = View::Atlas;
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    self.selected = (self.selected + 1) % n;
                }
                if i.key_pressed(egui::Key::ArrowLeft) {
                    self.selected = (self.selected + n - 1) % n;
                }
            }
        });

        // Switching candidate resets the editable working copy.
        if self.selected != self.last_selected {
            self.edited = self.levels[self.selected].clone();
            self.last_selected = self.selected;
        }

        egui::TopBottomPanel::top("bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Config:");
                egui::ComboBox::from_id_salt("config")
                    .width(360.0)
                    .selected_text(level_label(&self.levels[self.selected]))
                    .show_ui(ui, |ui| {
                        for (i, d) in self.levels.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, i, level_label(d));
                        }
                    });
                ui.label(format!("({}/{})", self.selected + 1, n));
                ui.separator();
                ui.selectable_value(&mut self.view, View::Overlay, "Overlay");
                ui.selectable_value(&mut self.view, View::Atlas, "Atlas");
                ui.separator();
                ui.label("zoom");
                ui.add(egui::Slider::new(&mut self.zoom, 0.25..=12.0));
                ui.separator();
                ui.label("atlas tol");
                ui.add(egui::Slider::new(&mut self.tolerance, 0.0..=32.0));
                ui.separator();
                ui.label("←/→ config · F1/F2 (or 1/2) view");
            });
        });

        egui::SidePanel::left("params")
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Grid (editable)");
                ui.add_space(4.0);
                let iw = self.img_w;
                let ih = self.img_h;
                egui::Grid::new("paramgrid").num_columns(3).show(ui, |ui| {
                    ui.label("");
                    ui.label("X");
                    ui.label("Y");
                    ui.end_row();

                    ui.label("offset");
                    ui.add(egui::DragValue::new(&mut self.edited.offset_x).range(0..=iw));
                    ui.add(egui::DragValue::new(&mut self.edited.offset_y).range(0..=ih));
                    ui.end_row();

                    ui.label("stride");
                    ui.add(egui::DragValue::new(&mut self.edited.stride_x).range(1..=iw));
                    ui.add(egui::DragValue::new(&mut self.edited.stride_y).range(1..=ih));
                    ui.end_row();

                    ui.label("tile");
                    ui.add(
                        egui::DragValue::new(&mut self.edited.tile_width)
                            .range(1..=self.edited.stride_x.max(1)),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.edited.tile_height)
                            .range(1..=self.edited.stride_y.max(1)),
                    );
                    ui.end_row();
                });

                ui.add_space(6.0);
                self.normalize_edited();
                ui.label(format!(
                    "margin: {} x {}",
                    self.edited.margin_x, self.edited.margin_y
                ));
                ui.label(format!(
                    "grid:   {} x {}",
                    self.edited.columns, self.edited.rows
                ));

                ui.add_space(8.0);
                if ui.button("Reset to candidate").clicked() {
                    self.edited = self.levels[self.selected].clone();
                }
            });

        self.ensure_cache(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            let cache = self.cache.as_ref().expect("cache built above");
            match self.view {
                View::Overlay => {
                    let tex = &cache.overlay;
                    let sized = egui::load::SizedTexture::new(tex.id(), tex.size_vec2());
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.add(egui::Image::new(sized).fit_to_original_size(self.zoom));
                    });
                }
                View::Atlas => {
                    let g = &cache.geom;
                    ui.label(format!(
                        "{} unique tiles of {} cells  ·  atlas {} cols  ·  tile {}x{}",
                        g.unique, g.total, g.cols, g.tw, g.th
                    ));
                    let tex = &cache.atlas;
                    let sized = egui::load::SizedTexture::new(tex.id(), tex.size_vec2());
                    egui::ScrollArea::both().show(ui, |ui| {
                        let resp = ui.add(egui::Image::new(sized).fit_to_original_size(self.zoom));
                        if resp.hovered() {
                            if let Some(p) = resp.hover_pos() {
                                let local = p - resp.rect.min;
                                let cw = (g.tw + g.pad) as f32 * self.zoom;
                                let ch = (g.th + g.pad) as f32 * self.zoom;
                                if cw > 0.0 && ch > 0.0 {
                                    let cx = (local.x / cw) as i64;
                                    let cy = (local.y / ch) as i64;
                                    if cx >= 0 && cy >= 0 {
                                        let idx = cy as u64 * g.cols as u64 + cx as u64;
                                        if (idx as usize) < g.unique {
                                            resp.on_hover_text(format!("tile #{idx}"));
                                        }
                                    }
                                }
                            }
                        }
                    });
                }
            }
        });
    }
}
