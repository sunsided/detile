# detile

Detect regular tile grids in images (sprite sheets, tile atlases, game maps).

Reduces the image to 1D axis signals, finds periodic strides by autocorrelation,
refines offsets, and splits each stride into tile size plus margin. No brute-force
search over the full `tile × margin × offset` parameter space.

## Install / build

```bash
cargo build --release
```

Binary: `target/release/tile-detect`. Library crate: `detile`.

## CLI

```bash
tile-detect image.png
tile-detect image.png --json
tile-detect image.png --debug-overlay out.png
tile-detect image.png --min-stride 8 --max-stride 256
tile-detect image.png --top-candidates 20
tile-detect image.png --prefer-square
tile-detect image.png --no-margin
tile-detect image.png --levels 6
```

`--levels N` reports every distinct periodic scale (fine texture, base tile,
macro layout) instead of just the strongest, each as its own row. Harmonic
multiples of a scale are folded into their fundamental. When several scales are
nested (e.g. a 4px texture inside a 16px tile inside a 40px room), the dominant
one is reported; use `--min-stride` / `--max-stride` to target a specific scale.

Text output:

```text
tiling detected: yes
tile size:       20 x 16
stride:          20 x 16
offset:          0 x 0
margin:          0 x 0
grid:            95 columns x 63 rows
confidence:      0.93
```

`--json` adds `x_axis`, `y_axis`, and a ranked `candidates` array for diagnostics.

## Library

```rust
use detile::{detect_tiling, DetectOptions, DetectionResult};

let img = image::open("atlas.png")?;
match detect_tiling(&img, &DetectOptions::default())? {
    DetectionResult::Found(grid) => {
        println!("{}x{} stride, {} cols x {} rows",
            grid.stride_x, grid.stride_y, grid.columns, grid.rows);
    }
    DetectionResult::NotFound { best_confidence, .. } => {
        println!("no grid (best {best_confidence:.2})");
    }
}
```

## Concepts

```text
tile_size = visible tile dimensions
margin    = gap between tiles
stride    = tile_size + margin
offset    = first tile start position
```

A result of `tile=18, margin=1, stride=19, offset=3` means tiles start at
`x = 3, 22, 41, ...`, each occupying `[x, x+18)` with a 1px gap before the next.

## Notes

- An image can contain several valid periodicities at different scales (texture,
  base tile, macro layout). The detector reports the strongest; use
  `--min-stride` / `--max-stride` to steer it, and `--top-candidates` to inspect
  alternatives.
- First version is CPU-only, deterministic, no native dependencies. ~0.12s on a
  2 MP image.
