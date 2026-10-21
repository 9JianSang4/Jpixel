//! Full-monitor capture.
//!
//! `screenshots::Screen::capture_area()` takes coordinates local to the
//! display and applies `scale_factor` again. Because Jpixel works in physical
//! pixels (PerMonitorV2), we must use the unchecked Windows variant with
//! (0, 0, physical_width, physical_height).

use crate::core::monitor::MonitorInfo;
use anyhow::{anyhow, Result};
use image::RgbaImage;

pub trait CaptureService: Send + Sync {
    fn capture_monitor(&self, monitor: &MonitorInfo) -> Result<RgbaImage>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ScreenshotsCapture;

impl CaptureService for ScreenshotsCapture {
    fn capture_monitor(&self, monitor: &MonitorInfo) -> Result<RgbaImage> {
        capture_monitor(monitor)
    }
}

pub fn capture_monitor(monitor: &MonitorInfo) -> Result<RgbaImage> {
    let screen = screenshots::Screen::from_point(monitor.rect.left + 1, monitor.rect.top + 1)
        .map_err(|e| anyhow!("Screen::from_point failed: {e}"))?;

    #[cfg(target_os = "windows")]
    {
        let width = monitor.width() as u32;
        let height = monitor.height() as u32;
        let image = screen
            .capture_area_ignore_area_check(0, 0, width, height)
            .map_err(|e| anyhow!("capture_area_ignore_area_check failed: {e}"))?;
        ensure_size(image.width(), image.height(), width, height)?;
        Ok(image)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = screen;
        Err(anyhow!("native capture is only implemented on Windows"))
    }
}

fn ensure_size(actual_w: u32, actual_h: u32, expected_w: u32, expected_h: u32) -> Result<()> {
    if actual_w != expected_w || actual_h != expected_h {
        return Err(anyhow!(
            "capture size mismatch: got {actual_w}x{actual_h}, expected {expected_w}x{expected_h}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_mismatch_is_an_error() {
        assert!(ensure_size(1920, 1080, 1920, 1200).is_err());
    }

    #[test]
    fn matching_size_is_accepted() {
        assert!(ensure_size(1920, 1200, 1920, 1200).is_ok());
    }
}
