use crate::error::CaptureError;
use image::RgbaImage;

/// A rectangle in screen coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Abstraction over screen capture backends.
///
/// Implementations must be thread-safe (`Send + Sync`) because they may be
/// used from async Tauri command handlers.
pub trait ScreenCapture: Send + Sync {
    /// Capture a rectangular region of the screen.
    fn capture_region(&self, rect: Rect) -> Result<RgbaImage, CaptureError>;

    /// Capture a single pixel color.
    ///
    /// Default implementation delegates to `capture_region` with a 1x1 area.
    /// Backends that support direct pixel read should override this.
    fn capture_pixel(&self, x: i32, y: i32) -> Result<image::Rgba<u8>, CaptureError> {
        let img = self.capture_region(Rect {
            x,
            y,
            width: 1,
            height: 1,
        })?;
        Ok(*img.get_pixel(0, 0))
    }

    /// Capture a square area suitable for a magnifier preview.
    fn capture_magnifier(
        &self,
        x: i32,
        y: i32,
        size: u32,
    ) -> Result<RgbaImage, CaptureError> {
        let half = (size / 2) as i32;
        let cx = (x - half).max(0);
        let cy = (y - half).max(0);
        self.capture_region(Rect {
            x: cx,
            y: cy,
            width: size,
            height: size,
        })
    }
}

/// Screen capture implementation backed by the `screenshots` crate.
pub struct ScreenshotsCapture;

impl ScreenCapture for ScreenshotsCapture {
    fn capture_region(&self, rect: Rect) -> Result<RgbaImage, CaptureError> {
        let screen = screenshots::Screen::from_point(rect.x, rect.y).map_err(|e| {
            CaptureError::ScreenNotFound(rect.x, rect.y, e.to_string())
        })?;

        let image = screen
            .capture_area(rect.x, rect.y, rect.width, rect.height)
            .map_err(|e| {
                CaptureError::AreaFailed(rect.x, rect.y, rect.width, rect.height, e.to_string())
            })?;

        Ok(image)
    }
}

/// Global singleton for the default screen capture backend.
use std::sync::OnceLock;

static DEFAULT_CAPTURE: OnceLock<ScreenshotsCapture> = OnceLock::new();

pub fn default_capture() -> &'static ScreenshotsCapture {
    DEFAULT_CAPTURE.get_or_init(|| ScreenshotsCapture)
}
