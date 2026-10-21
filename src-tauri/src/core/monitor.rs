//! Monitor enumeration and "which monitor owns the focused window" selection.
//!
//! All rectangles are physical pixels because the spike opts into PerMonitorV2
//! DPI awareness before creating any window.

use anyhow::{anyhow, Context, Result};
use std::mem::size_of;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HDC, HMONITOR,
    MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetDesktopWindow, GetForegroundWindow, GetWindowThreadProcessId,
    MONITORINFOF_PRIMARY,
};

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub hmonitor: HMONITOR,
    pub rect: RECT,
    #[allow(dead_code)]
    pub work: RECT,
    pub primary: bool,
    pub dpi_x: u32,
    #[allow(dead_code)]
    pub dpi_y: u32,
}

impl MonitorInfo {
    pub fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }

    pub fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }

    pub fn scale(&self) -> f64 {
        self.dpi_x as f64 / 96.0
    }

    pub fn label(&self, index: usize) -> String {
        format!("{}: {}", index + 1, self.describe())
    }

    pub fn describe(&self) -> String {
        format!(
            "{}x{} @{:.0}% rect=({},{},{},{}){}",
            self.width(),
            self.height(),
            self.scale() * 100.0,
            self.rect.left,
            self.rect.top,
            self.rect.right,
            self.rect.bottom,
            if self.primary { " primary" } else { "" }
        )
    }
}

unsafe extern "system" fn enum_monitor(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<MonitorInfo>);
    if let Some(info) = monitor_info(hmonitor) {
        out.push(info);
    }
    BOOL(1)
}

fn monitor_info(hmonitor: HMONITOR) -> Option<MonitorInfo> {
    let mut mi = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(hmonitor, &mut mi) }.as_bool() {
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
        Some(MonitorInfo {
            hmonitor,
            rect: mi.rcMonitor,
            work: mi.rcWork,
            primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0,
            dpi_x,
            dpi_y,
        })
    } else {
        None
    }
}

pub fn enumerate() -> Result<Vec<MonitorInfo>> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor),
            LPARAM(&mut monitors as *mut Vec<MonitorInfo> as isize),
        )
    };
    if !ok.as_bool() {
        return Err(anyhow!("EnumDisplayMonitors failed"));
    }
    if monitors.is_empty() {
        return Err(anyhow!("no monitors found"));
    }
    Ok(monitors)
}

fn monitor_by_handle(monitors: &[MonitorInfo], hmonitor: HMONITOR) -> Option<MonitorInfo> {
    monitors.iter().find(|m| m.hmonitor == hmonitor).cloned()
}

fn cursor_monitor(monitors: &[MonitorInfo]) -> Result<MonitorInfo> {
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt) }.context("GetCursorPos failed")?;
    let hmonitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    monitor_by_handle(monitors, hmonitor)
        .ok_or_else(|| anyhow!("could not resolve monitor under cursor"))
}

fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

fn is_shell_window(hwnd: HWND) -> bool {
    if hwnd.0 == 0 || hwnd == unsafe { GetDesktopWindow() } {
        return true;
    }
    matches!(
        window_class(hwnd).as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

fn is_own_window(hwnd: HWND) -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid == unsafe { GetCurrentProcessId() }
}

/// Resolve the monitor that owns the currently focused window.
///
/// Fallback order:
/// 1. foreground window's monitor (the product rule);
/// 2. if the foreground window is the desktop, taskbar, or our own process,
///    use the monitor under the mouse cursor;
/// 3. if that cannot be resolved, use the primary monitor.
pub fn target_monitor(monitors: &[MonitorInfo]) -> Result<MonitorInfo> {
    let hwnd = unsafe { GetForegroundWindow() };
    if !is_shell_window(hwnd) && !is_own_window(hwnd) {
        let hmonitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        if let Some(info) = monitor_by_handle(monitors, hmonitor) {
            return Ok(info);
        }
    }
    if let Ok(info) = cursor_monitor(monitors) {
        return Ok(info);
    }
    monitors
        .iter()
        .find(|m| m.primary)
        .cloned()
        .or_else(|| monitors.first().cloned())
        .ok_or_else(|| anyhow!("no usable monitor"))
}

pub fn by_index(monitors: &[MonitorInfo], index_1_based: usize) -> Result<MonitorInfo> {
    monitors
        .get(index_1_based.saturating_sub(1))
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "monitor {} is out of range (1..={})",
                index_1_based,
                monitors.len()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn fake(hmonitor: isize, r: RECT, primary: bool) -> MonitorInfo {
        MonitorInfo {
            hmonitor: HMONITOR(hmonitor),
            rect: r,
            work: r,
            primary,
            dpi_x: 96,
            dpi_y: 96,
        }
    }

    #[test]
    fn width_and_height_are_positive() {
        let m = fake(1, rect(-1920, 0, 0, 1080), false);
        assert_eq!(m.width(), 1920);
        assert_eq!(m.height(), 1080);
    }

    #[test]
    fn scale_is_dpi_over_96() {
        let mut m = fake(1, rect(0, 0, 1920, 1200), true);
        m.dpi_x = 120;
        assert!((m.scale() - 1.25).abs() < f64::EPSILON);
    }

    #[test]
    fn by_index_is_one_based() {
        let monitors = vec![
            fake(1, rect(0, 0, 1920, 1200), true),
            fake(2, rect(1920, 0, 3840, 1080), false),
        ];
        assert_eq!(by_index(&monitors, 1).unwrap().rect.left, 0);
        assert_eq!(by_index(&monitors, 2).unwrap().rect.left, 1920);
        assert!(by_index(&monitors, 3).is_err());
    }
}
