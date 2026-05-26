mod cli;

use anyhow::Context;
use clap::Parser;
use cli::Cli;
use detile::{detect_levels, detect_tiling, draw_overlay, DetectOptions, DetectionResult, GridDetection};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let img = image::open(&cli.image)
        .with_context(|| format!("failed to open image: {}", cli.image.display()))?;

    let options = DetectOptions {
        min_stride: cli.min_stride,
        max_stride: cli.max_stride,
        max_margin: cli.max_margin,
        top_candidates: cli.top_candidates,
        prefer_square: cli.prefer_square,
        allow_margin: !cli.no_margin,
        min_confidence: cli.min_confidence,
    };

    if let Some(n) = cli.levels {
        // Widen the candidate pool so distinct scales survive enumeration.
        let level_opts = DetectOptions {
            top_candidates: cli.top_candidates.max(16),
            ..options.clone()
        };
        let levels = detect_levels(&img, &level_opts, n)?;
        print_levels(&levels, cli.json)?;
        return Ok(());
    }

    let result = detect_tiling(&img, &options)?;

    if cli.json {
        let json_val = match &result {
            DetectionResult::Found(d) => serde_json::json!({
                "detected": true,
                "tile_width": d.tile_width,
                "tile_height": d.tile_height,
                "stride_x": d.stride_x,
                "stride_y": d.stride_y,
                "offset_x": d.offset_x,
                "offset_y": d.offset_y,
                "margin_x": d.margin_x,
                "margin_y": d.margin_y,
                "columns": d.columns,
                "rows": d.rows,
                "confidence": d.confidence,
                "x_axis": d.x_axis,
                "y_axis": d.y_axis,
                "candidates": d.candidates,
            }),
            DetectionResult::NotFound {
                reason,
                best_confidence,
                candidates,
            } => serde_json::json!({
                "detected": false,
                "reason": reason,
                "best_confidence": best_confidence,
                "candidates": candidates,
            }),
        };
        println!("{}", serde_json::to_string_pretty(&json_val)?);
    } else {
        match &result {
            DetectionResult::Found(d) => {
                println!("tiling detected: yes");
                println!("tile size:       {} x {}", d.tile_width, d.tile_height);
                println!("stride:          {} x {}", d.stride_x, d.stride_y);
                println!("offset:          {} x {}", d.offset_x, d.offset_y);
                println!("margin:          {} x {}", d.margin_x, d.margin_y);
                println!(
                    "grid:            {} columns x {} rows",
                    d.columns, d.rows
                );
                println!("confidence:      {:.2}", d.confidence);
            }
            DetectionResult::NotFound { best_confidence, .. } => {
                println!("tiling detected: no");
                println!("confidence:      {:.2}", best_confidence);
            }
        }
    }

    if let Some(overlay_path) = &cli.debug_overlay {
        match &result {
            DetectionResult::Found(detection) => {
                let overlay = draw_overlay(&img, detection);
                overlay
                    .save(overlay_path)
                    .with_context(|| format!("failed to save overlay: {}", overlay_path.display()))?;
                eprintln!("overlay saved to {}", overlay_path.display());
            }
            DetectionResult::NotFound { .. } => {
                eprintln!("no tiling detected; overlay not saved");
            }
        }
    }

    Ok(())
}

fn print_levels(levels: &[GridDetection], json: bool) -> anyhow::Result<()> {
    if json {
        let arr: Vec<_> = levels
            .iter()
            .map(|d| {
                serde_json::json!({
                    "detected": d.detected,
                    "tile_width": d.tile_width,
                    "tile_height": d.tile_height,
                    "stride_x": d.stride_x,
                    "stride_y": d.stride_y,
                    "offset_x": d.offset_x,
                    "offset_y": d.offset_y,
                    "margin_x": d.margin_x,
                    "margin_y": d.margin_y,
                    "columns": d.columns,
                    "rows": d.rows,
                    "confidence": d.confidence,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if levels.is_empty() {
        println!("no grid levels detected");
        return Ok(());
    }

    println!("detected {} grid level(s):", levels.len());
    for (i, d) in levels.iter().enumerate() {
        println!(
            "  [{}] tile {}x{}  stride {}x{}  offset {}x{}  margin {}x{}  grid {}x{}  conf {:.2}",
            i + 1,
            d.tile_width,
            d.tile_height,
            d.stride_x,
            d.stride_y,
            d.offset_x,
            d.offset_y,
            d.margin_x,
            d.margin_y,
            d.columns,
            d.rows,
            d.confidence,
        );
    }
    Ok(())
}
