# detile

Detect regular tile grids in images (sprite sheets, tile atlases, game maps).

Reduces the image to 1D axis signals, finds periodic strides by autocorrelation,
refines offsets, and splits each stride into tile size plus margin. No brute-force
search over the full `tile × margin × offset` parameter space.

| Overlay view | Atlas view |
|---|---|
| ![grid overlay on the source image](https://raw.githubusercontent.com/sunsided/detile/main/.readme/overlay.png) | ![recovered deduplicated tileset](https://raw.githubusercontent.com/sunsided/detile/main/.readme/atlas.png) |

The interactive viewer: the **Overlay** shows the detected grid on the source
(editable by mouse), and the **Atlas** shows the deduplicated tileset recovered
from that grid.

## Install / build

Install the `detile` binary from crates.io:

```bash
cargo install --locked detile
cargo install --locked --features ui detile   # with the interactive viewer
```

Or build from source:

```bash
cargo build --release
```

Binary: `target/release/detile`. Library crate: `detile`.

## CLI

```bash
detile image.png
detile image.png --json
detile image.png --debug-overlay out.png
detile image.png --atlas tileset.png        # recovered deduped tileset
detile image.png --atlas tileset.png --atlas-tolerance 0   # exact (PNG)
detile image.png --min-stride 8 --max-stride 256
detile image.png --top-candidates 20
detile image.png --prefer-square
detile image.png --no-margin
detile image.png --levels 6
```

`--levels N` reports every distinct periodic scale (fine texture, base tile,
macro layout) instead of just the strongest, each as its own row. Harmonic
multiples of a scale are folded into their fundamental. When several scales are
nested (e.g. a 4px texture inside a 16px tile inside a 40px room), the dominant
one is reported; use `--min-stride` / `--max-stride` to target a specific scale.

## Interactive viewer

An egui viewer lets you flick through the detected configurations and inspect
the recovered tileset. It is behind the optional `ui` feature:

```bash
cargo run --release --features ui -- ui image.png
cargo run --release --features ui -- ui image.png --min-stride 12 --max-stride 28
```

- Dropdown / `←` `→` arrows: switch between detected grid configurations
- Left panel: edit offset / stride / tile per axis; margin and grid counts are
  re-derived and the overlay + atlas update live. `Reset to candidate` restores
  the detected values. Useful when detection locks onto a near-miss offset.
- `Overlay` view: the source image with the grid drawn on top. Mouse editing:
  click sets the offset, left-drag draws the tile rectangle, right-drag sets the
  stride.
- `Atlas` view: every grid cell deduplicated into the unique tileset, packed
  into one image (hover a tile for its index)
- `F1` / `F2` (or `1` / `2`): switch Overlay / Atlas view (`Tab` stays free for
  field navigation) · zoom slider scales the view · atlas tol slider controls
  dedup tolerance

Without `--features ui`, the `ui` subcommand prints a hint to rebuild with it.

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
