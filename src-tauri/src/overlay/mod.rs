//! Native overlay window: capture one monitor, show the frozen frame, let the
//! user drag a rectangle, copy it, and restore focus.

mod gdi;

use self::gdi::Dib;
use crate::core::monitor::{self, MonitorInfo};
use anyhow::{anyhow, Result};
use arboard::{Clipboard, ImageData};
use image::RgbaImage;
use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, UpdateWindow, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, ReleaseCapture, SetActiveWindow, SetCapture, SetFocus, UnregisterHotKey,
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, ClipCursor, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetForegroundWindow, GetMessageW, GetWindowRect, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW,
    CS_VREDRAW, HWND_TOPMOST, IDC_CROSS, MSG, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, WM_CAPTURECHANGED,
    WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const HOTKEY_ID: i32 = 0x4A50; // "JP"
const CLASS_NAME: PCWSTR = w!("JpixelNativeOverlay");
static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

#[derive(Debug, Clone)]
pub struct CaptureStats {
    pub capture_ms: f64,
    pub first_frame_ms: f64,
    pub monitor_label: String,
    pub monitor_rect: String,
    pub scale: f64,
}

#[derive(Debug, Clone)]
pub struct CopyStats {
    pub clipboard_ms: f64,
    pub pixel_match: bool,
    pub max_diff: u8,
    pub width: u32,
    pub height: u32,
}

struct OverlayState {
    hwnd: HWND,
    back: Option<Dib>,
    original: Option<Dib>,
    dimmed: Option<Dib>,
    image: Option<RgbaImage>,
    start: Option<POINT>,
    end: Option<POINT>,
    dragging: bool,
    visible: bool,
    prev_foreground: HWND,
    monitor_label: String,
    monitor_rect: String,
    scale: f64,
    mode: String,
    hotkey_vk: u32,
    hotkey_mods: u32,
    hotkey_name: String,
    capture_ms: f64,
    first_frame_ms: f64,
}

impl OverlayState {
    fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            back: None,
            original: None,
            dimmed: None,
            image: None,
            start: None,
            end: None,
            dragging: false,
            visible: false,
            prev_foreground: HWND(0),
            monitor_label: String::new(),
            monitor_rect: String::new(),
            scale: 1.0,
            mode: "manual".to_string(),
            hotkey_vk: 0,
            hotkey_mods: 0,
            hotkey_name: String::new(),
            capture_ms: 0.0,
            first_frame_ms: 0.0,
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
}

fn with_state<T>(f: impl FnOnce(&OverlayState) -> T) -> Result<T> {
    STATE.with(|cell| {
        let borrow = cell.borrow();
        let state = borrow
            .as_ref()
            .ok_or_else(|| anyhow!("overlay is not initialized"))?;
        Ok(f(state))
    })
}

fn with_state_mut<T>(f: impl FnOnce(&mut OverlayState) -> Result<T>) -> Result<T> {
    STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = borrow
            .as_mut()
            .ok_or_else(|| anyhow!("overlay is not initialized"))?;
        f(state)
    })
}

fn is_visible() -> bool {
    with_state(|s| s.visible).unwrap_or(false)
}

pub fn init() -> Result<()> {
    let module = unsafe { GetModuleHandleW(None) }?;
    let hinstance = HINSTANCE(module.0);

    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        hCursor: unsafe { LoadCursorW(None, IDC_CROSS)? },
        hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(0),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        return Err(anyhow!("RegisterClassW failed"));
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("Jpixel Overlay"),
            WS_POPUP,
            0,
            0,
            100,
            100,
            None,
            None,
            hinstance,
            None,
        )
    };
    if hwnd.0 == 0 {
        return Err(anyhow!("CreateWindowExW failed"));
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    STATE.with(|cell| {
        *cell.borrow_mut() = Some(OverlayState::new(hwnd));
    });
    OVERLAY_HWND.store(hwnd.0, Ordering::SeqCst);
    Ok(())
}

/// Start the native overlay on its own Win32 message-loop thread.
pub fn start(hotkey: &str) -> Result<()> {
    let parsed = crate::core::hotkey::parse(hotkey)
        .map_err(|e| anyhow!("invalid native hotkey '{hotkey}': {e}"))?;
    std::thread::Builder::new()
        .name("jpixel-overlay".to_string())
        .spawn(move || {
            if let Err(e) = run(parsed) {
                log::error!("native overlay thread stopped: {e}");
            }
        })
        .map_err(|e| anyhow!("cannot spawn overlay thread: {e}"))?;
    Ok(())
}

fn run(hotkey: crate::core::hotkey::Hotkey) -> Result<()> {
    enable_per_monitor_v2();
    init()?;
    let monitors = monitor::enumerate()?;
    warm_up(&monitors);
    register_hotkey(hotkey.vk, hotkey.mods, &hotkey.name)?;
    run_message_loop();
    Ok(())
}

fn enable_per_monitor_v2() {
    // A previous manifest may already have set awareness; failure is fine.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Trigger a capture from the Tauri thread (tray menu, single-instance command).
pub fn trigger_capture() {
    let raw = OVERLAY_HWND.load(Ordering::SeqCst);
    if raw == 0 {
        log::warn!("native overlay is not ready yet");
        return;
    }
    unsafe {
        let _ = PostMessageW(HWND(raw), WM_HOTKEY, WPARAM(HOTKEY_ID as usize), LPARAM(0));
    }
}

pub fn register_hotkey(vk: u32, mods: u32, name: &str) -> Result<()> {
    let hwnd = with_state(|state| state.hwnd)?;
    let modifiers = HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT;
    if let Err(e) = unsafe { RegisterHotKey(hwnd, HOTKEY_ID, modifiers, vk) } {
        log::error!("cannot register hotkey {name}: {e}; another app may already own it");
        return Err(anyhow!(
            "RegisterHotKey({name}) failed; another app may already own it"
        ));
    }
    let _ = with_state_mut(|state| {
        state.hotkey_vk = vk;
        state.hotkey_mods = mods;
        state.hotkey_name = name.to_string();
        Ok(())
    });
    log::info!(
        "registered native hotkey: {name} (overlay hwnd=0x{:X})",
        hwnd.0
    );
    Ok(())
}

fn reregister_hotkey() {
    let (hwnd, vk, mods, name) = with_state(|state| {
        (
            state.hwnd,
            state.hotkey_vk,
            state.hotkey_mods,
            state.hotkey_name.clone(),
        )
    })
    .unwrap_or((HWND(0), 0, 0, String::new()));
    if vk == 0 || hwnd.0 == 0 {
        return;
    }
    let modifiers = HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT;
    unsafe {
        let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
    }
    match unsafe { RegisterHotKey(hwnd, HOTKEY_ID, modifiers, vk) } {
        Ok(()) => log::debug!("re-registered native hotkey: {name}"),
        Err(e) => log::warn!("cannot re-register native hotkey {name}: {e}"),
    }
}

pub fn run_message_loop() {
    let mut msg = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn force_foreground(hwnd: HWND) {
    unsafe {
        let _ = BringWindowToTop(hwnd);
        let set_foreground = SetForegroundWindow(hwnd).as_bool();
        let _ = SetActiveWindow(hwnd);
        let set_focus = SetFocus(hwnd).0 != 0;
        let foreground = GetForegroundWindow();
        log::debug!(
            "native overlay focus: set_foreground={} set_focus={} foreground={} target={}",
            set_foreground,
            set_focus,
            foreground.0,
            hwnd.0
        );
    }
}

fn capture_screen(monitor: &MonitorInfo) -> Result<RgbaImage> {
    crate::core::capture::capture_monitor(monitor)
}

/// Initialize DXGI/COM for each monitor once so the first real hotkey is warm.
pub fn warm_up(monitors: &[MonitorInfo]) {
    let started = Instant::now();
    for monitor in monitors {
        let _ = capture_screen(monitor);
    }
    log::info!(
        "native capture warm-up: {:.1}ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

fn choose_prev_foreground(foreground: HWND, overlay: HWND, stored: HWND) -> HWND {
    if foreground.0 == 0 || foreground == overlay {
        if stored.0 != 0 {
            stored
        } else {
            foreground
        }
    } else {
        foreground
    }
}

pub fn capture_monitor(monitor: &MonitorInfo, mode: &str) -> Result<CaptureStats> {
    if is_visible() {
        return Err(anyhow!("overlay is already visible"));
    }
    let t0 = Instant::now();
    let image = capture_screen(monitor)?;
    let capture_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let original = Dib::from_rgba(&image)?;
    let dimmed = Dib::dimmed_from_rgba(&image)?;
    let back = Dib::blank(monitor.width(), monitor.height())?;
    let foreground = unsafe { GetForegroundWindow() };

    let hwnd = with_state_mut(|state| {
        // On a re-capture the overlay itself may still be the foreground
        // window; keep the original app so closing the new overlay restores
        // focus to the app the user was using, not to a hidden overlay.
        let prev = choose_prev_foreground(foreground, state.hwnd, state.prev_foreground);
        state.back = Some(back);
        state.original = Some(original);
        state.dimmed = Some(dimmed);
        state.image = Some(image);
        state.start = None;
        state.end = None;
        state.dragging = false;
        state.visible = true;
        state.prev_foreground = prev;
        state.monitor_label = monitor.describe();
        state.monitor_rect = format!(
            "({},{},{},{})",
            monitor.rect.left, monitor.rect.top, monitor.rect.right, monitor.rect.bottom
        );
        state.scale = monitor.scale();
        state.mode = mode.to_string();
        state.capture_ms = capture_ms;
        Ok(state.hwnd)
    })?;

    unsafe {
        let shown = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            monitor.rect.left,
            monitor.rect.top,
            monitor.width(),
            monitor.height(),
            SWP_SHOWWINDOW,
        );
        if shown.is_err() {
            return Err(anyhow!("SetWindowPos failed"));
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        force_foreground(hwnd);
    }
    gdi::invalidate(hwnd);
    unsafe {
        let _ = UpdateWindow(hwnd);
        let _ = DwmFlush();
    }
    let first_frame_ms = t0.elapsed().as_secs_f64() * 1000.0;

    with_state_mut(|state| {
        state.first_frame_ms = first_frame_ms;
        Ok(())
    })?;

    log::debug!(
        "native capture {} capture={:.1}ms first_frame={:.1}ms",
        monitor.describe(),
        capture_ms,
        first_frame_ms
    );

    Ok(CaptureStats {
        capture_ms,
        first_frame_ms,
        monitor_label: monitor.describe(),
        monitor_rect: format!(
            "({},{},{},{})",
            monitor.rect.left, monitor.rect.top, monitor.rect.right, monitor.rect.bottom
        ),
        scale: monitor.scale(),
    })
}

fn selection_rect(state: &OverlayState) -> Option<RECT> {
    let (start, end) = (state.start?, state.end?);
    let (image_w, image_h) = state.image.as_ref()?.dimensions();
    let left = start.x.min(end.x).max(0).min(image_w as i32 - 1);
    let top = start.y.min(end.y).max(0).min(image_h as i32 - 1);
    let right = start.x.max(end.x).max(0).min(image_w as i32);
    let bottom = start.y.max(end.y).max(0).min(image_h as i32);
    if right > left && bottom > top {
        Some(RECT {
            left,
            top,
            right,
            bottom,
        })
    } else {
        None
    }
}

pub fn set_bench_selection() -> Result<()> {
    let hwnd = with_state_mut(|state| {
        let (w, h) = state
            .image
            .as_ref()
            .map(|image| image.dimensions())
            .ok_or_else(|| anyhow!("no capture available"))?;
        let sel_w = 800.min(w.saturating_sub(40)).max(2) as i32;
        let sel_h = 600.min(h.saturating_sub(40)).max(2) as i32;
        let left = (w as i32 - sel_w) / 2;
        let top = (h as i32 - sel_h) / 2;
        state.start = Some(POINT { x: left, y: top });
        state.end = Some(POINT {
            x: left + sel_w,
            y: top + sel_h,
        });
        Ok(state.hwnd)
    })?;
    gdi::invalidate(hwnd);
    unsafe {
        let _ = UpdateWindow(hwnd);
    }
    Ok(())
}

pub fn copy_current_selection() -> Result<CopyStats> {
    let (image, rect) = STATE.with(|cell| {
        let borrow = cell.borrow();
        let state = borrow
            .as_ref()
            .ok_or_else(|| anyhow!("overlay is not initialized"))?;
        let image = state
            .image
            .as_ref()
            .ok_or_else(|| anyhow!("no capture available"))?;
        let rect = selection_rect(state).unwrap_or(RECT {
            left: 0,
            top: 0,
            right: image.width() as i32,
            bottom: image.height() as i32,
        });
        Ok::<(RgbaImage, RECT), anyhow::Error>((image.clone(), rect))
    })?;

    let x = rect.left.max(0) as u32;
    let y = rect.top.max(0) as u32;
    let w = (rect.right - rect.left).max(1) as u32;
    let h = (rect.bottom - rect.top).max(1) as u32;
    let crop = image::imageops::crop_imm(&image, x, y, w, h).to_image();

    let data = ImageData {
        width: w as usize,
        height: h as usize,
        bytes: Cow::Owned(crop.as_raw().clone()),
    };

    let t0 = Instant::now();
    let mut clipboard = Clipboard::new().map_err(|e| anyhow!("Clipboard::new failed: {e}"))?;
    clipboard
        .set_image(data)
        .map_err(|e| anyhow!("set_image failed: {e}"))?;
    let clipboard_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let returned = clipboard.get_image();
    let (pixel_match, max_diff) = match returned {
        Ok(returned) => {
            if returned.width as u32 != w || returned.height as u32 != h {
                (false, 255)
            } else {
                let mut max_diff = 0u8;
                for (a, b) in crop.as_raw().iter().zip(returned.bytes.iter()) {
                    max_diff = max_diff.max(a.abs_diff(*b));
                }
                (max_diff <= 1, max_diff)
            }
        }
        Err(_) => (false, 255),
    };

    Ok(CopyStats {
        clipboard_ms,
        pixel_match,
        max_diff,
        width: w,
        height: h,
    })
}

fn report_current(note: &str, copy: Option<&CopyStats>) {
    let values = with_state(|state| {
        (
            state.mode.clone(),
            state.monitor_label.clone(),
            state.monitor_rect.clone(),
            state.scale,
            state.capture_ms,
            state.first_frame_ms,
        )
    });
    let Ok((mode, monitor_label, monitor_rect, scale, capture_ms, first_frame_ms)) = values else {
        return;
    };
    log::debug!(
        "overlay {note}: mode={mode} monitor={monitor_label} rect={monitor_rect} scale={scale:.3} capture={capture_ms:.1}ms first_frame={first_frame_ms:.1}ms"
    );
    if let Some(copy) = copy {
        log::debug!(
            "overlay copy: {:.1}ms pixel_match={} max_diff={}",
            copy.clipboard_ms,
            copy.pixel_match,
            copy.max_diff
        );
    }
}

/// Hide the current overlay without restoring focus. Used when the hotkey is
/// pressed again to re-capture another monitor; `capture_monitor` reuses the
/// original `prev_foreground` so the final close still restores the right app.
fn hide_overlay_for_recapture() {
    release_cursor_clip();
    let hwnd = with_state_mut(|state| {
        let hwnd = state.hwnd;
        state.visible = false;
        state.dragging = false;
        state.start = None;
        state.end = None;
        state.back = None;
        state.original = None;
        state.dimmed = None;
        state.image = None;
        Ok(hwnd)
    })
    .unwrap_or(HWND(0));

    unsafe {
        if hwnd.0 != 0 {
            let _ = ReleaseCapture();
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

pub fn hide_overlay() {
    release_cursor_clip();
    let (hwnd, prev) = with_state_mut(|state| {
        let hwnd = state.hwnd;
        let prev = state.prev_foreground;
        state.visible = false;
        state.dragging = false;
        state.start = None;
        state.end = None;
        state.back = None;
        state.original = None;
        state.dimmed = None;
        state.image = None;
        Ok((hwnd, prev))
    })
    .unwrap_or((HWND(0), HWND(0)));

    unsafe {
        if hwnd.0 != 0 {
            let _ = ReleaseCapture();
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        if prev.0 != 0 && prev != hwnd {
            let _ = SetForegroundWindow(prev);
        }
    }

    reregister_hotkey();
}

fn copy_and_close() {
    match copy_current_selection() {
        Ok(copy) => {
            report_current("copy", Some(&copy));
            log::info!(
                "copied {}x{} clipboard={:.1}ms pixel_match={} max_diff={}",
                copy.width,
                copy.height,
                copy.clipboard_ms,
                copy.pixel_match,
                copy.max_diff
            );
        }
        Err(e) => {
            report_current(&format!("copy-error: {e}"), None);
            log::error!("native copy failed: {e}");
        }
    }
    hide_overlay();
}

fn cancel_and_close() {
    report_current("cancel", None);
    hide_overlay();
}

fn handle_hotkey() {
    let recapture = is_visible();
    if recapture {
        log::debug!("native hotkey received; recapturing on the current target monitor");
    } else {
        log::debug!("native hotkey received; resolving target monitor");
    }
    let result = monitor::enumerate()
        .and_then(|monitors| monitor::target_monitor(&monitors))
        .and_then(|target| {
            if recapture {
                hide_overlay_for_recapture();
            }
            capture_monitor(&target, "manual")
        });
    match result {
        Ok(_) => report_current(if recapture { "recapture" } else { "capture" }, None),
        Err(e) => log::error!("native capture failed: {e}"),
    }
}

fn paint(hwnd: HWND) {
    STATE.with(|cell| {
        let borrow = cell.borrow();
        if let Some(state) = borrow.as_ref() {
            if let (Some(back), Some(dimmed), Some(original)) =
                (&state.back, &state.dimmed, &state.original)
            {
                if let Err(e) =
                    gdi::paint_overlay(hwnd, back, dimmed, original, selection_rect(state))
                {
                    log::error!("native overlay paint failed: {e}");
                }
                return;
            }
        }
        // No capture yet: still consume the paint message so the invalid region
        // does not spin.
        let mut ps = PAINTSTRUCT::default();
        unsafe {
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
        }
    });
}

fn clamp_to_image(state: &OverlayState, point: POINT) -> POINT {
    let Some(image) = state.image.as_ref() else {
        return point;
    };
    let max_x = image.width().saturating_sub(1) as i32;
    let max_y = image.height().saturating_sub(1) as i32;
    POINT {
        x: point.x.clamp(0, max_x.max(0)),
        y: point.y.clamp(0, max_y.max(0)),
    }
}

fn clip_cursor_to_window(hwnd: HWND) {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
        unsafe {
            let _ = ClipCursor(Some(&rect as *const RECT));
        }
    }
}

fn release_cursor_clip() {
    unsafe {
        let _ = ClipCursor(None);
    }
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xffff) as i32,
        y: ((lparam.0 >> 16) & 0xffff) as i32,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            let pt = point_from_lparam(lparam);
            let _ = SetCapture(hwnd);
            clip_cursor_to_window(hwnd);
            with_state_mut(|state| {
                if !state.visible {
                    return Ok(());
                }
                let clamped = clamp_to_image(state, pt);
                state.dragging = true;
                state.start = Some(clamped);
                state.end = Some(clamped);
                Ok(())
            })
            .ok();
            gdi::invalidate(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let pt = point_from_lparam(lparam);
            let changed = with_state_mut(|state| {
                if state.dragging {
                    state.end = Some(clamp_to_image(state, pt));
                    Ok(true)
                } else {
                    Ok(false)
                }
            })
            .unwrap_or(false);
            if changed {
                gdi::invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            release_cursor_clip();
            with_state_mut(|state| {
                state.dragging = false;
                Ok(())
            })
            .ok();
            gdi::invalidate(hwnd);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            release_cursor_clip();
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == VK_ESCAPE.0 as usize {
                cancel_and_close();
            } else if wparam.0 == VK_RETURN.0 as usize {
                copy_and_close();
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            if wparam.0 == HOTKEY_ID as usize {
                handle_hotkey();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            release_cursor_clip();
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_capture_uses_the_foreground_app() {
        assert_eq!(
            choose_prev_foreground(HWND(0x10), HWND(0x99), HWND(0)).0,
            0x10
        );
    }

    #[test]
    fn recapture_with_overlay_foreground_keeps_the_original_app() {
        assert_eq!(
            choose_prev_foreground(HWND(0x99), HWND(0x99), HWND(0x10)).0,
            0x10
        );
    }

    #[test]
    fn recapture_with_another_app_foreground_uses_that_app() {
        assert_eq!(
            choose_prev_foreground(HWND(0x20), HWND(0x99), HWND(0x10)).0,
            0x20
        );
    }

    #[test]
    fn zero_foreground_falls_back_to_the_stored_app() {
        assert_eq!(
            choose_prev_foreground(HWND(0), HWND(0x99), HWND(0x10)).0,
            0x10
        );
    }
}
