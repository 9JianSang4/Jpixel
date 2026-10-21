//! Generates a high-contrast, fine-grid test pattern for visual alignment and
//! clipboard checks. The pattern contains no text, so it does not depend on a
//! font or IME.

use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};
use std::path::Path;

const BG: Rgba<u8> = Rgba([24, 24, 28, 255]);
const GRID_10: Rgba<u8> = Rgba([60, 60, 70, 255]);
const GRID_100: Rgba<u8> = Rgba([120, 120, 130, 255]);
const BORDER: Rgba<u8> = Rgba([0, 200, 255, 255]);

const BLOCKS: [(u32, u32, u32, u32, Rgba<u8>); 5] = [
    (100, 100, 200, 200, Rgba([220, 40, 40, 255])),
    (400, 100, 200, 200, Rgba([40, 200, 80, 255])),
    (700, 100, 200, 200, Rgba([40, 100, 220, 255])),
    (100, 400, 200, 200, Rgba([230, 200, 40, 255])),
    (400, 400, 200, 200, Rgba([200, 40, 220, 255])),
];

fn draw_pattern(width: u32, height: u32) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(width, height, BG);

    for x in (0..width).step_by(10) {
        let color = if x % 100 == 0 { GRID_100 } else { GRID_10 };
        for y in 0..height {
            img.put_pixel(x, y, color);
        }
    }
    for y in (0..height).step_by(10) {
        let color = if y % 100 == 0 { GRID_100 } else { GRID_10 };
        for x in 0..width {
            img.put_pixel(x, y, color);
        }
    }

    for (x, y, w, h, color) in BLOCKS {
        if x + w > width || y + h > height {
            continue;
        }
        for py in y..y + h {
            for px in x..x + w {
                img.put_pixel(px, py, color);
            }
        }
    }

    // Checkerboard in the bottom-right corner: catches 1px scaling offsets.
    let cb_size = 320u32;
    let cb_cell = 20u32;
    let start_x = width.saturating_sub(cb_size + 40);
    let start_y = height.saturating_sub(cb_size + 40);
    for y in 0..cb_size {
        for x in 0..cb_size {
            let cell = ((x / cb_cell) + (y / cb_cell)) % 2;
            let color = if cell == 0 {
                Rgba([250, 250, 250, 255])
            } else {
                Rgba([10, 10, 10, 255])
            };
            let px = start_x + x;
            let py = start_y + y;
            if px < width && py < height {
                img.put_pixel(px, py, color);
            }
        }
    }

    for x in 0..width {
        for y in 0..3 {
            img.put_pixel(x, y.min(height - 1), BORDER);
            img.put_pixel(x, height.saturating_sub(1 + y), BORDER);
        }
    }
    for y in 0..height {
        for x in 0..3 {
            img.put_pixel(x.min(width - 1), y, BORDER);
            img.put_pixel(width.saturating_sub(1 + x), y, BORDER);
        }
    }

    img
}

fn ensure(path: &str, width: u32, height: u32) -> Result<()> {
    if Path::new(path).exists() {
        return Ok(());
    }
    let img = draw_pattern(width, height);
    img.save(path).with_context(|| format!("save {}", path))?;
    println!("generated {}", path);
    Ok(())
}

pub fn ensure_patterns() -> Result<()> {
    ensure("test-pattern-1920x1200.png", 1920, 1200)?;
    ensure("test-pattern-1920x1080.png", 1920, 1080)?;
    Ok(())
}
