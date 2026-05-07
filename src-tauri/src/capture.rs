use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub async fn save_region_dialog(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<(), String> {
    use screenshots::Screen;
    use tauri_plugin_dialog::DialogExt;

    let screen = Screen::from_point(x, y)
        .map_err(|e| format!("Failed to get screen at ({},{}): {}", x, y, e))?;
    let image = screen
        .capture_area(x, y, width, height)
        .map_err(|e| format!("Failed to capture area (x={}, y={}, w={}, h={}): {}", x, y, width, height, e))?;

    let app_clone = app.clone();
    let file_path = tauri::async_runtime::spawn_blocking(move || {
        app_clone
            .dialog()
            .file()
            .add_filter("PNG Image", &["png"])
            .set_file_name("screenshot.png")
            .blocking_save_file()
    })
    .await
    .map_err(|e| format!("Save dialog thread failed: {}", e))?;

    if let Some(file_path) = file_path {
        let raw = file_path.to_string();
        // Tauri v2 dialog may return file:// URI on some platforms; normalize to local path
        let cleaned = raw.strip_prefix("file:///").unwrap_or(&raw);
        let path = std::path::PathBuf::from(cleaned);
        image
            .save(&path)
            .map_err(|e| format!("Failed to save image to '{:?}': {}", path, e))?;
        log::info!("Screenshot saved to '{:?}'", path);
    }

    Ok(())
}


/// Clean up old temporary pin files from the temp directory.
pub fn cleanup_temp_pins(temp_dir: &PathBuf) {
    if let Ok(entries) = std::fs::read_dir(temp_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("jpixel-pin-") && name_str.ends_with(".png") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[tauri::command]
pub fn capture_screen_region(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<String, String> {
    use arboard::Clipboard;
    use screenshots::Screen;

    let screen = Screen::from_point(x, y)
        .map_err(|e| format!("Failed to get screen at ({},{}): {}", x, y, e))?;
    let image = screen
        .capture_area(x, y, width, height)
        .map_err(|e| format!("Failed to capture area (x={}, y={}, w={}, h={}): {}", x, y, width, height, e))?;

    let picture_dir = app
        .path()
        .picture_dir()
        .map_err(|e| format!("Failed to get picture directory: {}", e))?;
    let save_dir = picture_dir.join("Jpixel");
    std::fs::create_dir_all(&save_dir)
        .map_err(|e| format!("Failed to create save directory: {}", e))?;

    let filename = format!(
        "jpixel-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let path = save_dir.join(&filename);
    image
        .save(&path)
        .map_err(|e| format!("Failed to save screenshot to {:?}: {}", path, e))?;

    let mut clipboard = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;
    let img_data = arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: std::borrow::Cow::Borrowed(image.as_raw()),
    };
    clipboard
        .set_image(img_data)
        .map_err(|e| format!("Failed to copy image to clipboard: {}", e))?;

    log::info!("Screenshot saved to {:?}", path);
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn create_pin_from_region(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<String, String> {
    use screenshots::Screen;

    let screen = Screen::from_point(x, y)
        .map_err(|e| format!("Failed to get screen at ({},{}): {}", x, y, e))?;
    let image = screen
        .capture_area(x, y, width, height)
        .map_err(|e| format!("Failed to capture area (x={}, y={}, w={}, h={}): {}", x, y, width, height, e))?;

    let temp_dir = app
        .path()
        .temp_dir()
        .map_err(|e| format!("Failed to get temp directory: {}", e))?;
    let jpixel_temp = temp_dir.join("jpixel");
    std::fs::create_dir_all(&jpixel_temp)
        .map_err(|e| format!("Failed to create temp directory: {}", e))?;

    // Clean up old temp files before creating new one
    cleanup_temp_pins(&jpixel_temp);

    let filename = format!(
        "jpixel-pin-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let path = jpixel_temp.join(&filename);
    image
        .save(&path)
        .map_err(|e| format!("Failed to save pin temp file to {:?}: {}", path, e))?;

    log::info!("Pin temp file saved to {:?}", path);

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn get_pixel_color(x: i32, y: i32) -> Result<String, String> {
    use screenshots::Screen;

    let screen = Screen::from_point(x, y)
        .map_err(|e| format!("Failed to get screen at ({},{}): {}", x, y, e))?;
    let image = screen
        .capture_area(x, y, 1, 1)
        .map_err(|e| format!("Failed to capture pixel at ({},{}): {}", x, y, e))?;

    let pixel = image.get_pixel(0, 0);
    Ok(format!("{},{},{}", pixel[0], pixel[1], pixel[2]))
}

#[tauri::command]
pub fn get_magnifier_area(x: i32, y: i32, size: u32) -> Result<String, String> {
    use base64::Engine;
    use image::{DynamicImage, ImageOutputFormat};
    use screenshots::Screen;
    use std::io::Cursor;

    let half = (size / 2) as i32;
    let capture_x = (x - half).max(0);
    let capture_y = (y - half).max(0);

    let screen = Screen::from_point(x, y)
        .map_err(|e| format!("Failed to get screen at ({},{}): {}", x, y, e))?;
    let image = screen
        .capture_area(capture_x, capture_y, size, size)
        .map_err(|e| format!(
            "Failed to capture magnifier area at ({},{} size={}): {}",
            x, y, size, e
        ))?;

    let dynamic = DynamicImage::ImageRgba8(image);
    let mut cursor = Cursor::new(Vec::new());
    dynamic
        .write_to(&mut cursor, ImageOutputFormat::Png)
        .map_err(|e| format!("Failed to encode magnifier image: {}", e))?;

    let base64_str = base64::engine::general_purpose::STANDARD.encode(cursor.into_inner());
    Ok(format!("data:image/png;base64,{}", base64_str))
}
