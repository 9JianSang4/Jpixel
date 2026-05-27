use crate::error::CaptureError;
use image::RgbaImage;
use std::sync::{Mutex, OnceLock};

/// A rectangle in screen coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Global lock that serialises all DXGI screen-capture operations.
///
/// The `screenshots` crate creates a fresh `IDXGIOutputDuplication` interface
/// per call.  Calling `AcquireNextFrame` concurrently from multiple threads
/// can cause the graphics driver to stall or hang permanently.  This mutex
/// ensures only one capture is in-flight at any time.
fn capture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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
        // Wait for the DWM to finish compositing the current frame before
        // capturing.  This ensures that any DOM element we just hid in the
        // frontend has actually disappeared from the screen, preventing
        // the toolbar / selection box from appearing in screenshots.
        #[cfg(target_os = "windows")]
        unsafe {
            let _ = windows::Win32::Graphics::Dwm::DwmFlush();
        }

        let _lock = capture_lock().lock().unwrap();

        // Fast path: if the region fits inside a single monitor, use the
        // point-based API which returns the pixel buffer directly (zero copy).
        // This covers 95%+ of captures and avoids the stitching overhead.
        if let Ok(screen) = screenshots::Screen::from_point(rect.x, rect.y) {
            let info = &screen.display_info;
            let sx = info.x;
            let sy = info.y;
            let sw = info.width as i32;
            let sh = info.height as i32;

            if rect.x >= sx
                && rect.y >= sy
                && rect.x + rect.width as i32 <= sx + sw
                && rect.y + rect.height as i32 <= sy + sh
            {
                return screen
                    .capture_area(rect.x, rect.y, rect.width, rect.height)
                    .map_err(|e| CaptureError::AreaFailed(rect.x, rect.y, rect.width, rect.height, e.to_string()));
            }
        }

        // Slow path: multi-monitor stitching via row-level memcpy.
        let screens = screenshots::Screen::all().map_err(|e| {
            CaptureError::ScreenNotFound(rect.x, rect.y, e.to_string())
        })?;

        let mut result = RgbaImage::new(rect.width, rect.height);
        let dst_stride = rect.width as usize * 4;

        for screen in &screens {
            let info = &screen.display_info;
            let sx = info.x;
            let sy = info.y;
            let sw = info.width as i32;
            let sh = info.height as i32;

            let ix = rect.x.max(sx);
            let iy = rect.y.max(sy);
            let iw = (rect.x + rect.width as i32).min(sx + sw) - ix;
            let ih = (rect.y + rect.height as i32).min(sy + sh) - iy;

            if iw <= 0 || ih <= 0 {
                continue;
            }

            let captured = screen
                .capture_area(ix, iy, iw as u32, ih as u32)
                .map_err(|e| {
                    CaptureError::AreaFailed(ix, iy, iw as u32, ih as u32, e.to_string())
                })?;

            let dx = (ix - rect.x) as u32;
            let dy = (iy - rect.y) as u32;
            let src_stride = iw as usize * 4;
            let src_raw = captured.as_raw();
            let dst_raw = result.as_mut();

            for cy in 0..(ih as u32) {
                let src_off = cy as usize * src_stride;
                let dst_off = (dy + cy) as usize * dst_stride + dx as usize * 4;
                dst_raw[dst_off..dst_off + src_stride]
                    .copy_from_slice(&src_raw[src_off..src_off + src_stride]);
            }
        }

        Ok(result)
    }
}

/// Global singleton for the default screen capture backend.
static DEFAULT_CAPTURE: OnceLock<ScreenshotsCapture> = OnceLock::new();

pub fn default_capture() -> &'static ScreenshotsCapture {
    DEFAULT_CAPTURE.get_or_init(|| ScreenshotsCapture)
}
