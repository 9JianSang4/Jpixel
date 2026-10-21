//! Native overlay window: capture one monitor, show the frozen frame, let the
//! user drag a rectangle, copy it, and restore focus.

mod gdi;

use self::gdi::Dib;
use crate::config::SelectionMode;
use crate::core::hotkey::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
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
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, Ellipse, EndPaint,
    FillRect, GetStockObject, RoundRect, ScreenToClient, SelectObject, SetBkMode, SetTextColor,
    UpdateWindow, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HDC, HGDIOBJ, HFONT, NULL_BRUSH, NULL_PEN,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, RegisterHotKey, ReleaseCapture, SetActiveWindow, SetCapture, SetFocus,
    UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU,
    VK_RETURN, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, ClipCursor, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowRect, LoadCursorW, MessageBoxW,
    PostMessageW, PostQuitMessage, RegisterClassW, SetCursor, SetForegroundWindow, SetWindowPos,
    ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, IDC_ARROW,
    IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, IDC_SIZEALL, MB_ICONERROR, MB_OK, MSG,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, WM_CAPTURECHANGED, WM_DESTROY,
    WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const HOTKEY_ID: i32 = 0x4A50; // "JP"
const WM_APP_SET_HOTKEY: u32 = 0x8001;
const WM_APP_SET_OPTIONS: u32 = 0x8002;
const CLASS_NAME: PCWSTR = w!("JpixelNativeOverlay");
static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub vk: u32,
    pub mods: u32,
}

impl KeyBinding {
    pub const fn new(vk: u32, mods: u32) -> Self {
        Self { vk, mods }
    }

    fn matches(self, vk: u32, mods: u32) -> bool {
        self.vk == vk && self.mods == mods
    }
}

pub const DEFAULT_COPY_HOTKEY: KeyBinding = KeyBinding::new(0x43, MOD_CONTROL); // Ctrl+C
pub const DEFAULT_SAVE_HOTKEY: KeyBinding = KeyBinding::new(0x53, MOD_CONTROL); // Ctrl+S

#[derive(Debug, Clone, Copy)]
pub struct OverlayOptions {
    pub crosshair_guides: bool,
    pub selection_mode: SelectionMode,
    pub double_press_enabled: bool,
    pub double_press_window_ms: u64,
    pub copy_hotkey: KeyBinding,
    pub save_hotkey: KeyBinding,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            crosshair_guides: true,
            selection_mode: SelectionMode::Reselect,
            double_press_enabled: false,
            double_press_window_ms: 500,
            copy_hotkey: DEFAULT_COPY_HOTKEY,
            save_hotkey: DEFAULT_SAVE_HOTKEY,
        }
    }
}

static OPTIONS: Mutex<OverlayOptions> = Mutex::new(OverlayOptions {
    crosshair_guides: true,
    selection_mode: SelectionMode::Reselect,
    double_press_enabled: false,
    double_press_window_ms: 500,
    copy_hotkey: DEFAULT_COPY_HOTKEY,
    save_hotkey: DEFAULT_SAVE_HOTKEY,
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
enum DrawTool {
    Pen,
    Eraser,
}

#[derive(Debug, Clone)]
struct EraserSegment {
    width: i32,
    points: Vec<POINT>,
}

#[derive(Debug, Clone)]
struct Stroke {
    tool: DrawTool,
    color: COLORREF,
    alpha: u8,
    width: i32,
    points: Vec<POINT>,
    /// Completed eraser sub-strokes. Width changes only affect later points.
    segments: Vec<EraserSegment>,
}

const PEN_PALETTE: [(u8, u8, u8); 9] = [
    (255, 70, 70),
    (255, 149, 0),
    (255, 214, 10),
    (52, 199, 89),
    (50, 173, 230),
    (47, 111, 237),
    (175, 82, 222),
    (255, 255, 255),
    (30, 30, 30),
];
const PEN_WIDTH: i32 = 3;
const ERASER_WIDTH: i32 = 20;
const ERASER_MIN_WIDTH: i32 = 6;
const ERASER_MAX_WIDTH: i32 = 160;
const ERASER_WHEEL_STEP: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarButton {
    Copy,
    Save,
    Pin,
    Pen,
    Undo,
    Redo,
    Close,
}

const TOOLBAR_ITEMS: [(ToolbarButton, &str); 7] = [
    (ToolbarButton::Copy, "复制"),
    (ToolbarButton::Save, "保存"),
    (ToolbarButton::Pin, "贴图"),
    (ToolbarButton::Pen, "画笔"),
    (ToolbarButton::Undo, "撤销"),
    (ToolbarButton::Redo, "重做"),
    (ToolbarButton::Close, "关闭"),
];

const TOOLBAR_BUTTON_W: i32 = 44;
const TOOLBAR_BUTTON_H: i32 = 34;
const TOOLBAR_PAD: i32 = 6;
const TOOLBAR_GAP: i32 = 2;
const TOOLBAR_OFFSET: i32 = 8;
const TOOLBAR_W: i32 = 7 * TOOLBAR_BUTTON_W + 6 * TOOLBAR_GAP + 2 * TOOLBAR_PAD;
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
    last_hotkey_press: Option<Instant>,
    tool: Option<DrawTool>,
    strokes: Vec<Stroke>,
    redo: Vec<Stroke>,
    active_stroke: Option<Stroke>,
    pen_color_index: usize,
    pen_alpha: u8,
    eraser_width: i32,
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
            last_hotkey_press: None,
            tool: None,
            strokes: Vec::new(),
            redo: Vec::new(),
            active_stroke: None,
            pen_color_index: 0,
            pen_alpha: 255,
            eraser_width: ERASER_WIDTH,
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
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
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
pub fn set_options(options: OverlayOptions) {
    if let Ok(mut current) = OPTIONS.lock() {
        *current = options;
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
        state.last_hotkey_press = None;
        state.tool = None;
        state.strokes.clear();
        state.redo.clear();
        state.active_stroke = None;
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

fn toolbar_button_enabled(state: &OverlayState, button: ToolbarButton) -> bool {
    match button {
        ToolbarButton::Copy | ToolbarButton::Save | ToolbarButton::Close => true,
        ToolbarButton::Pen => true,
        ToolbarButton::Undo => !state.strokes.is_empty(),
        ToolbarButton::Redo => !state.redo.is_empty(),
        ToolbarButton::Pin => false,
    }
}

fn toolbar_glyph(button: ToolbarButton) -> char {
    match button {
        ToolbarButton::Copy => '\u{E8C8}',
        ToolbarButton::Save => '\u{E74E}',
        ToolbarButton::Pin => '\u{E718}',
        ToolbarButton::Pen => '\u{E70F}',
        ToolbarButton::Undo => '\u{E7A7}',
        ToolbarButton::Redo => '\u{E7A6}',
        ToolbarButton::Close => '\u{E711}',
    }
}

fn pen_rgb(state: &OverlayState) -> (u8, u8, u8) {
    PEN_PALETTE[state.pen_color_index.min(PEN_PALETTE.len() - 1)]
}

fn fill_frame(hdc: HDC, rect: RECT, thickness: i32, color: COLORREF) {
    let t = thickness.max(1);
    let pieces = [
        RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.top + t,
        },
        RECT {
            left: rect.left,
            top: rect.bottom - t,
            right: rect.right,
            bottom: rect.bottom,
        },
        RECT {
            left: rect.left,
            top: rect.top + t,
            right: rect.left + t,
            bottom: rect.bottom - t,
        },
        RECT {
            left: rect.right - t,
            top: rect.top + t,
            right: rect.right,
            bottom: rect.bottom - t,
        },
    ];
    unsafe {
        let brush = CreateSolidBrush(color);
        for piece in pieces {
            let _ = FillRect(hdc, &piece, brush);
        }
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

fn draw_pen_color_preview(hdc: HDC, back: &Dib, state: &OverlayState) {
    if state.hover_button != Some(ToolbarButton::Pen) {
        return;
    }
    let Some(toolbar) = toolbar_rect(state) else {
        return;
    };
    let Some((_, pen_rect)) = toolbar_button_rects(state)
        .into_iter()
        .find(|(button, _)| *button == ToolbarButton::Pen)
    else {
        return;
    };
    let size = 24;
    let left = pen_rect.left + (TOOLBAR_BUTTON_W - size) / 2;
    let top = if toolbar.top - size - 10 >= 2 {
        toolbar.top - size - 8
    } else {
        toolbar.bottom + 6
    };
    let (r, g, b) = pen_rgb(state);
    unsafe {
        // Checkerboard so low-opacity colors (especially white) are visible.
        let light = CreateSolidBrush(rgb(236, 236, 238));
        let dark = CreateSolidBrush(rgb(118, 118, 124));
        let cell = 6;
        for row in 0..((size + cell - 1) / cell) {
            for col in 0..((size + cell - 1) / cell) {
                let rect = RECT {
                    left: left + col * cell,
                    top: top + row * cell,
                    right: (left + (col + 1) * cell).min(left + size),
                    bottom: (top + (row + 1) * cell).min(top + size),
                };
                let brush = if (row + col) % 2 == 0 { light } else { dark };
                let _ = FillRect(hdc, &rect, brush);
            }
        }
        let _ = DeleteObject(HGDIOBJ(light.0));
        let _ = DeleteObject(HGDIOBJ(dark.0));

        // Blend the selected color over the checkerboard using its alpha.
        for y in top..top + size {
            back.blend_hline(y, left, left + size, (r, g, b), state.pen_alpha);
        }

        // Inset alpha track: never touches the border, so all four edges stay 1px.
        let track = RECT {
            left: left + 4,
            top: top + size - 7,
            right: left + size - 4,
            bottom: top + size - 4,
        };
        let track_brush = CreateSolidBrush(rgb(54, 54, 58));
        let _ = FillRect(hdc, &track, track_brush);
        let _ = DeleteObject(HGDIOBJ(track_brush.0));

        let track_width = track.right - track.left;
        let fill_width = (track_width * state.pen_alpha as i32 / 255).max(1);
        let fill = RECT {
            left: track.left,
            top: track.top,
            right: (track.left + fill_width).min(track.right),
            bottom: track.bottom,
        };
        let fill_brush = CreateSolidBrush(rgb(255, 255, 255));
        let _ = FillRect(hdc, &fill, fill_brush);
        let _ = DeleteObject(HGDIOBJ(fill_brush.0));

        // Symmetric 4px dark outer frame, then a 1px white inner frame.
        let outer = RECT {
            left: left - 4,
            top: top - 4,
            right: left + size + 4,
            bottom: top + size + 4,
        };
        fill_frame(hdc, outer, 4, rgb(28, 28, 30));

        let frame = RECT {
            left,
            top,
            right: left + size,
            bottom: top + size,
        };
        fill_frame(hdc, frame, 1, rgb(255, 255, 255));
    }
}

fn draw_toolbar(hdc: HDC, back: &Dib, state: &OverlayState) -> Result<()> {
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
            -16,
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
            w!("Segoe MDL2 Assets"),
        );
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        let _ = SetBkMode(hdc, TRANSPARENT);

        for (button, rect) in toolbar_button_rects(state) {
            let enabled = toolbar_button_enabled(state, button);
            let active = matches!(
                (button, state.tool),
                (ToolbarButton::Pen, Some(DrawTool::Pen))
            );
            if enabled && (active || state.hover_button == Some(button)) {
                let fill = if active {
                    rgb(47, 111, 237)
                } else {
                    rgb(58, 58, 62)
                };
                let brush = CreateSolidBrush(fill);
                let old = SelectObject(hdc, HGDIOBJ(brush.0));
                let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
                let _ = RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, 8, 8);
                SelectObject(hdc, old_pen);
                SelectObject(hdc, old);
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
            let color = if enabled {
                rgb(245, 245, 245)
            } else {
                rgb(125, 125, 130)
            };
            let _ = SetTextColor(hdc, color);
            let mut text: Vec<u16> = vec![toolbar_glyph(button) as u16];
            let mut text_rect = rect;
            let _ = DrawTextW(
                hdc,
                &mut text,
                &mut text_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }

        draw_pen_color_preview(hdc, back, state);
        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
    Ok(())
}

fn invalidate_overlay() {
    if let Ok(hwnd) = with_state(|state| state.hwnd) {
        gdi::invalidate(hwnd);
    }
}

fn toggle_tool(tool: DrawTool) {
    let _ = with_state_mut(|state| {
        state.tool = if state.tool == Some(tool) { None } else { Some(tool) };
        state.active_stroke = None;
        Ok(())
    });
    invalidate_overlay();
}

fn undo_stroke() {
    let _ = with_state_mut(|state| {
        if let Some(stroke) = state.strokes.pop() {
            state.redo.push(stroke);
        }
        let _ = refresh_annotated(state);
        Ok(())
    });
    invalidate_overlay();
}

fn redo_stroke() {
    let _ = with_state_mut(|state| {
        if let Some(stroke) = state.redo.pop() {
            state.strokes.push(stroke);
        }
        let _ = refresh_annotated(state);
        Ok(())
    });
    invalidate_overlay();
}

fn handle_toolbar_click(button: ToolbarButton) {
    match button {
        ToolbarButton::Copy => copy_and_close(),
        ToolbarButton::Save => save_and_close(),
        ToolbarButton::Close => cancel_and_close(),
        ToolbarButton::Pen => toggle_tool(DrawTool::Pen),
        ToolbarButton::Undo => undo_stroke(),
        ToolbarButton::Redo => redo_stroke(),
        ToolbarButton::Pin => log::debug!("pin is not implemented until P2.3"),
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

fn draw_eraser_cursor(hdc: HDC, state: &OverlayState) -> Result<()> {
    let Some(stroke) = state.active_stroke.as_ref() else {
        return Ok(());
    };
    if stroke.tool != DrawTool::Eraser {
        return Ok(());
    }
    let Some(cursor) = state.cursor else {
        return Ok(());
    };
    let radius = (state.eraser_width / 2).max(1);
    unsafe {
        let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));

        let outer = CreatePen(PS_SOLID, 3, rgb(10, 10, 12));
        let old_pen = SelectObject(hdc, HGDIOBJ(outer.0));
        let _ = Ellipse(
            hdc,
            cursor.x - radius,
            cursor.y - radius,
            cursor.x + radius,
            cursor.y + radius,
        );
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(HGDIOBJ(outer.0));

        let inner = CreatePen(PS_SOLID, 1, rgb(255, 255, 255));
        let old_pen = SelectObject(hdc, HGDIOBJ(inner.0));
        let _ = Ellipse(
            hdc,
            cursor.x - radius,
            cursor.y - radius,
            cursor.x + radius,
            cursor.y + radius,
        );
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(HGDIOBJ(inner.0));

        SelectObject(hdc, old_brush);
    }
    Ok(())
}

fn clamp_to_selection(state: &OverlayState, point: POINT) -> Option<POINT> {
    let selection = selection_rect(state)?;
    Some(POINT {
        x: point.x.clamp(selection.left, (selection.right - 1).max(selection.left)),
        y: point.y.clamp(selection.top, (selection.bottom - 1).max(selection.top)),
    })
}

fn refresh_annotated(state: &mut OverlayState) -> Result<()> {
    let Some(base) = state.image.as_ref() else {
        return Ok(());
    };
    let annotated = render_strokes(base, &state.strokes, state.active_stroke.as_ref());
    if let Some(original) = state.original.as_mut() {
        original.replace_from_rgba(&annotated)?;
    }
    Ok(())
}

fn begin_stroke(hwnd: HWND, point: POINT, tool: DrawTool) -> bool {
    let started = with_state_mut(|state| {
        if state.tool != Some(DrawTool::Pen) {
            return Ok(false);
        }
        let Some(selection) = selection_rect(state) else {
            return Ok(false);
        };
        if point.x < selection.left
            || point.x >= selection.right
            || point.y < selection.top
            || point.y >= selection.bottom
        {
            return Ok(false);
        }
        let Some(point) = clamp_to_selection(state, point) else {
            return Ok(false);
        };
        let (color, alpha, width) = match tool {
            DrawTool::Pen => {
                let (r, g, b) = PEN_PALETTE[state.pen_color_index.min(PEN_PALETTE.len() - 1)];
                (
                    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16)),
                    state.pen_alpha,
                    PEN_WIDTH,
                )
            }
            DrawTool::Eraser => (COLORREF(0), 255, state.eraser_width),
        };
        state.active_stroke = Some(Stroke {
            tool,
            color,
            alpha,
            width,
            points: vec![point],
            segments: Vec::new(),
        });
        let _ = refresh_annotated(state);
        Ok(true)
    })
    .unwrap_or(false);
    if started {
        unsafe {
            let _ = SetCapture(hwnd);
        }
        clip_cursor_to_window(hwnd);
    }
    started
}

fn finish_stroke() -> bool {
    with_state_mut(|state| {
        if let Some(stroke) = state.active_stroke.take() {
            if !stroke.points.is_empty() {
                state.strokes.push(stroke);
                state.redo.clear();
                let _ = refresh_annotated(state);
            }
            return Ok(true);
        }
        Ok(false)
    })
    .unwrap_or(false)
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
        IDC_ARROW
    } else if options.selection_mode == SelectionMode::Resize && selection.is_some() {
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
        // Unified behavior: normal arrow before selection, while dragging a new
        // selection, and after selection in reselect mode.
        IDC_ARROW
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
    let (image, rect, strokes, active) = STATE.with(|cell| {
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
        Ok::<(RgbaImage, RECT, Vec<Stroke>, Option<Stroke>), anyhow::Error>((
            image.clone(),
            rect,
            state.strokes.clone(),
            state.active_stroke.clone(),
        ))
    })?;

    let image = render_strokes(&image, &strokes, active.as_ref());
    let x = rect.left.max(0) as u32;
    let y = rect.top.max(0) as u32;
    let w = (rect.right - rect.left).max(1) as u32;
    let h = (rect.bottom - rect.top).max(1) as u32;
    let crop = image::imageops::crop_imm(&image, x, y, w, h).to_image();
    Ok((crop, rect))
}

fn color_rgb(color: COLORREF) -> (u8, u8, u8) {
    (
        (color.0 & 0xff) as u8,
        ((color.0 >> 8) & 0xff) as u8,
        ((color.0 >> 16) & 0xff) as u8,
    )
}

fn paint_brush(canvas: &mut RgbaImage, center: POINT, radius: i32, color: (u8, u8, u8), alpha: u8) {
    let a = alpha as u32;
    let inv = 255 - a;
    for y in (center.y - radius)..=(center.y + radius) {
        for x in (center.x - radius)..=(center.x + radius) {
            if x < 0 || y < 0 || x >= canvas.width() as i32 || y >= canvas.height() as i32 {
                continue;
            }
            let dx = x - center.x;
            let dy = y - center.y;
            if dx * dx + dy * dy > radius * radius {
                continue;
            }
            let pixel = canvas.get_pixel_mut(x as u32, y as u32);
            let data = pixel.0;
            pixel.0 = [
                ((color.0 as u32 * a + data[0] as u32 * inv) / 255) as u8,
                ((color.1 as u32 * a + data[1] as u32 * inv) / 255) as u8,
                ((color.2 as u32 * a + data[2] as u32 * inv) / 255) as u8,
                255,
            ];
        }
    }
}

fn erase_brush(canvas: &mut RgbaImage, base: &RgbaImage, cx: i32, cy: i32, radius: i32) {
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            if x >= 0 && y >= 0 && x < canvas.width() as i32 && y < canvas.height() as i32 {
                let pixel = *base.get_pixel(x as u32, y as u32);
                canvas.put_pixel(x as u32, y as u32, pixel);
            }
        }
    }
}

fn render_eraser_points(
    canvas: &mut RgbaImage,
    base: &RgbaImage,
    width: i32,
    points: &[POINT],
) {
    if points.is_empty() {
        return;
    }
    let radius = (width / 2).max(1);
    if points.len() == 1 {
        erase_brush(canvas, base, points[0].x, points[0].y, radius);
        return;
    }
    for pair in points.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let dy = pair[1].y - pair[0].y;
        let steps = dx.abs().max(dy.abs()).max(1);
        for index in 0..=steps {
            let t = index as f32 / steps as f32;
            let cx = pair[0].x + (dx as f32 * t).round() as i32;
            let cy = pair[0].y + (dy as f32 * t).round() as i32;
            erase_brush(canvas, base, cx, cy, radius);
        }
    }
}

fn render_strokes(base: &RgbaImage, strokes: &[Stroke], active: Option<&Stroke>) -> RgbaImage {
    let mut canvas = base.clone();
    for stroke in strokes.iter().chain(active.into_iter()) {
        if stroke.points.is_empty() && stroke.segments.is_empty() {
            continue;
        }
        match stroke.tool {
            DrawTool::Pen => {
                let color = color_rgb(stroke.color);
                let radius = (stroke.width / 2).max(1);
                if stroke.points.len() == 1 {
                    paint_brush(&mut canvas, stroke.points[0], radius, color, stroke.alpha);
                } else {
                    for pair in stroke.points.windows(2) {
                        let dx = pair[1].x - pair[0].x;
                        let dy = pair[1].y - pair[0].y;
                        let steps = dx.abs().max(dy.abs()).max(1);
                        for index in 0..=steps {
                            let t = index as f32 / steps as f32;
                            let x = pair[0].x + (dx as f32 * t).round() as i32;
                            let y = pair[0].y + (dy as f32 * t).round() as i32;
                            paint_brush(&mut canvas, POINT { x, y }, radius, color, stroke.alpha);
                        }
                    }
                }
            }
            DrawTool::Eraser => {
                for segment in &stroke.segments {
                    render_eraser_points(&mut canvas, base, segment.width, &segment.points);
                }
                render_eraser_points(&mut canvas, base, stroke.width, &stroke.points);
            }
        }
    }
    canvas
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
        state.last_hotkey_press = None;
        state.tool = None;
        state.strokes.clear();
        state.redo.clear();
        state.active_stroke = None;
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
        state.last_hotkey_press = None;
        state.tool = None;
        state.strokes.clear();
        state.redo.clear();
        state.active_stroke = None;
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

fn active_modifiers() -> u32 {
    let mut mods = 0;
    unsafe {
        if GetKeyState(VK_CONTROL.0 as i32) < 0 {
            mods |= MOD_CONTROL;
        }
        if GetKeyState(VK_MENU.0 as i32) < 0 {
            mods |= MOD_ALT;
        }
        if GetKeyState(VK_SHIFT.0 as i32) < 0 {
            mods |= MOD_SHIFT;
        }
        if GetKeyState(VK_LWIN.0 as i32) < 0 || GetKeyState(VK_RWIN.0 as i32) < 0 {
            mods |= MOD_WIN;
        }
    }
    mods
}

fn handle_hotkey() {
    let recapture = is_visible();
    if !recapture && current_options().double_press_enabled {
        let now = Instant::now();
        let window = Duration::from_millis(current_options().double_press_window_ms);
        let should_capture = with_state_mut(|state| {
            let should = state
                .last_hotkey_press
                .map(|last| now.duration_since(last) <= window)
                .unwrap_or(false);
            state.last_hotkey_press = if should { None } else { Some(now) };
            Ok(should)
        })
        .unwrap_or(false);
        if !should_capture {
            log::debug!("double-press enabled: waiting for the second press");
            return;
        }
    }
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
                        draw_eraser_cursor(hdc, state)?;
                        draw_toolbar(hdc, back, state)
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
                gdi::invalidate(hwnd);
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
                    if begin_stroke(hwnd, pt, DrawTool::Pen) {
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
            } else if begin_stroke(hwnd, pt, DrawTool::Pen) {
                return LRESULT(0);
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
                state.active_stroke = None;
                state.strokes.clear();
                state.redo.clear();
                let _ = refresh_annotated(state);
                state.start = Some(clamped);
                state.end = Some(clamped);
                Ok(())
            })
            .ok();
            gdi::invalidate(hwnd);
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let pt = point_from_lparam(lparam);
            let _ = with_state_mut(|state| {
                state.cursor = Some(pt);
                state.hover_button = None;
                Ok(())
            });
            let _ = begin_stroke(hwnd, pt, DrawTool::Eraser);
            gdi::invalidate(hwnd);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let _ = ReleaseCapture();
            release_cursor_clip();
            if finish_stroke() {
                gdi::invalidate(hwnd);
                update_cursor(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as i32 / 120;
            let mut point = POINT {
                x: (lparam.0 & 0xffff) as u16 as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32,
            };
            let _ = unsafe { ScreenToClient(hwnd, &mut point) };
            let erasing = with_state(|state| {
                state
                    .active_stroke
                    .as_ref()
                    .is_some_and(|stroke| stroke.tool == DrawTool::Eraser)
            })
            .unwrap_or(false);
            if erasing && delta != 0 {
                let _ = with_state_mut(|state| {
                    let next = (state.eraser_width + delta * ERASER_WHEEL_STEP)
                        .clamp(ERASER_MIN_WIDTH, ERASER_MAX_WIDTH);
                    state.eraser_width = next;
                    if let Some(stroke) = state.active_stroke.as_mut() {
                        let old_width = stroke.width;
                        if old_width != next {
                            if let Some(last) = stroke.points.last().copied() {
                                let points = std::mem::take(&mut stroke.points);
                                stroke.segments.push(EraserSegment {
                                    width: old_width,
                                    points,
                                });
                                stroke.points = vec![last];
                            }
                            stroke.width = next;
                        }
                    }
                    let _ = refresh_annotated(state);
                    Ok(())
                });
                gdi::invalidate(hwnd);
                return LRESULT(0);
            }
            let over_pen =
                with_state(|state| toolbar_button_at(state, point) == Some(ToolbarButton::Pen))
                    .unwrap_or(false);
            if over_pen && delta != 0 {
                let ctrl = active_modifiers() & MOD_CONTROL != 0;
                let _ = with_state_mut(|state| {
                    if ctrl {
                        state.pen_alpha =
                            (state.pen_alpha as i32 + delta * 20).clamp(30, 255) as u8;
                    } else {
                        let len = PEN_PALETTE.len() as i32;
                        state.pen_color_index =
                            (((state.pen_color_index as i32 - delta) % len + len) % len) as usize;
                    }
                    Ok(())
                });
                gdi::invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let pt = point_from_lparam(lparam);
            with_state_mut(|state| {
                state.cursor = Some(pt);
                if state.active_stroke.is_some() {
                    if let Some(point) = clamp_to_selection(state, pt) {
                        if let Some(stroke) = state.active_stroke.as_mut() {
                            if stroke.points.last() != Some(&point) {
                                stroke.points.push(point);
                            }
                        }
                    }
                    let _ = refresh_annotated(state);
                    return Ok(());
                }
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
            if finish_stroke() {
                gdi::invalidate(hwnd);
                update_cursor(hwnd);
                return LRESULT(0);
            }
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
            update_cursor(hwnd);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            release_cursor_clip();
            finish_stroke();
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
            let key = wparam.0 as u32;
            let options = current_options();
            let mods = active_modifiers();
            if key == VK_ESCAPE.0 as u32 {
                cancel_and_close();
            } else if key == VK_RETURN.0 as u32 {
                copy_and_close();
            } else if options.copy_hotkey.matches(key, mods) {
                copy_and_close();
            } else if options.save_hotkey.matches(key, mods) {
                save_and_close();
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
    use image::Rgba;

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

    #[test]
    fn key_binding_requires_exact_modifiers() {
        assert!(DEFAULT_COPY_HOTKEY.matches(0x43, MOD_CONTROL));
        assert!(!DEFAULT_COPY_HOTKEY.matches(0x43, MOD_CONTROL | MOD_SHIFT));
        assert!(!DEFAULT_COPY_HOTKEY.matches(0x53, MOD_CONTROL));
    }

    #[test]
    fn pen_stroke_is_composited_into_image() {
        let base = RgbaImage::from_pixel(10, 10, Rgba([255, 255, 255, 255]));
        let stroke = Stroke {
            tool: DrawTool::Pen,
            color: COLORREF(255 | (70 << 8) | (70 << 16)),
            alpha: 255,
            width: 3,
            points: vec![POINT { x: 2, y: 5 }, POINT { x: 8, y: 5 }],
            segments: Vec::new(),
        };
        let rendered = render_strokes(&base, &[stroke], None);
        assert_eq!(rendered.get_pixel(5, 5), &Rgba([255, 70, 70, 255]));
    }

    #[test]
    fn eraser_restores_base_pixels() {
        let base = RgbaImage::from_pixel(10, 10, Rgba([255, 255, 255, 255]));
        let pen = Stroke {
            tool: DrawTool::Pen,
            color: COLORREF(0 | (0 << 8) | (255 << 16)),
            alpha: 255,
            width: 3,
            points: vec![POINT { x: 2, y: 5 }, POINT { x: 8, y: 5 }],
            segments: Vec::new(),
        };
        let eraser = Stroke {
            tool: DrawTool::Eraser,
            color: COLORREF(0),
            alpha: 255,
            width: 10,
            points: vec![POINT { x: 5, y: 5 }, POINT { x: 5, y: 5 }],
            segments: Vec::new(),
        };
        let rendered = render_strokes(&base, &[pen, eraser], None);
        assert_eq!(rendered.get_pixel(5, 5), &Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn eraser_segments_keep_their_original_width() {
        let base = RgbaImage::from_pixel(24, 18, Rgba([255, 255, 255, 255]));
        let pen_left = Stroke {
            tool: DrawTool::Pen,
            color: COLORREF(255 | (70 << 8) | (70 << 16)),
            alpha: 255,
            width: 3,
            points: vec![POINT { x: 4, y: 0 }, POINT { x: 4, y: 17 }],
            segments: Vec::new(),
        };
        let pen_right = Stroke {
            tool: DrawTool::Pen,
            color: COLORREF(255 | (70 << 8) | (70 << 16)),
            alpha: 255,
            width: 3,
            points: vec![POINT { x: 18, y: 0 }, POINT { x: 18, y: 17 }],
            segments: Vec::new(),
        };
        let eraser = Stroke {
            tool: DrawTool::Eraser,
            color: COLORREF(0),
            alpha: 255,
            width: 18,
            points: vec![POINT { x: 18, y: 9 }],
            segments: vec![EraserSegment {
                width: 4,
                points: vec![POINT { x: 4, y: 9 }],
            }],
        };
        let rendered = render_strokes(&base, &[pen_left, pen_right, eraser], None);
        // The old 4px seg must not grow to the current 18px width.
        assert_eq!(rendered.get_pixel(4, 3), &Rgba([255, 70, 70, 255]));
        assert_eq!(rendered.get_pixel(4, 9), &Rgba([255, 255, 255, 255]));
        assert_eq!(rendered.get_pixel(18, 4), &Rgba([255, 255, 255, 255]));
    }
}
