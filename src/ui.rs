use anyhow::Context;
use eframe::egui;

use crate::cli::UiArgs;
use detile::{build_atlas, decompose_layers, detect_levels, draw_overlay, quantize_image, DetectOptions, GridDetection, LayerBaseMode};

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
        layers_threshold: 15.0,
        layers_use_median: false,
        layers_bases: 1,
        quant_enabled: false,
        quant_colors: 256,
        cache: None,
        drag: None,
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
    Layers,
}

#[derive(Clone, Copy)]
enum DragButton {
    Left,
    Right,
}

#[derive(Clone, Copy)]
struct DragOp {
    button: DragButton,
    start: (u32, u32),
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
    layers_tol_key: i32,
    layers_use_median: bool,
    layers_bases: u32,
    quant_key: u32,
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
    work_src: image::DynamicImage,
    overlay: egui::TextureHandle,
    atlas: egui::TextureHandle,
    geom: AtlasGeom,
    layers_ready: bool,
    bases: Vec<egui::TextureHandle>,
    detail_atlas: Option<egui::TextureHandle>,
    decomp_cols: u32,
    decomp_rows: u32,
    decomp_tw: u32,
    decomp_th: u32,
    decomp_total: u32,
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
    layers_threshold: f32,
    layers_use_median: bool,
    layers_bases: u32,
    quant_enabled: bool,
    quant_colors: u32,
    cache: Option<Cache>,
    drag: Option<DragOp>,
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
            layers_tol_key: (self.layers_threshold * 10.0).round() as i32,
            layers_use_median: self.layers_use_median,
            layers_bases: self.layers_bases,
            quant_key: if self.quant_enabled { self.quant_colors } else { 0 },
        }
    }

    // Mouse editing on the overlay (screen coords mapped to image pixels):
    //   left click       -> set offset
    //   left click-drag   -> draw the tile rectangle (offset + tile size)
    //   right click-drag  -> set stride from the drag extent (min 1)
    fn handle_overlay_input(&mut self, ctx: &egui::Context, rect: egui::Rect, resp: &egui::Response) {
        let zoom = self.zoom.max(0.01);
        let iw = self.img_w;
        let ih = self.img_h;
        let to_px = |p: egui::Pos2| -> (u32, u32) {
            let x = ((p.x - rect.min.x) / zoom)
                .floor()
                .clamp(0.0, iw.saturating_sub(1) as f32) as u32;
            let y = ((p.y - rect.min.y) / zoom)
                .floor()
                .clamp(0.0, ih.saturating_sub(1) as f32) as u32;
            (x, y)
        };

        let cur = ctx.input(|i| i.pointer.hover_pos()).map(to_px);
        let (prim_pressed, sec_pressed, prim_released, sec_released) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.secondary_pressed(),
                i.pointer.primary_released(),
                i.pointer.secondary_released(),
            )
        });
        let over = resp.hovered();

        if over && prim_pressed {
            if let Some(s) = cur {
                self.drag = Some(DragOp {
                    button: DragButton::Left,
                    start: s,
                });
            }
        }
        if over && sec_pressed {
            if let Some(s) = cur {
                self.drag = Some(DragOp {
                    button: DragButton::Right,
                    start: s,
                });
            }
        }

        let Some(drag) = self.drag else {
            return;
        };
        if let Some((cx, cy)) = cur {
            let (sx, sy) = drag.start;
            match drag.button {
                DragButton::Left => {
                    let w = cx.abs_diff(sx);
                    let h = cy.abs_diff(sy);
                    if w <= 2 && h <= 2 {
                        // negligible movement: treat as a click -> set offset
                        self.edited.offset_x = sx;
                        self.edited.offset_y = sy;
                    } else {
                        self.edited.offset_x = sx.min(cx);
                        self.edited.offset_y = sy.min(cy);
                        self.edited.tile_width = w.max(1);
                        self.edited.tile_height = h.max(1);
                        // keep stride at least the tile size so the tile fits
                        self.edited.stride_x = self.edited.stride_x.max(self.edited.tile_width);
                        self.edited.stride_y = self.edited.stride_y.max(self.edited.tile_height);
                    }
                }
                DragButton::Right => {
                    self.edited.stride_x = cx.abs_diff(sx).max(1);
                    self.edited.stride_y = cy.abs_diff(sy).max(1);
                }
            }
            self.normalize_edited();
        }

        if prim_released || sec_released {
            self.drag = None;
        }
    }

    fn ensure_cache(&mut self, ctx: &egui::Context) {
        let key = self.current_key();
        if self.cache.as_ref().map(|c| c.key) == Some(key) {
            return;
        }
        let n_colors = if self.quant_enabled { self.quant_colors } else { 0 };
        // Overlay always uses original colors; atlas and layers use quantized source.
        let overlay_img = draw_overlay(&self.source, &self.edited).to_rgba8();
        let work_src = if n_colors > 0 {
            quantize_image(&self.source, n_colors)
        } else {
            self.source.clone()
        };
        let atlas = build_atlas(&work_src, &self.edited, self.tolerance, 0);

        self.cache = Some(Cache {
            key,
            work_src,
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
            layers_ready: false,
            bases: Vec::new(),
            detail_atlas: None,
            decomp_cols: 0,
            decomp_rows: 0,
            decomp_tw: 0,
            decomp_th: 0,
            decomp_total: 0,
        });
    }

    fn ensure_layers(&mut self, ctx: &egui::Context) {
        if self.cache.as_ref().map(|c| c.layers_ready).unwrap_or(true) {
            return;
        }
        let layer_mode = if self.layers_use_median { LayerBaseMode::Median } else { LayerBaseMode::Mode };
        let work_src = self.cache.as_ref().unwrap().work_src.clone();
        let decomp = decompose_layers(
            &work_src,
            &self.edited,
            layer_mode,
            self.layers_threshold,
            0,
            self.layers_bases,
        );
        let cache = self.cache.as_mut().unwrap();
        cache.bases = decomp
            .bases
            .iter()
            .enumerate()
            .map(|(i, b)| to_texture(ctx, &format!("base_{i}"), b))
            .collect();
        cache.detail_atlas = Some(to_texture(ctx, "detail_atlas", &decomp.detail_atlas));
        cache.decomp_cols = decomp.columns;
        cache.decomp_rows = decomp.rows;
        cache.decomp_tw = decomp.tile_width;
        cache.decomp_th = decomp.tile_height;
        cache.decomp_total = decomp.total_cells;
        cache.layers_ready = true;
    }
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let n = self.levels.len();
        // When a text field / DragValue has focus, leave Tab, arrows and digit
        // keys to it; only the dedicated F1/F2/F3 keys switch views unconditionally.
        let editing = ctx.memory(|m| m.focused().is_some());
        ctx.input(|i| {
            if i.key_pressed(egui::Key::F1) {
                self.view = View::Overlay;
            }
            if i.key_pressed(egui::Key::F2) {
                self.view = View::Atlas;
            }
            if i.key_pressed(egui::Key::F3) {
                self.view = View::Layers;
            }
            if !editing {
                if i.key_pressed(egui::Key::Num1) {
                    self.view = View::Overlay;
                }
                if i.key_pressed(egui::Key::Num2) {
                    self.view = View::Atlas;
                }
                if i.key_pressed(egui::Key::Num3) {
                    self.view = View::Layers;
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
                ui.selectable_value(&mut self.view, View::Layers, "Layers");
                ui.separator();
                ui.label("zoom");
                ui.add(egui::Slider::new(&mut self.zoom, 0.25..=12.0));
                ui.separator();
                ui.label("atlas tol");
                ui.add(egui::Slider::new(&mut self.tolerance, 0.0..=32.0));
                ui.separator();
                ui.label("layer tol");
                ui.add(egui::Slider::new(&mut self.layers_threshold, 0.0..=64.0));
                ui.checkbox(&mut self.layers_use_median, "median");
                ui.label("bases");
                ui.add(egui::Slider::new(&mut self.layers_bases, 1u32..=8u32).integer());
                ui.separator();
                ui.checkbox(&mut self.quant_enabled, "quantize");
                if self.quant_enabled {
                    ui.add(egui::Slider::new(&mut self.quant_colors, 2u32..=256u32).integer());
                }
                ui.separator();
                ui.label("←/→ config · F1/F2/F3 (or 1/2/3) view");
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
        if self.view == View::Layers {
            self.ensure_layers(ctx);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.view {
                View::Overlay => {
                    let (id, size) = {
                        let c = self.cache.as_ref().expect("cache built above");
                        (c.overlay.id(), c.overlay.size_vec2())
                    };
                    let sized = egui::load::SizedTexture::new(id, size);
                    ui.label("click = offset · drag = tile · right-drag = stride");
                    egui::ScrollArea::both().show(ui, |ui| {
                        let img_resp = ui.add(egui::Image::new(sized).fit_to_original_size(self.zoom));
                        let resp = ui.interact(
                            img_resp.rect,
                            egui::Id::new("overlay_canvas"),
                            egui::Sense::click_and_drag(),
                        );
                        self.handle_overlay_input(ctx, img_resp.rect, &resp);
                    });
                }
                View::Atlas => {
                    let cache = self.cache.as_ref().expect("cache built above");
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
                View::Layers => {
                    let layers_ready = self.cache.as_ref().map(|c| c.layers_ready).unwrap_or(false);
                    if !layers_ready {
                        ui.label("Computing layers...");
                    } else {
                        // Extract cheap-copy data from cache before closures borrow self
                        let (bases_info, detail_id, detail_sz, tw, th, cols, rows, total) = {
                            let c = self.cache.as_ref().expect("cache built above");
                            let bi: Vec<(egui::TextureId, egui::Vec2)> = c
                                .bases
                                .iter()
                                .map(|b| (b.id(), b.size_vec2()))
                                .collect();
                            let da = c.detail_atlas.as_ref().expect("layers_ready implies detail_atlas");
                            (
                                bi,
                                da.id(),
                                da.size_vec2(),
                                c.decomp_tw,
                                c.decomp_th,
                                c.decomp_cols,
                                c.decomp_rows,
                                c.decomp_total,
                            )
                        };
                        ui.label(format!(
                            "{}×{} tile  ·  {} cells ({}×{} grid)  ·  {} base(s)  ·  thr {:.1}  ·  {}",
                            tw,
                            th,
                            total,
                            cols,
                            rows,
                            bases_info.len(),
                            self.layers_threshold,
                            if self.layers_use_median { "median" } else { "mode" },
                        ));
                        ui.separator();

                        let base_zoom = self.zoom.max((64.0 / tw.max(1) as f32).ceil());
                        let zoom = self.zoom;
                        ui.columns(2, |cols_ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("bases_scroll")
                                .show(&mut cols_ui[0], |ui| {
                                    for (i, (id, sz)) in bases_info.iter().enumerate() {
                                        ui.label(format!("Base {}  ({}×{})", i, tw, th));
                                        ui.add(
                                            egui::Image::new(egui::load::SizedTexture::new(*id, *sz))
                                                .fit_to_original_size(base_zoom),
                                        );
                                        ui.add_space(4.0);
                                    }
                                });
                            cols_ui[1].label("Detail atlas (transparent = matches assigned base)");
                            egui::ScrollArea::both()
                                .id_salt("detail_scroll")
                                .show(&mut cols_ui[1], |ui| {
                                    ui.add(
                                        egui::Image::new(egui::load::SizedTexture::new(
                                            detail_id, detail_sz,
                                        ))
                                        .fit_to_original_size(zoom),
                                    );
                                });
                        });
                    }
                }
            }
        });
    }
}
