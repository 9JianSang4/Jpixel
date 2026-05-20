pub mod gif;
pub mod screen;

pub use screen::{default_capture, ScreenCapture, Rect};

use crate::error::{io_err, JpixelError};
use crate::ocr::OcrEngine;
use base64::Engine;
use image::{DynamicImage, ImageOutputFormat};
use tauri_plugin_dialog::DialogExt;
use std::io::Cursor;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

// ─────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────

fn screenshot_dir(app: &AppHandle) -> Result<PathBuf, JpixelError> {
    let picture_dir = app
        .path()
        .picture_dir()
        .map_err(|e| JpixelError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Pictures dir: {}", e),
        )))?;
    let dir = picture_dir.join("Jpixel");
    std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    Ok(dir)
}

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
) -> Result<(), JpixelError> {
    crate::window::hide_capture_windows(&app);

    let image = default_capture()
        .capture_region(Rect { x, y, width, height })
        .map_err(JpixelError::Capture)?;

    // Close the hidden windows so the file dialog shows the desktop behind it.
    crate::window::close_capture_windows(app.clone());

    let app_clone = app.clone();
    let file_path: Option<tauri_plugin_dialog::FilePath> =
        tauri::async_runtime::spawn_blocking(move || {
            app_clone
                .dialog()
                .file()
                .add_filter("PNG Image", &["png"])
                .set_file_name("screenshot.png")
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

/// Capture a region, save to Pictures/Jpixel, copy to clipboard, and open editor.
#[tauri::command]
pub fn capture_screen_region(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<String, JpixelError> {
    // Hide the overlay before capturing so the magnifier / toolbar are not
    // included in the screenshot image.
    crate::window::hide_capture_windows(&app);

    let image = default_capture()
        .capture_region(Rect { x, y, width, height })
        .map_err(JpixelError::Capture)?;

    let save_dir = screenshot_dir(&app)?;
    let filename = timestamp_filename("jpixel", "png");
    let path = save_dir.join(&filename);
    image.save(&path).map_err(|e| io_err(&path, e))?;

    copy_image_to_clipboard(&image)?;

    log::info!("Screenshot saved to {:?}", path);
    Ok(path.to_string_lossy().to_string())
}

/// Create a pin window from a captured region.
#[tauri::command]
pub fn create_pin_from_region(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<String, JpixelError> {
    crate::window::hide_capture_windows(&app);

    let image = default_capture()
        .capture_region(Rect { x, y, width, height })
        .map_err(JpixelError::Capture)?;

    let temp_dir = temp_jpixel_dir(&app)?;
    cleanup_temp_pins(&temp_dir);

    let filename = timestamp_filename("jpixel-pin", "png");
    let path = temp_dir.join(&filename);
    image.save(&path).map_err(|e| io_err(&path, e))?;

    let path_str = path.to_string_lossy().to_string();
    crate::window::spawn_pin_window(app.clone(), path_str.clone(), x, y, width, height);
    crate::window::close_capture_windows(app);

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
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:image/png;base64,{}", b64))
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
// Clipboard helper
// ─────────────────────────────────────────────────────────────

fn copy_image_to_clipboard(image: &image::RgbaImage) -> Result<(), JpixelError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| JpixelError::Clipboard(e.to_string()))?;
    let img_data = arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: std::borrow::Cow::Borrowed(image.as_raw()),
    };
    clipboard
        .set_image(img_data)
        .map_err(|e| JpixelError::Clipboard(e.to_string()))?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Temp file cleanup
// ─────────────────────────────────────────────────────────────

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
        if name_str.starts_with("jpixel-pin-") && name_str.ends_with(".png") {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                log::warn!("Failed to remove temp pin file: {}", e);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// OCR command
// ─────────────────────────────────────────────────────────────

#[tauri::command]
pub fn ocr_region(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<String, JpixelError> {
    crate::window::hide_capture_windows(&app);

    let image = default_capture()
        .capture_region(Rect { x, y, width, height })
        .map_err(JpixelError::Capture)?;

    let ocr = crate::ocr::default_ocr();
    let text = ocr
        .recognize(&image)
        .map_err(JpixelError::Ocr)?;

    Ok(text)
}

// ─────────────────────────────────────────────────────────────
// GIF recording commands
// ─────────────────────────────────────────────────────────────

use crate::capture::gif::GifRecorder;
use std::sync::{Mutex, OnceLock};

fn gif_recorder() -> &'static Mutex<GifRecorder> {
    static INSTANCE: OnceLock<Mutex<GifRecorder>> = OnceLock::new();
    INSTANCE.get_or_init(|| Mutex::new(GifRecorder::new()))
}

#[tauri::command]
pub fn start_gif_record(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    fps: u8,
    scale: f32,
    quality: u8,
) -> Result<(), JpixelError> {
    let mut recorder = gif_recorder().lock().map_err(|_| {
        JpixelError::Gif(crate::error::GifError::External("GIF recorder mutex poisoned".to_string()))
    })?;
    recorder
        .start(Rect { x, y, width, height }, fps, scale, quality)
        .map_err(JpixelError::Gif)
}

#[tauri::command]
pub fn stop_gif_record(
    app: AppHandle,
) -> Result<String, JpixelError> {
    let mut recorder = gif_recorder().lock().map_err(|_| {
        JpixelError::Gif(crate::error::GifError::External("GIF recorder mutex poisoned".to_string()))
    })?;

    let save_dir = screenshot_dir(&app)?;
    let filename = timestamp_filename("jpixel", "gif");
    let path = save_dir.join(&filename);

    recorder
        .stop(path.clone())
        .map_err(JpixelError::Gif)?;

    Ok(path.to_string_lossy().to_string())
}
