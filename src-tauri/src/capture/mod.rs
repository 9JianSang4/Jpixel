pub mod screen;

pub use screen::{default_capture, ScreenCapture, Rect};

use crate::error::{io_err, JpixelError};
use base64::Engine;
use image::{DynamicImage, ImageOutputFormat, Rgba};
use imageproc::drawing::draw_filled_circle_mut;
use tauri_plugin_dialog::DialogExt;
use std::io::Cursor;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

// ─────────────────────────────────────────────────────────────
// Stroke compositing (from frontend pen/eraser tools)
// ─────────────────────────────────────────────────────────────

/// A single point in a drawing stroke, in physical pixels relative to the region.
#[derive(serde::Deserialize)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
}

/// A drawing stroke from the frontend capture overlay.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StrokeData {
    Pen {
        points: Vec<StrokePoint>,
        color: String,
        width: f32,
    },
    Eraser {
        points: Vec<StrokePoint>,
        size: f32,
    },
}

fn parse_hex_color(hex: &str) -> Option<Rgba<u8>> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some(Rgba([r, g, b, 255]))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Rgba([r, g, b, 255]))
        }
        _ => None,
    }
}

/// Composite frontend drawing strokes onto a captured image.
///
/// All strokes (pen and eraser) operate on a separate overlay layer.  The
/// overlay is alpha-blended onto the original image at the end, so the
/// eraser only removes pen strokes — the screenshot underneath stays intact.
pub fn composite_strokes(image: &mut image::RgbaImage, strokes_json: &str) -> Result<(), JpixelError> {
    let strokes: Vec<StrokeData> = serde_json::from_str(strokes_json)
        .map_err(|e| JpixelError::Image(format!("Failed to parse strokes: {}", e)))?;

    let (w, h) = image.dimensions();
    let mut overlay = image::RgbaImage::new(w, h);

    for stroke in &strokes {
        match stroke {
            StrokeData::Pen { points, color, width } => {
                let rgba = parse_hex_color(color).unwrap_or(Rgba([255, 0, 0, 255]));
                let radius = (*width / 2.0).max(0.5) as i32;
                let step = (radius as f32 * 0.5).max(1.0);
                for i in 1..points.len() {
                    let (x0, y0) = (points[i - 1].x, points[i - 1].y);
                    let (x1, y1) = (points[i].x, points[i].y);
                    let dx = x1 - x0;
                    let dy = y1 - y0;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let n = (dist / step).ceil() as i32;
                    for j in 0..=n {
                        let t = if n == 0 { 0.0 } else { j as f32 / n as f32 };
                        let cx = (x0 + dx * t) as i32;
                        let cy = (y0 + dy * t) as i32;
                        draw_filled_circle_mut(&mut overlay, (cx, cy), radius, rgba);
                    }
                }
            }
            StrokeData::Eraser { points, size } => {
                let transparent = Rgba([0, 0, 0, 0]);
                let radius = (*size / 2.0).max(1.0) as i32;
                let step = (radius as f32 * 0.5).max(1.0);
                for i in 1..points.len() {
                    let (x0, y0) = (points[i - 1].x, points[i - 1].y);
                    let (x1, y1) = (points[i].x, points[i].y);
                    let dx = x1 - x0;
                    let dy = y1 - y0;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let n = (dist / step).ceil() as i32;
                    for j in 0..=n {
                        let t = if n == 0 { 0.0 } else { j as f32 / n as f32 };
                        let cx = (x0 + dx * t) as i32;
                        let cy = (y0 + dy * t) as i32;
                        draw_filled_circle_mut(&mut overlay, (cx, cy), radius, transparent);
                    }
                }
                if let Some(last) = points.last() {
                    draw_filled_circle_mut(&mut overlay, (last.x as i32, last.y as i32), radius, transparent);
                }
            }
        }
    }

    // Alpha-blend the overlay onto the original image.
    for y in 0..h {
        for x in 0..w {
            let opx = overlay.get_pixel(x, y);
            if opx[3] > 0 {
                let a = opx[3] as f32 / 255.0;
                let ipx = image.get_pixel(x, y);
                image.put_pixel(x, y, Rgba([
                    (opx[0] as f32 * a + ipx[0] as f32 * (1.0 - a)).round() as u8,
                    (opx[1] as f32 * a + ipx[1] as f32 * (1.0 - a)).round() as u8,
                    (opx[2] as f32 * a + ipx[2] as f32 * (1.0 - a)).round() as u8,
                    ipx[3],
                ]));
            }
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────

fn temp_jpixel_dir(app: &AppHandle) -> Result<PathBuf, JpixelError> {
    let temp_dir = app
        .path()
        .temp_dir()
        .map_err(|e| JpixelError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Temp dir: {}", e),
        )))?;
    let dir = temp_dir.join("jpixel");
    std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    Ok(dir)
}

fn now_timestamp() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let fmt = time::format_description::parse(
        "[year]-[month]-[day]-[hour]-[minute]-[second]",
    )
    .unwrap();
    now.format(&fmt).unwrap_or_else(|_| "unknown".into())
}

fn timestamp_filename(prefix: &str, ext: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{}.{}", prefix, nanos, ext)
}

// ─────────────────────────────────────────────────────────────
// Commands
// ─────────────────────────────────────────────────────────────

/// Save a region via a file dialog.
#[tauri::command]
pub async fn save_region_dialog(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    strokes: Option<String>,
) -> Result<(), JpixelError> {
    crate::window::hide_capture_windows(&app);

    let mut image = default_capture()
        .capture_region(Rect { x, y, width, height })
        .map_err(JpixelError::Capture)?;

    // Close hidden windows in background — WebView2 teardown is async
    // and shouldn't block the file dialog.
    let app_close = app.clone();
    std::thread::spawn(move || {
        crate::window::close_capture_windows(app_close);
    });

    if let Some(ref s) = strokes {
        if !s.is_empty() {
            composite_strokes(&mut image, s)?;
        }
    }

    let app_clone = app.clone();
    let file_path: Option<tauri_plugin_dialog::FilePath> =
        tauri::async_runtime::spawn_blocking(move || {
            app_clone
                .dialog()
                .file()
                .add_filter("PNG Image", &["png"])
                .set_file_name(format!("JpixelFile-{}.png", now_timestamp()))
                .blocking_save_file()
        })
        .await
        .map_err(|e| JpixelError::Window(format!("Save dialog thread failed: {}", e)))?;

    if let Some(file_path) = file_path {
        let path = file_path.as_path().ok_or_else(|| {
            JpixelError::Window("Invalid file path".to_string())
        })?;
        image.save(&path).map_err(|e| io_err(&path, e))?;
        log::info!("Screenshot saved to '{:?}'", path);
    } else {
        return Err(JpixelError::DialogCancelled);
    }

    Ok(())
}


/// Create a pin window from a captured region.
#[tauri::command]
pub fn create_pin_from_region(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    strokes: Option<String>,
) -> Result<String, JpixelError> {
    crate::window::hide_capture_windows(&app);

    let mut image = default_capture()
        .capture_region(Rect { x, y, width, height })
        .map_err(JpixelError::Capture)?;

    if let Some(ref s) = strokes {
        if !s.is_empty() {
            composite_strokes(&mut image, s)?;
        }
    }

    let temp_dir = temp_jpixel_dir(&app)?;

    let filename = timestamp_filename("jpixel-pin", "bmp");
    let path = temp_dir.join(&filename);
    image.save(&path).map_err(|e| io_err(&path, e))?;

    let path_str = path.to_string_lossy().to_string();
    // Close capture windows synchronously before spawning pin window.
    // This avoids lock contention with the next create_capture_window call
    // and ensures cleanup happens before the pin window opens.
    crate::window::close_capture_windows(app.clone());
    crate::window::spawn_pin_window(app.clone(), path_str.clone(), x, y, width, height);

    Ok(path_str)
}

/// Read an image file and return it as a base64 data URL.
///
/// The path is validated to prevent directory traversal: only files inside
/// the app’s temp or pictures directories are allowed.
#[tauri::command]
pub fn read_image_base64(app: AppHandle, path: String) -> Result<String, JpixelError> {
    let path = PathBuf::from(&path);

    // Reject any path containing parent-directory references.
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(JpixelError::Config(
                "Path traversal not allowed".to_string(),
            ));
        }
    }

    if !path.is_absolute() {
        return Err(JpixelError::Config(
            "Relative paths not allowed".to_string(),
        ));
    }

    // Determine allowed directories.
    let temp_dir = app
        .path()
        .temp_dir()
        .map_err(|e| JpixelError::Config(format!("Temp dir: {}", e)))?
        .join("jpixel");
    let pic_dir = app
        .path()
        .picture_dir()
        .map_err(|e| JpixelError::Config(format!("Pictures dir: {}", e)))?
        .join("Jpixel");

    let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
    let temp_canon = temp_dir.canonicalize().unwrap_or_else(|_| temp_dir);
    let pic_canon = pic_dir.canonicalize().unwrap_or_else(|_| pic_dir);

    let allowed = canon.starts_with(&temp_canon) || canon.starts_with(&pic_canon);
    if !allowed {
        return Err(JpixelError::Config(
            "Access denied: path outside allowed directories".to_string(),
        ));
    }

    let bytes = std::fs::read(&canon).map_err(|e| io_err(&canon, e))?;
    let mime = match canon.extension().and_then(|e| e.to_str()) {
        Some("bmp") => "image/bmp",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => "image/png",
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, b64))
}

/// RGB color returned by `get_pixel_color`.
#[derive(serde::Serialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Get the RGB color of a single pixel.
#[tauri::command]
pub fn get_pixel_color(x: i32, y: i32) -> Result<RgbColor, JpixelError> {
    let pixel = default_capture()
        .capture_pixel(x, y)
        .map_err(JpixelError::Capture)?;
    Ok(RgbColor {
        r: pixel[0],
        g: pixel[1],
        b: pixel[2],
    })
}

/// Get a magnified area around a point as a base64 PNG.
#[tauri::command]
pub fn get_magnifier_area(x: i32, y: i32, size: u32) -> Result<String, JpixelError> {
    let image = default_capture()
        .capture_magnifier(x, y, size)
        .map_err(JpixelError::Capture)?;

    let dynamic = DynamicImage::ImageRgba8(image);
    let mut cursor = Cursor::new(Vec::new());
    dynamic
        .write_to(&mut cursor, ImageOutputFormat::Png)
        .map_err(|e| JpixelError::Image(format!("PNG encode failed: {}", e)))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(cursor.into_inner());
    Ok(format!("data:image/png;base64,{}", b64))
}

// ─────────────────────────────────────────────────────────────
// Temp file cleanup
// ─────────────────────────────────────────────────────────────

pub fn cleanup_temp_pins_on_startup(app: &AppHandle) {
    let temp_dir = match temp_jpixel_dir(app) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Cannot resolve temp dir for cleanup: {}", e);
            return;
        }
    };
    cleanup_temp_pins(&temp_dir);
}

pub fn cleanup_temp_pins(temp_dir: &PathBuf) {
    let entries = match std::fs::read_dir(temp_dir) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("Failed to read temp pin dir {:?}: {}", temp_dir, e);
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("jpixel-pin-") && name_str.ends_with(".bmp") {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                log::warn!("Failed to remove temp pin file: {}", e);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Stress tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod stress_tests {
    use super::*;

    fn make_test_image(w: u32, h: u32) -> image::RgbaImage {
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 200, 200, 255]))
    }

    fn make_large_strokes(count: usize) -> String {
        let strokes: Vec<serde_json::Value> = (0..count)
            .map(|i| {
                let x = (i * 7 % 1000) as f32;
                let y = (i * 13 % 800) as f32;
                serde_json::json!({
                    "type": "pen",
                    "points": [
                        {"x": x, "y": y},
                        {"x": x + 10.0, "y": y + 10.0},
                        {"x": x + 20.0, "y": y},
                    ],
                    "color": "#FF0000",
                    "width": 3.0,
                })
            })
            .collect();
        serde_json::to_string(&strokes).unwrap()
    }

    /// Composite 500 strokes onto a 1000×800 image.
    #[test]
    #[ignore]
    fn composite_strokes_large() {
        let mut img = make_test_image(1000, 800);
        let strokes = make_large_strokes(500);
        composite_strokes(&mut img, &strokes).unwrap();
    }

    /// Eraser-only strokes.
    #[test]
    #[ignore]
    fn composite_strokes_eraser() {
        let mut img = make_test_image(500, 500);
        let strokes = serde_json::json!([
            {
                "type": "eraser",
                "points": [{"x": 0.0, "y": 0.0}, {"x": 500.0, "y": 500.0}],
                "size": 20.0
            }
        ]);
        composite_strokes(&mut img, &strokes.to_string()).unwrap();
    }

    /// Mixed pen/eraser strokes.
    #[test]
    #[ignore]
    fn composite_strokes_mixed() {
        let mut img = make_test_image(800, 600);
        let strokes = serde_json::json!([
            {
                "type": "pen",
                "points": [{"x": 10.0, "y": 10.0}, {"x": 100.0, "y": 100.0}],
                "color": "#00FF00",
                "width": 5.0
            },
            {
                "type": "eraser",
                "points": [{"x": 20.0, "y": 20.0}, {"x": 50.0, "y": 50.0}],
                "size": 15.0
            }
        ]);
        composite_strokes(&mut img, &strokes.to_string()).unwrap();
    }

    /// Stress: 20 concurrent composite_strokes calls to test pixel-level contention.
    #[test]
    #[ignore]
    fn composite_strokes_concurrent() {
        const THREADS: usize = 10;
        let base_img = make_test_image(500, 400);
        let strokes_json = make_large_strokes(50);

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let img = base_img.clone();
                let s = strokes_json.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        let mut copy = img.clone();
                        composite_strokes(&mut copy, &s).unwrap();
                    }
                })
            })
            .collect();

        for (i, h) in handles.into_iter().enumerate() {
            h.join().expect(&format!("thread {} panicked", i));
        }
    }
}

