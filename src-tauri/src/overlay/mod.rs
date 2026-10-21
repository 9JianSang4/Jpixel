//! Native overlay window: capture one monitor, show the frozen frame, let the
//! user drag a rectangle, copy it, and restore focus.

mod gdi;

use self::gdi::Dib;
use crate::config::SelectionMode;
use crate::core::monitor::{self, MonitorInfo};
use anyhow::{anyhow, Result};
use arboard::{Clipboard, ImageData};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ColorType, ImageEncoder, RgbaImage};
use std::borrow::Cow;
use std::cell::RefCell;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    GetStockObject, RoundRect, ScreenToClient, SelectObject, SetBkMode, SetTextColor, UpdateWindow,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HDC, HGDIOBJ, HFONT, NULL_PEN, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, TRANSPARENT,
};
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
    GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowRect, LoadCursorW, MessageBoxW,
    PostMessageW, PostQuitMessage, RegisterClassW, SetCursor, SetForegroundWindow, SetWindowPos,
    ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, IDC_ARROW, IDC_CROSS,
    IDC_HAND, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, IDC_SIZEALL, MB_ICONERROR, MB_OK,
    MSG, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, WM_CAPTURECHANGED, WM_DESTROY,
    WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_SETCURSOR, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const HOTKEY_ID: i32 = 0x4A50; // "JP"
const WM_APP_SET_HOTKEY: u32 = 0x8001;
const WM_APP_SET_OPTIONS: u32 = 0x8002;
const CLASS_NAME: PCWSTR = w!("JpixelNativeOverlay");
static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

#[derive(Debug, Clone, Copy)]
pub struct OverlayOptions {
    pub crosshair_guides: bool,
    pub selection_mode: SelectionMode,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            crosshair_guides: true,
            selection_mode: SelectionMode::Reselect,
        }
    }
}

static OPTIONS: Mutex<OverlayOptions> = Mutex::new(OverlayOptions {
    crosshair_guides: true,
    selection_mode: SelectionMode::Reselect,
});

fn current_options() -> OverlayOptions {
    OPTIONS
        .lock()
        .map(|options| *options)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeHandle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

#[derive(Debug, Clone, Copy)]
struct ResizeDrag {
    handle: ResizeHandle,
    start_rect: RECT,
}

#[derive(Debug, Clone, Copy)]
struct MoveDrag {
    start_rect: RECT,
    start_point: POINT,
}

struct HotkeyRequest {
    vk: u32,
    mods: u32,
    name: String,
    reply: mpsc::Sender<Result<(), String>>,
}

static PENDING_HOTKEY: Mutex<Option<HotkeyRequest>> = Mutex::new(None);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarButton {
    Copy,
    Save,
    Pin,
    Pen,
    Eraser,
    Undo,
    Redo,
    Close,
}

const TOOLBAR_ITEMS: [(ToolbarButton, &str); 8] = [
    (ToolbarButton::Copy, "复制"),
    (ToolbarButton::Save, "保存"),
    (ToolbarButton::Pin, "贴图"),
    (ToolbarButton::Pen, "画笔"),
    (ToolbarButton::Eraser, "橡皮"),
    (ToolbarButton::Undo, "撤销"),
    (ToolbarButton::Redo, "重做"),
    (ToolbarButton::Close, "关闭"),
];

const TOOLBAR_BUTTON_W: i32 = 44;
const TOOLBAR_BUTTON_H: i32 = 34;
const TOOLBAR_PAD: i32 = 6;
const TOOLBAR_GAP: i32 = 2;
const TOOLBAR_OFFSET: i32 = 8;
const TOOLBAR_W: i32 = 8 * TOOLBAR_BUTTON_W + 7 * TOOLBAR_GAP + 2 * TOOLBAR_PAD;
const TOOLBAR_H: i32 = TOOLBAR_BUTTON_H + 2 * TOOLBAR_PAD;

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
    hover_button: Option<ToolbarButton>,
    cursor: Option<POINT>,
    resize_drag: Option<ResizeDrag>,
    move_drag: Option<MoveDrag>,
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
            hover_button: None,
            cursor: None,
            resize_drag: None,
            move_drag: None,
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

/// Update live overlay options from the settings thread.
pub fn set_options(crosshair_guides: bool, selection_mode: SelectionMode) {
    if let Ok(mut options) = OPTIONS.lock() {
        *options = OverlayOptions {
            crosshair_guides,
            selection_mode,
        };
    }
    let raw = OVERLAY_HWND.load(Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(HWND(raw), WM_APP_SET_OPTIONS, WPARAM(0), LPARAM(0));
        }
    }
}

/// Start the native overlay on its own Win32 message-loop thread.
pub fn start(hotkey: &str, options: OverlayOptions) -> Result<()> {
    if let Ok(mut current) = OPTIONS.lock() {
        *current = options;
    }
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

/// Change the native hotkey from the Tauri/settings thread.
///
/// RegisterHotKey must run on the thread that owns the overlay window, so the
/// request is posted to the overlay message loop and the result is returned
/// through a channel. The old hotkey is restored if the new one is taken.
pub fn change_hotkey(vk: u32, mods: u32, name: &str) -> Result<()> {
    // STATE is thread-local to the overlay thread. This function runs on the
    // Tauri/settings thread, so use the global HWND published by init().
    let raw = OVERLAY_HWND.load(Ordering::SeqCst);
    if raw == 0 {
        return Err(anyhow!("overlay is not initialized"));
    }
    let hwnd = HWND(raw);
    let (reply, result) = mpsc::channel();
    {
        let mut pending = PENDING_HOTKEY
            .lock()
            .map_err(|_| anyhow!("hotkey request lock poisoned"))?;
        if pending.is_some() {
            return Err(anyhow!("another hotkey change is already pending"));
        }
        *pending = Some(HotkeyRequest {
            vk,
            mods,
            name: name.to_string(),
            reply,
        });
    }

    unsafe {
        PostMessageW(hwnd, WM_APP_SET_HOTKEY, WPARAM(0), LPARAM(0))
            .map_err(|e| anyhow!("cannot post hotkey change: {e}"))?;
    }

    result
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| anyhow!("hotkey change timed out"))?
        .map_err(|e| anyhow!(e))
}

fn handle_pending_hotkey() {
    let request = PENDING_HOTKEY
        .lock()
        .ok()
        .and_then(|mut pending| pending.take());
    if let Some(request) = request {
        let result = apply_hotkey_change(request.vk, request.mods, &request.name);
        let _ = request.reply.send(result);
    }
}

fn apply_hotkey_change(vk: u32, mods: u32, name: &str) -> std::result::Result<(), String> {
    let (hwnd, old_vk, old_mods) =
        with_state(|state| (state.hwnd, state.hotkey_vk, state.hotkey_mods))
            .map_err(|e| e.to_string())?;
    if hwnd.0 == 0 {
        return Err("native overlay window is not ready".to_string());
    }

    unsafe {
        let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
    }
    let modifiers = HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT;
    match unsafe { RegisterHotKey(hwnd, HOTKEY_ID, modifiers, vk) } {
        Ok(()) => {
            let _ = with_state_mut(|state| {
                state.hotkey_vk = vk;
                state.hotkey_mods = mods;
                state.hotkey_name = name.to_string();
                Ok(())
            });
            log::info!("changed native hotkey to {name}");
            Ok(())
        }
        Err(e) => {
            let old_modifiers = HOT_KEY_MODIFIERS(old_mods) | MOD_NOREPEAT;
            let restored = unsafe { RegisterHotKey(hwnd, HOTKEY_ID, old_modifiers, old_vk) };
            if let Err(restore_err) = restored {
                log::error!("failed to restore previous hotkey: {restore_err}");
            }
            Err(format!("RegisterHotKey({name}) failed: {e}"))
        }
    }
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
    let initial_cursor = {
        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) }.is_ok() {
            cursor.x -= monitor.rect.left;
            cursor.y -= monitor.rect.top;
            Some(cursor)
        } else {
            None
        }
    };

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
        state.hover_button = None;
        state.cursor = initial_cursor;
        state.resize_drag = None;
        state.move_drag = None;
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

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

fn toolbar_rect(state: &OverlayState) -> Option<RECT> {
    if state.dragging {
        return None;
    }
    let image = state.image.as_ref()?;
    let selection = selection_rect(state)?;
    let screen_w = image.width() as i32;
    let screen_h = image.height() as i32;
    let mut left = selection.right - TOOLBAR_W;
    left = left.clamp(4, (screen_w - TOOLBAR_W - 4).max(4));
    let mut top = selection.bottom + TOOLBAR_OFFSET;
    if top + TOOLBAR_H > screen_h - 4 {
        top = selection.top - TOOLBAR_H - TOOLBAR_OFFSET;
    }
    top = top.clamp(4, (screen_h - TOOLBAR_H - 4).max(4));
    Some(RECT {
        left,
        top,
        right: left + TOOLBAR_W,
        bottom: top + TOOLBAR_H,
    })
}

fn toolbar_button_rects(state: &OverlayState) -> Vec<(ToolbarButton, RECT)> {
    let Some(toolbar) = toolbar_rect(state) else {
        return Vec::new();
    };
    TOOLBAR_ITEMS
        .iter()
        .enumerate()
        .map(|(index, (button, _))| {
            let left = toolbar.left + TOOLBAR_PAD + index as i32 * (TOOLBAR_BUTTON_W + TOOLBAR_GAP);
            let rect = RECT {
                left,
                top: toolbar.top + TOOLBAR_PAD,
                right: left + TOOLBAR_BUTTON_W,
                bottom: toolbar.top + TOOLBAR_PAD + TOOLBAR_BUTTON_H,
            };
            (*button, rect)
        })
        .collect()
}

fn toolbar_button_at(state: &OverlayState, point: POINT) -> Option<ToolbarButton> {
    toolbar_button_rects(state)
        .into_iter()
        .find(|(_, rect)| {
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        })
        .map(|(button, _)| button)
}

fn toolbar_button_enabled(_state: &OverlayState, button: ToolbarButton) -> bool {
    match button {
        ToolbarButton::Copy | ToolbarButton::Save | ToolbarButton::Close => true,
        // P2.2b enables these once annotations are wired.
        ToolbarButton::Pin
        | ToolbarButton::Pen
        | ToolbarButton::Eraser
        | ToolbarButton::Undo
        | ToolbarButton::Redo => false,
    }
}

fn draw_toolbar(hdc: HDC, state: &OverlayState) -> Result<()> {
    let Some(toolbar) = toolbar_rect(state) else {
        return Ok(());
    };
    unsafe {
        let background = CreateSolidBrush(rgb(28, 28, 30));
        let old_brush = SelectObject(hdc, HGDIOBJ(background.0));
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = RoundRect(
            hdc,
            toolbar.left,
            toolbar.top,
            toolbar.right,
            toolbar.bottom,
            12,
            12,
        );
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(background.0));

        let font: HFONT = CreateFontW(
            -12,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft YaHei UI"),
        );
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        let _ = SetBkMode(hdc, TRANSPARENT);

        for (button, rect) in toolbar_button_rects(state) {
            let enabled = toolbar_button_enabled(state, button);
            if enabled && state.hover_button == Some(button) {
                let hover = CreateSolidBrush(rgb(58, 58, 62));
                let old = SelectObject(hdc, HGDIOBJ(hover.0));
                let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
                let _ = RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, 8, 8);
                SelectObject(hdc, old_pen);
                SelectObject(hdc, old);
                let _ = DeleteObject(HGDIOBJ(hover.0));
            }
            let label = TOOLBAR_ITEMS
                .iter()
                .find(|(item, _)| *item == button)
                .map(|(_, label)| *label)
                .unwrap_or("");
            let color = if enabled {
                rgb(245, 245, 245)
            } else {
                rgb(125, 125, 130)
            };
            let _ = SetTextColor(hdc, color);
            let mut text: Vec<u16> = label.encode_utf16().collect();
            let mut text_rect = rect;
            let _ = DrawTextW(
                hdc,
                &mut text,
                &mut text_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }

        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
    Ok(())
}

fn handle_toolbar_click(button: ToolbarButton) {
    match button {
        ToolbarButton::Copy => copy_and_close(),
        ToolbarButton::Save => save_and_close(),
        ToolbarButton::Close => cancel_and_close(),
        other => log::debug!("toolbar button {:?} is not implemented yet", other),
    }
}

const HANDLE_SIZE: i32 = 7;

fn selection_handle_rects(state: &OverlayState) -> Vec<(ResizeHandle, RECT)> {
    if current_options().selection_mode != SelectionMode::Resize {
        return Vec::new();
    }
    let Some(selection) = selection_rect(state) else {
        return Vec::new();
    };
    let cx = (selection.left + selection.right) / 2;
    let cy = (selection.top + selection.bottom) / 2;
    let half = HANDLE_SIZE / 2;
    let positions = [
        (ResizeHandle::NorthWest, selection.left, selection.top),
        (ResizeHandle::North, cx, selection.top),
        (ResizeHandle::NorthEast, selection.right, selection.top),
        (ResizeHandle::East, selection.right, cy),
        (ResizeHandle::SouthEast, selection.right, selection.bottom),
        (ResizeHandle::South, cx, selection.bottom),
        (ResizeHandle::SouthWest, selection.left, selection.bottom),
        (ResizeHandle::West, selection.left, cy),
    ];
    positions
        .into_iter()
        .map(|(handle, x, y)| {
            (
                handle,
                RECT {
                    left: x - half,
                    top: y - half,
                    right: x + half + 1,
                    bottom: y + half + 1,
                },
            )
        })
        .collect()
}

fn selection_handle_at(state: &OverlayState, point: POINT) -> Option<ResizeHandle> {
    selection_handle_rects(state)
        .into_iter()
        .find(|(_, rect)| {
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        })
        .map(|(handle, _)| handle)
}

fn handle_cursor(handle: ResizeHandle) -> PCWSTR {
    match handle {
        ResizeHandle::NorthWest | ResizeHandle::SouthEast => IDC_SIZENWSE,
        ResizeHandle::NorthEast | ResizeHandle::SouthWest => IDC_SIZENESW,
        ResizeHandle::North | ResizeHandle::South => IDC_SIZENS,
        ResizeHandle::East | ResizeHandle::West => IDC_SIZEWE,
    }
}

fn image_bounds(state: &OverlayState) -> Option<RECT> {
    let image = state.image.as_ref()?;
    Some(RECT {
        left: 0,
        top: 0,
        right: image.width() as i32,
        bottom: image.height() as i32,
    })
}

fn set_selection(state: &mut OverlayState, rect: RECT) {
    state.start = Some(POINT {
        x: rect.left,
        y: rect.top,
    });
    state.end = Some(POINT {
        x: rect.right,
        y: rect.bottom,
    });
}

fn resize_selection(rect: RECT, handle: ResizeHandle, point: POINT, bounds: RECT) -> RECT {
    let mut next = rect;
    match handle {
        ResizeHandle::NorthWest => {
            next.left = point.x;
            next.top = point.y;
        }
        ResizeHandle::North => next.top = point.y,
        ResizeHandle::NorthEast => {
            next.right = point.x;
            next.top = point.y;
        }
        ResizeHandle::East => next.right = point.x,
        ResizeHandle::SouthEast => {
            next.right = point.x;
            next.bottom = point.y;
        }
        ResizeHandle::South => next.bottom = point.y,
        ResizeHandle::SouthWest => {
            next.left = point.x;
            next.bottom = point.y;
        }
        ResizeHandle::West => next.left = point.x,
    }
    next.left = next.left.clamp(bounds.left, bounds.right);
    next.top = next.top.clamp(bounds.top, bounds.bottom);
    next.right = next.right.clamp(bounds.left, bounds.right);
    next.bottom = next.bottom.clamp(bounds.top, bounds.bottom);

    if next.right - next.left < 3 {
        match handle {
            ResizeHandle::West | ResizeHandle::NorthWest | ResizeHandle::SouthWest => {
                next.left = (next.right - 3).max(bounds.left);
            }
            _ => next.right = (next.left + 3).min(bounds.right),
        }
    }
    if next.bottom - next.top < 3 {
        match handle {
            ResizeHandle::North | ResizeHandle::NorthWest | ResizeHandle::NorthEast => {
                next.top = (next.bottom - 3).max(bounds.top);
            }
            _ => next.bottom = (next.top + 3).min(bounds.bottom),
        }
    }
    next
}

fn move_selection(rect: RECT, delta: POINT, bounds: RECT) -> RECT {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let left = (rect.left + delta.x).clamp(bounds.left, (bounds.right - width).max(bounds.left));
    let top = (rect.top + delta.y).clamp(bounds.top, (bounds.bottom - height).max(bounds.top));
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn draw_crosshair(back: &Dib, state: &OverlayState) -> Result<()> {
    if !current_options().crosshair_guides {
        return Ok(());
    }
    let (Some(point), Some(bounds)) = (state.cursor, image_bounds(state)) else {
        return Ok(());
    };
    // Soft, low-contrast guides so they help alignment without dominating the
    // frozen image. Alpha blending is done directly in the BGRA back buffer.
    let color = (190, 205, 220);
    let alpha = 55;
    back.blend_hline(point.y, bounds.left, bounds.right, color, alpha);
    back.blend_vline(point.x, bounds.top, bounds.bottom, color, alpha);
    Ok(())
}

fn draw_handles(hdc: HDC, state: &OverlayState) -> Result<()> {
    if current_options().selection_mode != SelectionMode::Resize {
        return Ok(());
    }
    unsafe {
        let brush = CreateSolidBrush(rgb(255, 255, 255));
        for (_, rect) in selection_handle_rects(state) {
            let _ = FillRect(hdc, &rect, brush);
        }
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
    Ok(())
}

fn update_cursor(hwnd: HWND) {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return;
    }
    let _ = unsafe { ScreenToClient(hwnd, &mut point) };
    let state = with_state(|state| {
        (
            toolbar_button_at(state, point),
            selection_rect(state),
            selection_handle_at(state, point),
        )
    });
    let Ok((toolbar, selection, handle)) = state else {
        return;
    };
    let options = current_options();
    let cursor = if toolbar.is_some() {
        IDC_HAND
    } else if options.selection_mode == SelectionMode::Resize {
        if let Some(handle) = handle {
            handle_cursor(handle)
        } else if selection
            .map(|rect| {
                point.x >= rect.left
                    && point.x < rect.right
                    && point.y >= rect.top
                    && point.y < rect.bottom
            })
            .unwrap_or(false)
        {
            IDC_SIZEALL
        } else {
            IDC_ARROW
        }
    } else {
        IDC_CROSS
    };
    unsafe {
        if let Ok(cursor) = LoadCursorW(None, cursor) {
            let _ = SetCursor(cursor);
        }
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

fn cropped_selection() -> Result<(RgbaImage, RECT)> {
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
    Ok((crop, rect))
}

pub fn copy_current_selection() -> Result<CopyStats> {
    let (crop, _) = cropped_selection()?;
    let w = crop.width();
    let h = crop.height();

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
        state.hover_button = None;
        state.cursor = None;
        state.resize_drag = None;
        state.move_drag = None;
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
        state.hover_button = None;
        state.cursor = None;
        state.resize_drag = None;
        state.move_drag = None;
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

fn default_save_name() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let format = time::format_description::parse("[year][month][day]_[hour][minute][second]")
        .unwrap_or_default();
    let stamp = now.format(&format).unwrap_or_else(|_| "screenshot".to_string());
    format!("Jpixel_{stamp}.png")
}

fn ensure_png_extension(mut path: PathBuf) -> PathBuf {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("png"))
        .unwrap_or(false)
    {
        return path;
    }
    path.set_extension("png");
    path
}

fn save_current_selection() -> Result<Option<PathBuf>> {
    let (image, _) = cropped_selection()?;
    let mut dialog = rfd::FileDialog::new()
        .add_filter("PNG 图片", &["png"])
        .set_file_name(default_save_name());

    if let Ok(profile) = std::env::var("USERPROFILE") {
        let pictures = Path::new(&profile).join("Pictures");
        if pictures.is_dir() {
            dialog = dialog.set_directory(pictures);
        }
    }

    let Some(path) = dialog.save_file() else {
        return Ok(None);
    };
    let path = ensure_png_extension(path);
    let started = Instant::now();
    let mut file = File::create(&path).map_err(|e| anyhow!("create PNG failed: {e}"))?;
    let encoder = PngEncoder::new_with_quality(
        &mut file,
        CompressionType::Fast,
        FilterType::NoFilter,
    );
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ColorType::Rgba8,
        )
        .map_err(|e| anyhow!("save PNG failed: {e}"))?;
    file.sync_all()
        .map_err(|e| anyhow!("flush PNG failed: {e}"))?;
    log::info!(
        "native PNG written in {:.0}ms: {}",
        started.elapsed().as_secs_f64() * 1000.0,
        path.display()
    );
    Ok(Some(path))
}

fn show_error(message: &str) {
    let text = HSTRING::from(message);
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("Jpixel"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn hide_for_save_dialog() -> HWND {
    release_cursor_clip();
    let hwnd = with_state(|state| state.hwnd).unwrap_or(HWND(0));
    if hwnd.0 != 0 {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    hwnd
}

fn restore_after_save_dialog(hwnd: HWND) {
    if hwnd.0 == 0 {
        return;
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
    }
    force_foreground(hwnd);
    gdi::invalidate(hwnd);
}

fn save_and_close() {
    // The overlay is topmost and full-screen; hide it while the native save
    // dialog is open so the dialog is never covered. On cancel, restore the
    // selection and the overlay.
    let hwnd = hide_for_save_dialog();
    match save_current_selection() {
        Ok(Some(path)) => {
            report_current("save", None);
            log::info!("saved native capture to {}", path.display());
            hide_overlay();
        }
        Ok(None) => {
            log::debug!("native save cancelled");
            restore_after_save_dialog(hwnd);
        }
        Err(e) => {
            log::error!("native save failed: {e}");
            restore_after_save_dialog(hwnd);
            show_error(&format!("保存失败：{e}"));
        }
    }
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
                    gdi::paint_overlay(hwnd, back, dimmed, original, selection_rect(state), |hdc| {
                        draw_crosshair(back, state)?;
                        draw_handles(hdc, state)?;
                        draw_toolbar(hdc, state)
                    })
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
        WM_SETCURSOR => {
            update_cursor(hwnd);
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            let pt = point_from_lparam(lparam);
            if let Ok(Some(button)) = with_state(|state| toolbar_button_at(state, pt)) {
                handle_toolbar_click(button);
                return LRESULT(0);
            }
            let options = current_options();
            if options.selection_mode == SelectionMode::Resize {
                let selection = with_state(selection_rect).ok().flatten();
                if let Some(selection) = selection {
                    if let Some(handle) = with_state(|state| selection_handle_at(state, pt))
                        .ok()
                        .flatten()
                    {
                        let _ = SetCapture(hwnd);
                        clip_cursor_to_window(hwnd);
                        with_state_mut(|state| {
                            state.resize_drag = Some(ResizeDrag {
                                handle,
                                start_rect: selection,
                            });
                            Ok(())
                        })
                        .ok();
                        return LRESULT(0);
                    }
                    if pt.x >= selection.left
                        && pt.x < selection.right
                        && pt.y >= selection.top
                        && pt.y < selection.bottom
                    {
                        let _ = SetCapture(hwnd);
                        clip_cursor_to_window(hwnd);
                        with_state_mut(|state| {
                            state.move_drag = Some(MoveDrag {
                                start_rect: selection,
                                start_point: pt,
                            });
                            Ok(())
                        })
                        .ok();
                        return LRESULT(0);
                    }
                }
            }
            let _ = SetCapture(hwnd);
            clip_cursor_to_window(hwnd);
            with_state_mut(|state| {
                if !state.visible {
                    return Ok(());
                }
                let clamped = clamp_to_image(state, pt);
                state.dragging = true;
                state.hover_button = None;
                state.resize_drag = None;
                state.move_drag = None;
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
            with_state_mut(|state| {
                state.cursor = Some(pt);
                if let Some(drag) = state.resize_drag {
                    if let Some(bounds) = image_bounds(state) {
                        let next = resize_selection(drag.start_rect, drag.handle, pt, bounds);
                        set_selection(state, next);
                    }
                    return Ok(());
                }
                if let Some(drag) = state.move_drag {
                    if let Some(bounds) = image_bounds(state) {
                        let delta = POINT {
                            x: pt.x - drag.start_point.x,
                            y: pt.y - drag.start_point.y,
                        };
                        let next = move_selection(drag.start_rect, delta, bounds);
                        set_selection(state, next);
                    }
                    return Ok(());
                }
                if state.dragging {
                    state.end = Some(clamp_to_image(state, pt));
                    return Ok(());
                }
                state.hover_button = toolbar_button_at(state, pt);
                Ok(())
            })
            .ok();
            // Always repaint on mouse movement: the crosshair guide must track
            // the cursor even when no selection/toolbar state changed.
            gdi::invalidate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            release_cursor_clip();
            let was_adjusting = with_state_mut(|state| {
                let adjusting = state.resize_drag.is_some() || state.move_drag.is_some();
                state.resize_drag = None;
                state.move_drag = None;
                state.dragging = false;
                state.hover_button = None;
                if !adjusting {
                    if let Some(rect) = selection_rect(state) {
                        if rect.right - rect.left < 3 || rect.bottom - rect.top < 3 {
                            state.start = None;
                            state.end = None;
                        }
                    }
                }
                Ok(adjusting)
            })
            .unwrap_or(false);
            let _ = was_adjusting;
            gdi::invalidate(hwnd);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            release_cursor_clip();
            with_state_mut(|state| {
                state.dragging = false;
                state.hover_button = None;
                state.resize_drag = None;
                state.move_drag = None;
                Ok(())
            })
            .ok();
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
        WM_APP_SET_HOTKEY => {
            handle_pending_hotkey();
            LRESULT(0)
        }
        WM_APP_SET_OPTIONS => {
            gdi::invalidate(hwnd);
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

    fn fake_state(w: u32, h: u32, start: POINT, end: POINT) -> OverlayState {
        let mut state = OverlayState::new(HWND(1));
        state.image = Some(RgbaImage::new(w, h));
        state.start = Some(start);
        state.end = Some(end);
        state
    }

    #[test]
    fn toolbar_flips_above_when_selection_is_near_the_bottom() {
        let state = fake_state(
            800,
            600,
            POINT { x: 100, y: 100 },
            POINT { x: 700, y: 550 },
        );
        let rect = toolbar_rect(&state).expect("toolbar should exist");
        assert!(rect.top < 100, "toolbar should flip above the selection");
        assert!(rect.bottom <= 600);
        assert!(rect.left >= 0 && rect.right <= 800);
    }

    #[test]
    fn toolbar_hit_test_finds_copy_button() {
        let state = fake_state(
            800,
            600,
            POINT { x: 100, y: 100 },
            POINT { x: 700, y: 400 },
        );
        let buttons = toolbar_button_rects(&state);
        let (button, rect) = buttons.first().expect("toolbar buttons");
        assert_eq!(*button, ToolbarButton::Copy);
        let center = POINT {
            x: (rect.left + rect.right) / 2,
            y: (rect.top + rect.bottom) / 2,
        };
        assert_eq!(toolbar_button_at(&state, center), Some(ToolbarButton::Copy));
    }

    #[test]
    fn png_extension_is_added_or_replaced() {
        assert_eq!(ensure_png_extension(PathBuf::from("shot")), PathBuf::from("shot.png"));
        assert_eq!(
            ensure_png_extension(PathBuf::from("shot.PNG")),
            PathBuf::from("shot.PNG")
        );
        assert_eq!(
            ensure_png_extension(PathBuf::from("shot.jpg")),
            PathBuf::from("shot.png")
        );
    }

    #[test]
    fn default_save_name_is_png() {
        assert!(default_save_name().ends_with(".png"));
    }
}
