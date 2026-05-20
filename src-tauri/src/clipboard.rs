use crate::capture::screen::{default_capture, Rect, ScreenCapture};
use crate::error::JpixelError;
use arboard::Clipboard;
use tauri::AppHandle;

/// Copy a screen region to the clipboard as an image.
#[tauri::command]
pub fn copy_region_to_clipboard(
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

    let mut clipboard = Clipboard::new().map_err(|e| JpixelError::Clipboard(e.to_string()))?;
    let img_data = arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: std::borrow::Cow::Borrowed(image.as_raw()),
    };
    clipboard
        .set_image(img_data)
        .map_err(|e| JpixelError::Clipboard(e.to_string()))?;

    log::info!("Region copied to clipboard (x={}, y={}, w={}, h={})", x, y, width, height);
    Ok(())
}

/// Copy plain text to the clipboard.
#[tauri::command]
pub fn copy_text_to_clipboard(text: String) -> Result<(), JpixelError> {
    let mut clipboard = Clipboard::new().map_err(|e| JpixelError::Clipboard(e.to_string()))?;
    clipboard
        .set_text(text.clone())
        .map_err(|e| JpixelError::Clipboard(e.to_string()))?;
    log::debug!("Text copied to clipboard: {}", text);
    Ok(())
}
