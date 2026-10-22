//! Native overlay window: capture one monitor, show the frozen frame, let the
//! user drag a rectangle, copy it, and restore focus.

pub(crate) mod gdi;

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
use windows::Win32::Globalization::HIMC;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject,
    DrawTextW, Ellipse, EndPaint, FillRect, GetStockObject, IntersectClipRect, RestoreDC,
    RoundRect, SaveDC, ScreenToClient, SelectClipRgn, SelectObject, SetBkMode, SetStretchBltMode,
    SetTextColor, StretchBlt, UpdateWindow, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLORONCOLOR,
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_BOLD,
    FW_NORMAL, HDC, HGDIOBJ, HFONT, NULL_BRUSH, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::Ime::ImmAssociateContext;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, RegisterHotKey, ReleaseCapture, SetActiveWindow, SetCapture, SetFocus,
    UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU,
    VK_RETURN, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, ClipCursor, CreateWindowExW, DefWindowProcW,
    DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowRect,
    GetWindowThreadProcessId, KillTimer, LoadCursorW, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, IDC_ARROW, IDC_SIZENESW, IDC_SIZENS,
    IDC_SIZENWSE, IDC_SIZEWE, IDC_SIZEALL, MB_ICONERROR, MB_OK, MSG, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, WM_CAPTURECHANGED, WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const HOTKEY_ID: i32 = 0x4A50; // "JP"
const WM_APP_SET_HOTKEY: u32 = 0x8001;
const WM_APP_SET_OPTIONS: u32 = 0x8002;
const WM_APP_PIN_LIMIT: u32 = 0x8003;
const WM_APP_CONFIRM_PIN: u32 = 0x8004;
const PREVIEW_TIMER_ID: usize = 0x51;
const PREVIEW_ANIM_MS: u128 = 160;
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
pub const DEFAULT_PIN_HOTKEY: KeyBinding = KeyBinding::new(0x54, MOD_CONTROL); // Ctrl+T

#[derive(Debug, Clone, Copy)]
pub struct OverlayOptions {
    pub crosshair_guides: bool,
    pub magnifier_enabled: bool,
    pub selection_mode: SelectionMode,
    pub double_press_enabled: bool,
    pub double_press_window_ms: u64,
    pub copy_hotkey: KeyBinding,
    pub save_hotkey: KeyBinding,
    pub pin_hotkey: KeyBinding,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            crosshair_guides: true,
            magnifier_enabled: true,
            selection_mode: SelectionMode::Reselect,
            double_press_enabled: false,
            double_press_window_ms: 500,
            copy_hotkey: DEFAULT_COPY_HOTKEY,
            save_hotkey: DEFAULT_SAVE_HOTKEY,
            pin_hotkey: DEFAULT_PIN_HOTKEY,
        }
    }
}

static OPTIONS: Mutex<OverlayOptions> = Mutex::new(OverlayOptions {
    crosshair_guides: true,
    magnifier_enabled: true,
    selection_mode: SelectionMode::Reselect,
    double_press_enabled: false,
    double_press_window_ms: 500,
    copy_hotkey: DEFAULT_COPY_HOTKEY,
    save_hotkey: DEFAULT_SAVE_HOTKEY,
    pin_hotkey: DEFAULT_PIN_HOTKEY,
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
static STARTUP_HOTKEY: Mutex<Option<crate::core::hotkey::Hotkey>> = Mutex::new(None);

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
const TOOLBAR_ANIM_MS: u128 = 160;
const TOOLBAR_ANIM_OFFSET: i32 = 8;
const TOOLBAR_TIMER_ID: usize = 0x52;
const COLOR_COPY_TIMER_ID: usize = 0x53;
const HOTKEY_RETRY_TIMER_ID: usize = 0x54;
const MAGNIFIER_TIMER_ID: usize = 0x55;
const FOCUS_RETRY_TIMER_ID: usize = 0x56;
const MAGNIFIER_SLIDE_MS: u128 = 100;
const MAG_PANEL_W: i32 = 200;
const MAG_PANEL_H: i32 = 232;
const MAG_IMAGE_SIZE: i32 = 152;
const MAG_PAD: i32 = 14;
const MAG_SOURCE: i32 = 15;

#[derive(Debug, Clone, Copy)]
struct ToolbarMotion {
    from: POINT,
    to: POINT,
    start: Instant,
}

struct MagnifierMorph {
    from: RECT,
    to: RECT,
    start: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MagnifierSide {
    Above,
    Below,
    Left,
    Right,
    BottomRight,
}

struct MagnifierMotion {
    from: POINT,
    to: POINT,
    start: Instant,
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
    annotated: Option<RgbaImage>,
    active_applied: usize,
    pending_pin_confirm: bool,
    preview_start: Option<Instant>,
    toolbar_motion: Option<ToolbarMotion>,
    toolbar_has_appeared: bool,
    toolbar_freeze: Option<RECT>,
    toolbar_last: Option<RECT>,
    selection_pending_start: Option<POINT>,
    mouse_up_releasing: bool,
    color_copied: Option<Instant>,
    magnifier_morph: Option<MagnifierMorph>,
    magnifier_consumed: bool,
    magnifier_side: MagnifierSide,
    magnifier_side_since: Instant,
    magnifier_motion: Option<MagnifierMotion>,
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
            annotated: None,
            active_applied: 0,
            pending_pin_confirm: false,
            preview_start: None,
            toolbar_motion: None,
            toolbar_has_appeared: false,
            toolbar_freeze: None,
            toolbar_last: None,
            selection_pending_start: None,
            mouse_up_releasing: false,
            color_copied: None,
            magnifier_morph: None,
            magnifier_consumed: false,
            magnifier_side: MagnifierSide::BottomRight,
            magnifier_side_since: Instant::now(),
            magnifier_motion: None,
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
        // The overlay is a canvas, not a text field. Detaching the IME context
        // prevents Chinese input methods from opening a candidate window when
        // the user presses C to copy a color value.
        let _ = ImmAssociateContext(hwnd, HIMC(0));
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
    crate::pin::register_class()?;
    crate::bubble::register_class()?;
    let monitors = monitor::enumerate()?;
    warm_up(&monitors);
    if let Err(e) = register_hotkey(hotkey.vk, hotkey.mods, &hotkey.name) {
        log::error!("cannot register startup hotkey {}: {e}", hotkey.name);
        if let Ok(mut pending) = STARTUP_HOTKEY.lock() {
            *pending = Some(hotkey.clone());
        }
        if let Ok(hwnd) = with_state(|state| state.hwnd) {
            unsafe {
                let _ = SetTimer(hwnd, HOTKEY_RETRY_TIMER_ID, 2000, None);
            }
        }
        crate::bubble::show_hotkey_error(&hotkey.name);
    }
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

pub fn notify_pin_limit() {
    let raw = OVERLAY_HWND.load(Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(HWND(raw), WM_APP_PIN_LIMIT, WPARAM(0), LPARAM(0));
        }
    }
}

/// Tray click fallback for confirming a pin-limit replacement.
pub fn trigger_pin_confirm() {
    let raw = OVERLAY_HWND.load(Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(HWND(raw), WM_APP_CONFIRM_PIN, WPARAM(0), LPARAM(0));
        }
    }
}

fn try_register_hotkey(vk: u32, mods: u32, name: &str) -> Result<()> {
    let hwnd = with_state(|state| state.hwnd)?;
    let modifiers = HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT;
    if let Err(e) = unsafe { RegisterHotKey(hwnd, HOTKEY_ID, modifiers, vk) } {
        return Err(anyhow!(
            "RegisterHotKey({name}) failed: {e}; another app may already own it"
        ));
    }
    let _ = with_state_mut(|state| {
        state.hotkey_vk = vk;
        state.hotkey_mods = mods;
        state.hotkey_name = name.to_string();
        Ok(())
    });
    Ok(())
}

pub fn register_hotkey(vk: u32, mods: u32, name: &str) -> Result<()> {
    if let Err(e) = try_register_hotkey(vk, mods, name) {
        log::error!("{e}");
        return Err(e);
    }
    let hwnd = with_state(|state| state.hwnd)?;
    log::info!(
        "registered native hotkey: {name} (overlay hwnd=0x{:X})",
        hwnd.0
    );
    Ok(())
}

fn retry_startup_hotkey(hwnd: HWND) {
    let pending = STARTUP_HOTKEY
        .lock()
        .ok()
        .and_then(|hotkey| hotkey.clone());
    let Some(hotkey) = pending else {
        unsafe {
            let _ = KillTimer(hwnd, HOTKEY_RETRY_TIMER_ID);
        }
        return;
    };
    if try_register_hotkey(hotkey.vk, hotkey.mods, &hotkey.name).is_ok() {
        if let Ok(mut pending) = STARTUP_HOTKEY.lock() {
            *pending = None;
        }
        unsafe {
            let _ = KillTimer(hwnd, HOTKEY_RETRY_TIMER_ID);
        }
        log::info!("recovered screenshot hotkey {}", hotkey.name);
        crate::bubble::show_hotkey_ready(&hotkey.name);
    }
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
    if let Ok(mut pending) = STARTUP_HOTKEY.lock() {
        *pending = None;
    }
    unsafe {
        let _ = KillTimer(hwnd, HOTKEY_RETRY_TIMER_ID);
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
        let foreground = GetForegroundWindow();
        let foreground_thread = if foreground.0 != 0 {
            GetWindowThreadProcessId(foreground, None)
        } else {
            0
        };
        let current_thread = GetCurrentThreadId();
        let attached = foreground_thread != 0
            && foreground_thread != current_thread
            && AttachThreadInput(current_thread, foreground_thread, true).as_bool();

        let _ = BringWindowToTop(hwnd);
        let set_foreground = SetForegroundWindow(hwnd).as_bool();
        let _ = SetActiveWindow(hwnd);
        let set_focus = SetFocus(hwnd).0 != 0;

        if attached {
            let _ = AttachThreadInput(current_thread, foreground_thread, false);
        }
        // Foreground activation can be denied for a short moment after a fast
        // hotkey sequence. Retry once shortly after showing so Esc is never
        // silently delivered to the previous app.
        let _ = SetTimer(hwnd, FOCUS_RETRY_TIMER_ID, 50, None);
        log::debug!(
            "native overlay focus: attached={} set_foreground={} set_focus={} foreground={} target={}",
            attached,
            set_foreground,
            set_focus,
            GetForegroundWindow().0,
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
        state.annotated = Some(image.clone());
        state.image = Some(image);
        state.active_applied = 0;
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
        state.pending_pin_confirm = false;
        state.preview_start = None;
        state.toolbar_motion = None;
        state.toolbar_has_appeared = false;
        state.toolbar_freeze = None;
        state.toolbar_last = None;
        state.selection_pending_start = None;
        state.mouse_up_releasing = false;
        state.color_copied = None;
        state.magnifier_morph = None;
        state.magnifier_consumed = false;
        state.magnifier_side = MagnifierSide::BottomRight;
        state.magnifier_side_since = Instant::now();
        state.magnifier_motion = None;
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

fn toolbar_target_rect(state: &OverlayState) -> Option<RECT> {
    if state.dragging {
        return state.toolbar_freeze;
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

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn toolbar_motion_point(motion: &ToolbarMotion, now: Instant) -> POINT {
    let progress = (now.duration_since(motion.start).as_millis() as f32
        / TOOLBAR_ANIM_MS as f32)
        .min(1.0);
    let eased = ease_out_cubic(progress);
    POINT {
        x: motion.from.x + ((motion.to.x - motion.from.x) as f32 * eased).round() as i32,
        y: motion.from.y + ((motion.to.y - motion.from.y) as f32 * eased).round() as i32,
    }
}

fn rects_intersect(a: RECT, b: RECT) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

fn rect_union(a: RECT, b: RECT) -> RECT {
    RECT {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

fn rect_expand(rect: RECT, amount: i32) -> RECT {
    RECT {
        left: rect.left - amount,
        top: rect.top - amount,
        right: rect.right + amount,
        bottom: rect.bottom + amount,
    }
}

fn point_rect(point: POINT, width: i32, height: i32) -> RECT {
    RECT {
        left: point.x,
        top: point.y,
        right: point.x + width,
        bottom: point.y + height,
    }
}

fn magnifier_morph_rect_at(morph: &MagnifierMorph, at: Instant) -> RECT {
    let t = (at.saturating_duration_since(morph.start).as_millis() as f32
        / TOOLBAR_ANIM_MS as f32)
        .clamp(0.0, 1.0);
    let eased = ease_out_cubic(t);
    RECT {
        left: morph.from.left
            + ((morph.to.left - morph.from.left) as f32 * eased).round() as i32,
        top: morph.from.top
            + ((morph.to.top - morph.from.top) as f32 * eased).round() as i32,
        right: morph.from.right
            + ((morph.to.right - morph.from.right) as f32 * eased).round() as i32,
        bottom: morph.from.bottom
            + ((morph.to.bottom - morph.from.bottom) as f32 * eased).round() as i32,
    }
}

fn toolbar_dirty_rect(state: &OverlayState) -> Option<RECT> {
    if let Some(morph) = state.magnifier_morph.as_ref() {
        // While morphing, the toolbar is clipped to the morph card, so only
        // the card's current and immediately previous rectangles are visible.
        // Including the toolbar target here would recreate the huge diagonal
        // dirty rectangle this optimization is meant to avoid.
        let now = Instant::now();
        let current = magnifier_morph_rect_at(morph, now);
        let previous_at = now
            .checked_sub(Duration::from_millis(50))
            .unwrap_or(morph.start);
        let previous = magnifier_morph_rect_at(morph, previous_at);
        return Some(rect_expand(rect_union(current, previous), 8));
    }

    let mut dirty = toolbar_rect(state);
    if let Some(motion) = state.toolbar_motion.as_ref() {
        let from = point_rect(motion.from, TOOLBAR_W, TOOLBAR_H);
        let to = point_rect(motion.to, TOOLBAR_W, TOOLBAR_H);
        let motion_rect = rect_union(from, to);
        dirty = Some(dirty.map(|rect| rect_union(rect, motion_rect)).unwrap_or(motion_rect));
    }
    dirty.map(|rect| rect_expand(rect, 8))
}

fn magnifier_dirty_rect(state: &OverlayState) -> Option<RECT> {
    let mut dirty = magnifier_panel_rect(state);
    if let Some(motion) = state.magnifier_motion.as_ref() {
        let from = point_rect(motion.from, MAG_PANEL_W, MAG_PANEL_H);
        let to = point_rect(motion.to, MAG_PANEL_W, MAG_PANEL_H);
        dirty = Some(dirty.map(|rect| rect_union(rect, from)).unwrap_or(from));
        dirty = Some(dirty.map(|rect| rect_union(rect, to)).unwrap_or(to));
    }
    dirty.map(|rect| rect_expand(rect, 6))
}

fn invalidate_dirty(hwnd: HWND, dirty: Option<RECT>) {
    if let Some(rect) = dirty {
        gdi::invalidate_rect(hwnd, &rect);
    } else {
        gdi::invalidate(hwnd);
    }
}

fn toolbar_outside_selection(selection: RECT, screen_w: i32, screen_h: i32, current: RECT) -> RECT {
    let margin = TOOLBAR_OFFSET;
    let min_x = 4;
    let max_x = (screen_w - TOOLBAR_W - 4).max(4);
    let min_y = 4;
    let max_y = (screen_h - TOOLBAR_H - 4).max(4);
    let aligned_left = (selection.right - TOOLBAR_W).clamp(min_x, max_x);
    let aligned_top = selection.top.clamp(min_y, max_y);
    let candidates = [
        RECT {
            left: aligned_left,
            top: selection.bottom + margin,
            right: aligned_left + TOOLBAR_W,
            bottom: selection.bottom + margin + TOOLBAR_H,
        },
        RECT {
            left: aligned_left,
            top: selection.top - TOOLBAR_H - margin,
            right: aligned_left + TOOLBAR_W,
            bottom: selection.top - margin,
        },
        RECT {
            left: selection.right + margin,
            top: aligned_top,
            right: selection.right + margin + TOOLBAR_W,
            bottom: aligned_top + TOOLBAR_H,
        },
        RECT {
            left: selection.left - TOOLBAR_W - margin,
            top: aligned_top,
            right: selection.left - margin,
            bottom: aligned_top + TOOLBAR_H,
        },
    ];

    candidates
        .into_iter()
        .filter(|rect| {
            rect.left >= min_x
                && rect.top >= min_y
                && rect.right <= screen_w - 4
                && rect.bottom <= screen_h - 4
                && !rects_intersect(*rect, selection)
        })
        .min_by_key(|rect| {
            (rect.left - current.left).abs() + (rect.top - current.top).abs()
        })
        .unwrap_or(current)
}

fn toolbar_rect(state: &OverlayState) -> Option<RECT> {
    if state.dragging {
        let freeze = state.toolbar_freeze?;
        let Some(motion) = state.toolbar_motion else {
            return Some(freeze);
        };
        let point = toolbar_motion_point(&motion, Instant::now());
        return Some(RECT {
            left: point.x,
            top: point.y,
            right: point.x + TOOLBAR_W,
            bottom: point.y + TOOLBAR_H,
        });
    }
    let target = toolbar_target_rect(state)?;
    let Some(motion) = state.toolbar_motion else {
        return Some(target);
    };
    let point = toolbar_motion_point(&motion, Instant::now());
    Some(RECT {
        left: point.x,
        top: point.y,
        right: point.x + TOOLBAR_W,
        bottom: point.y + TOOLBAR_H,
    })
}

fn sync_toolbar_motion(state: &mut OverlayState) {
    let Some(target) = toolbar_target_rect(state) else {
        return;
    };
    let target_point = POINT {
        x: target.left,
        y: target.top,
    };
    state.toolbar_last = Some(target);
    let now = Instant::now();
    if !state.toolbar_has_appeared {
        state.toolbar_has_appeared = true;
        let from = POINT {
            x: target_point.x,
            y: target_point.y + TOOLBAR_ANIM_OFFSET,
        };
        state.toolbar_motion = Some(ToolbarMotion {
            from,
            to: target_point,
            start: now,
        });
    } else if state
        .toolbar_motion
        .map(|motion| motion.to != target_point)
        .unwrap_or(true)
    {
        let from = state
            .toolbar_motion
            .map(|motion| toolbar_motion_point(&motion, now))
            .unwrap_or(target_point);
        state.toolbar_motion = Some(ToolbarMotion {
            from,
            to: target_point,
            start: now,
        });
    }
    unsafe {
        let _ = SetTimer(state.hwnd, TOOLBAR_TIMER_ID, 16, None);
    }
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
        ToolbarButton::Pin => true,
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

fn draw_pen_color_preview(hdc: HDC, _back: &Dib, state: &OverlayState) {
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
    let progress = state
        .preview_start
        .map(|start| (start.elapsed().as_millis() as f32 / PREVIEW_ANIM_MS as f32).min(1.0))
        .unwrap_or(1.0);
    let eased = 1.0 - (1.0 - progress).powi(3);
    let size = 16 + ((24 - 16) as f32 * eased).round() as i32;
    let left = pen_rect.left + (TOOLBAR_BUTTON_W - size) / 2;
    let above = toolbar.top - 24 - 10 >= 2;
    let base_top = if above {
        toolbar.top - 24 - 8
    } else {
        toolbar.bottom + 6
    };
    let slide = ((1.0 - eased) * 8.0).round() as i32;
    let top = if above {
        base_top + slide
    } else {
        base_top - slide
    };
    let (r, g, b) = pen_rgb(state);
    unsafe {
        crate::round_rect::draw_round_rect(
            hdc,
            left - 4,
            top - 4,
            left + size + 4,
            top + size + 4,
            10.0,
            Some(0xFF1C1C1E),
            None,
        );

        // Checkerboard so low-opacity colors (especially white) are visible.
        // Clip it to the rounded card so it cannot spill past the border.
        let clip = CreateRoundRectRgn(left, top, left + size + 1, top + size + 1, 10, 10);
        let saved = SaveDC(hdc);
        SelectClipRgn(hdc, clip);
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
        let _ = RestoreDC(hdc, saved);
        let _ = DeleteObject(HGDIOBJ(clip.0));

        // Rounded alpha fill, clipped by GDI+ so it cannot exceed the border.
        let alpha = state.pen_alpha as u32;
        let argb = (alpha << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        crate::round_rect::draw_round_rect(
            hdc,
            left,
            top,
            left + size,
            top + size,
            6.0,
            Some(argb),
            None,
        );

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

        // Vector border, anti-aliased by GDI+.
        crate::round_rect::draw_round_rect(
            hdc,
            left,
            top,
            left + size,
            top + size,
            6.0,
            None,
            Some((0xFFFFFFFF, 1.0)),
        );
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
        let _ = rebuild_annotated(state);
        Ok(())
    });
    invalidate_overlay();
}

fn redo_stroke() {
    let _ = with_state_mut(|state| {
        if let Some(stroke) = state.redo.pop() {
            state.strokes.push(stroke);
        }
        let _ = rebuild_annotated(state);
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
        ToolbarButton::Pin => pin_and_close(),
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

fn magnifier_candidate(state: &OverlayState, side: MagnifierSide) -> Option<RECT> {
    let cursor = state.cursor?;
    let bounds = image_bounds(state)?;
    let selection = selection_rect(state);
    let margin = 12;
    let cursor_gap = 18;
    let (left, top) = match (selection, side) {
        (Some(selection), MagnifierSide::Above) => (
            cursor.x - MAG_PANEL_W / 2,
            selection.top - MAG_PANEL_H - margin,
        ),
        (Some(selection), MagnifierSide::Below) => (
            cursor.x - MAG_PANEL_W / 2,
            selection.bottom + margin,
        ),
        (Some(selection), MagnifierSide::Left) => (
            selection.left - MAG_PANEL_W - margin,
            cursor.y - MAG_PANEL_H / 2,
        ),
        (Some(selection), MagnifierSide::Right) => (
            selection.right + margin,
            cursor.y - MAG_PANEL_H / 2,
        ),
        (Some(selection), MagnifierSide::BottomRight) => (
            selection.right + margin,
            selection.bottom + margin,
        ),
        (None, MagnifierSide::Above) => (
            cursor.x - MAG_PANEL_W / 2,
            cursor.y - MAG_PANEL_H - cursor_gap,
        ),
        (None, MagnifierSide::Below) => (
            cursor.x - MAG_PANEL_W / 2,
            cursor.y + cursor_gap,
        ),
        (None, MagnifierSide::Left) => (
            cursor.x - MAG_PANEL_W - cursor_gap,
            cursor.y - MAG_PANEL_H / 2,
        ),
        (None, MagnifierSide::Right) => (
            cursor.x + cursor_gap,
            cursor.y - MAG_PANEL_H / 2,
        ),
        (None, MagnifierSide::BottomRight) => (
            cursor.x + cursor_gap,
            cursor.y + cursor_gap,
        ),
    };
    let max_x = (bounds.right - MAG_PANEL_W).max(bounds.left);
    let max_y = (bounds.bottom - MAG_PANEL_H).max(bounds.top);
    let rect = RECT {
        left: left.clamp(bounds.left, max_x),
        top: top.clamp(bounds.top, max_y),
        right: 0,
        bottom: 0,
    };
    let rect = RECT {
        left: rect.left,
        top: rect.top,
        right: rect.left + MAG_PANEL_W,
        bottom: rect.top + MAG_PANEL_H,
    };
    if rect.left < bounds.left
        || rect.top < bounds.top
        || rect.right > bounds.right
        || rect.bottom > bounds.bottom
    {
        return None;
    }
    // Never sit on top of the cursor itself. Without this, a panel clamped to
    // the top edge can cover the pixel it is supposed to magnify.
    let keepout = RECT {
        left: cursor.x - 16,
        top: cursor.y - 16,
        right: cursor.x + 16,
        bottom: cursor.y + 16,
    };
    if rects_intersect(rect, keepout) {
        return None;
    }
    if let Some(selection) = selection {
        if rects_intersect(rect, selection) {
            return None;
        }
    }
    Some(rect)
}

fn magnifier_side_space(state: &OverlayState, side: MagnifierSide) -> i32 {
    let Some(cursor) = state.cursor else {
        return 0;
    };
    let Some(bounds) = image_bounds(state) else {
        return 0;
    };
    let anchor = selection_rect(state);
    match side {
        MagnifierSide::Above => anchor.map(|r| r.top).unwrap_or(cursor.y) - bounds.top,
        MagnifierSide::Below => bounds.bottom - anchor.map(|r| r.bottom).unwrap_or(cursor.y),
        MagnifierSide::Left => anchor.map(|r| r.left).unwrap_or(cursor.x) - bounds.left,
        MagnifierSide::Right => bounds.right - anchor.map(|r| r.right).unwrap_or(cursor.x),
        MagnifierSide::BottomRight => {
            let right = bounds.right - anchor.map(|r| r.right).unwrap_or(cursor.x);
            let bottom = bounds.bottom - anchor.map(|r| r.bottom).unwrap_or(cursor.y);
            right.min(bottom)
        }
    }
}

fn magnifier_best_side(state: &OverlayState) -> MagnifierSide {
    let order = [
        MagnifierSide::BottomRight,
        MagnifierSide::Above,
        MagnifierSide::Below,
        MagnifierSide::Right,
        MagnifierSide::Left,
    ];
    let mut best = state.magnifier_side;
    let mut best_space = -1;
    for side in order {
        if magnifier_candidate(state, side).is_some() {
            let space = magnifier_side_space(state, side);
            if space > best_space {
                best = side;
                best_space = space;
            }
        }
    }
    best
}

fn magnifier_fallback_rect(state: &OverlayState) -> Option<RECT> {
    let cursor = state.cursor?;
    let bounds = image_bounds(state)?;
    let max_x = (bounds.right - MAG_PANEL_W).max(bounds.left);
    let max_y = (bounds.bottom - MAG_PANEL_H).max(bounds.top);
    let left = (cursor.x - MAG_PANEL_W / 2).clamp(bounds.left, max_x);
    let top = (cursor.y - MAG_PANEL_H - 18).clamp(bounds.top, max_y);
    Some(RECT {
        left,
        top,
        right: left + MAG_PANEL_W,
        bottom: top + MAG_PANEL_H,
    })
}

fn magnifier_panel_rect(state: &OverlayState) -> Option<RECT> {
    let target = magnifier_candidate(state, state.magnifier_side)
        .or_else(|| magnifier_fallback_rect(state))?;
    if let Some(motion) = state.magnifier_motion.as_ref() {
        if motion.start.elapsed().as_millis() < MAGNIFIER_SLIDE_MS {
            let t = (motion.start.elapsed().as_millis() as f32 / MAGNIFIER_SLIDE_MS as f32)
                .clamp(0.0, 1.0);
            let eased = ease_out_cubic(t);
            let left = motion.from.x + ((motion.to.x - motion.from.x) as f32 * eased) as i32;
            let top = motion.from.y + ((motion.to.y - motion.from.y) as f32 * eased) as i32;
            return Some(RECT {
                left,
                top,
                right: left + MAG_PANEL_W,
                bottom: top + MAG_PANEL_H,
            });
        }
    }
    Some(target)
}

fn switch_magnifier_side(state: &mut OverlayState, side: MagnifierSide) {
    if side == state.magnifier_side {
        return;
    }
    let from = magnifier_panel_rect(state)
        .or_else(|| magnifier_candidate(state, state.magnifier_side))
        .map(|rect| POINT {
            x: rect.left,
            y: rect.top,
        });
    state.magnifier_side = side;
    state.magnifier_side_since = Instant::now();
    if let (Some(from), Some(target)) = (from, magnifier_candidate(state, side)) {
        state.magnifier_motion = Some(MagnifierMotion {
            from,
            to: POINT {
                x: target.left,
                y: target.top,
            },
            start: Instant::now(),
        });
        unsafe {
            let _ = SetTimer(state.hwnd, MAGNIFIER_TIMER_ID, 16, None);
        }
    }
}

fn update_magnifier_side(state: &mut OverlayState) {
    if state.magnifier_consumed || !current_options().magnifier_enabled {
        return;
    }

    // Prefer bottom-right. If the panel was forced elsewhere by an edge or the
    // selection, return once the preferred side has been valid for a short
    // moment; the delay prevents side-to-side flicker at boundaries.
    let preferred = MagnifierSide::BottomRight;
    if state.magnifier_side != preferred
        && state.magnifier_side_since.elapsed().as_millis() >= 160
        && magnifier_candidate(state, preferred).is_some()
    {
        switch_magnifier_side(state, preferred);
    }

    if magnifier_candidate(state, state.magnifier_side).is_none() {
        let best = magnifier_best_side(state);
        switch_magnifier_side(state, best);
    }
    if let Some(target) = magnifier_candidate(state, state.magnifier_side) {
        if let Some(motion) = state.magnifier_motion.as_mut() {
            motion.to = POINT {
                x: target.left,
                y: target.top,
            };
        }
    }
}

fn draw_magnifier(hdc: HDC, state: &OverlayState) -> Result<()> {
    if !current_options().magnifier_enabled || state.magnifier_consumed {
        return Ok(());
    }
    if selection_rect(state).is_some() && !state.dragging {
        return Ok(());
    }
    let (Some(cursor), Some(image), Some(bounds), Some(panel)) = (
        state.cursor,
        state.original.as_ref(),
        image_bounds(state),
        magnifier_panel_rect(state),
    ) else {
        return Ok(());
    };
    let source_x = (cursor.x - MAG_SOURCE / 2)
        .clamp(bounds.left, (bounds.right - MAG_SOURCE).max(bounds.left));
    let source_y = (cursor.y - MAG_SOURCE / 2)
        .clamp(bounds.top, (bounds.bottom - MAG_SOURCE).max(bounds.top));
    let Some((r, g, b)) = image.pixel_rgb(cursor.x, cursor.y) else {
        return Ok(());
    };
    let x = panel.left;
    let y = panel.top;
    let image_x = x + (MAG_PANEL_W - MAG_IMAGE_SIZE) / 2;
    let image_y = y + MAG_PAD;

    unsafe {
        crate::round_rect::draw_round_rect(
            hdc,
            x,
            y,
            x + MAG_PANEL_W,
            y + MAG_PANEL_H,
            12.0,
            Some(0xFF1C1C1E),
            Some((0xFF3A3A40, 1.0)),
        );

        let _ = SetStretchBltMode(hdc, COLORONCOLOR);
        let _ = StretchBlt(
            hdc,
            image_x,
            image_y,
            MAG_IMAGE_SIZE,
            MAG_IMAGE_SIZE,
            image.hdc(),
            source_x,
            source_y,
            MAG_SOURCE,
            MAG_SOURCE,
            SRCCOPY,
        );

        let cross = CreateSolidBrush(rgb(255, 255, 255));
        let center_x = image_x + MAG_IMAGE_SIZE / 2;
        let center_y = image_y + MAG_IMAGE_SIZE / 2;
        let _ = FillRect(
            hdc,
            &RECT {
                left: center_x - 1,
                top: image_y,
                right: center_x + 1,
                bottom: image_y + MAG_IMAGE_SIZE,
            },
            cross,
        );
        let _ = FillRect(
            hdc,
            &RECT {
                left: image_x,
                top: center_y - 1,
                right: image_x + MAG_IMAGE_SIZE,
                bottom: center_y + 1,
            },
            cross,
        );
        let _ = DeleteObject(HGDIOBJ(cross.0));

        let _ = SetBkMode(hdc, TRANSPARENT);
        let font = CreateFontW(
            -14,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            0,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft YaHei UI"),
        );
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        let _ = SetTextColor(hdc, rgb(245, 245, 248));
        let mut hex: Vec<u16> = format!("#{r:02X}{g:02X}{b:02X}")
            .encode_utf16()
            .collect();
        let mut hex_rect = RECT {
            left: x + 8,
            top: image_y + MAG_IMAGE_SIZE + 4,
            right: x + MAG_PANEL_W - 8,
            bottom: image_y + MAG_IMAGE_SIZE + 24,
        };
        let _ = DrawTextW(
            hdc,
            &mut hex,
            &mut hex_rect,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );

        let small = CreateFontW(
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
            0,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft YaHei UI"),
        );
        let _ = SelectObject(hdc, HGDIOBJ(small.0));
        let _ = SetTextColor(hdc, rgb(184, 184, 192));
        let mut rgb_text: Vec<u16> = format!("R {r}   G {g}   B {b}")
            .encode_utf16()
            .collect();
        let mut rgb_rect = RECT {
            left: x + 8,
            top: image_y + MAG_IMAGE_SIZE + 26,
            right: x + MAG_PANEL_W - 8,
            bottom: image_y + MAG_IMAGE_SIZE + 44,
        };
        let _ = DrawTextW(
            hdc,
            &mut rgb_text,
            &mut rgb_rect,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
        let mut hint: Vec<u16> = if state
            .color_copied
            .map(|at| at.elapsed() < Duration::from_millis(1200))
            .unwrap_or(false)
        {
            "已复制".encode_utf16().collect()
        } else {
            "C 复制颜色".encode_utf16().collect()
        };
        let mut hint_rect = RECT {
            left: x + 8,
            top: y + MAG_PANEL_H - 24,
            right: x + MAG_PANEL_W - 8,
            bottom: y + MAG_PANEL_H - 6,
        };
        let _ = DrawTextW(
            hdc,
            &mut hint,
            &mut hint_rect,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
        let _ = SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(small.0));
    }
    Ok(())
}

fn draw_magnifier_morph(
    hdc: HDC,
    back: &Dib,
    state: &OverlayState,
    morph: &MagnifierMorph,
) -> Result<()> {
    let rect = magnifier_morph_rect_at(morph, Instant::now());
    unsafe {
        crate::round_rect::draw_round_rect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            12.0,
            Some(0xFF1C1C1E),
            Some((0xFF3A3A40, 1.0)),
        );
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(hdc, rect.left, rect.top, rect.right, rect.bottom);
        draw_toolbar(hdc, back, state)?;
        let _ = RestoreDC(hdc, saved);
    }
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

fn rebuild_annotated(state: &mut OverlayState) -> Result<()> {
    let Some(base) = state.image.as_ref() else {
        return Ok(());
    };
    let annotated = render_strokes(base, &state.strokes, None);
    if let Some(original) = state.original.as_mut() {
        original.replace_from_rgba(&annotated)?;
    }
    state.annotated = Some(annotated);
    state.active_applied = 0;
    Ok(())
}

fn union_rect(current: Option<RECT>, next: RECT) -> RECT {
    match current {
        Some(rect) => RECT {
            left: rect.left.min(next.left),
            top: rect.top.min(next.top),
            right: rect.right.max(next.right),
            bottom: rect.bottom.max(next.bottom),
        },
        None => next,
    }
}

fn apply_brush_to_annotated(
    state: &mut OverlayState,
    tool: DrawTool,
    width: i32,
    color: COLORREF,
    alpha: u8,
    start: POINT,
    end: POINT,
) -> Result<RECT> {
    let radius = (width / 2).max(1);
    let step = if tool == DrawTool::Eraser {
        (radius / 2).max(1)
    } else {
        1
    };
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let steps = (dx.abs().max(dy.abs()).max(1) / step).max(1);
    let rgb = color_rgb(color);

    let OverlayState {
        image, annotated, ..
    } = state;
    let base = image
        .as_ref()
        .ok_or_else(|| anyhow!("no base capture"))?;
    let canvas = annotated
        .as_mut()
        .ok_or_else(|| anyhow!("no annotation canvas"))?;

    let mut dirty: Option<RECT> = None;
    for index in 0..=steps {
        let t = index as f32 / steps as f32;
        let cx = start.x + (dx as f32 * t).round() as i32;
        let cy = start.y + (dy as f32 * t).round() as i32;
        if tool == DrawTool::Pen {
            paint_brush(canvas, POINT { x: cx, y: cy }, radius, rgb, alpha);
        } else {
            erase_brush(canvas, base, cx, cy, radius);
        }
        dirty = Some(union_rect(
            dirty,
            RECT {
                left: cx - radius,
                top: cy - radius,
                right: cx + radius + 1,
                bottom: cy + radius + 1,
            },
        ));
    }
    Ok(dirty.unwrap_or_default())
}

fn apply_active_points(state: &mut OverlayState) -> Result<()> {
    let Some(stroke) = state.active_stroke.as_ref() else {
        return Ok(());
    };
    let len = stroke.points.len();
    if len <= state.active_applied {
        return Ok(());
    }
    let tool = stroke.tool;
    let width = stroke.width;
    let color = stroke.color;
    let alpha = stroke.alpha;

    let mut dirty: Option<RECT> = None;
    for index in state.active_applied..len {
        let (start, end) = {
            let stroke = state.active_stroke.as_ref().expect("active stroke checked");
            let end = stroke.points[index];
            let start = if index == 0 {
                end
            } else {
                stroke.points[index - 1]
            };
            (start, end)
        };
        let rect = apply_brush_to_annotated(state, tool, width, color, alpha, start, end)?;
        dirty = Some(union_rect(dirty, rect));
    }
    state.active_applied = len;
    if let Some(dirty) = dirty {
        if let Some(annotated) = state.annotated.as_ref() {
            if let Some(original) = state.original.as_mut() {
                original.replace_region_from_rgba(
                    annotated,
                    dirty.left,
                    dirty.top,
                    dirty.right - dirty.left,
                    dirty.bottom - dirty.top,
                )?;
            }
        }
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
        state.active_applied = 0;
        state.active_stroke = Some(Stroke {
            tool,
            color,
            alpha,
            width,
            points: vec![point],
            segments: Vec::new(),
        });
        let _ = apply_active_points(state);
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
        let _ = apply_active_points(state);
        if let Some(stroke) = state.active_stroke.take() {
            if !stroke.points.is_empty() {
                state.strokes.push(stroke);
                state.redo.clear();
            }
            state.active_applied = 0;
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
    let (image, rect) = STATE.with(|cell| {
        let borrow = cell.borrow();
        let state = borrow
            .as_ref()
            .ok_or_else(|| anyhow!("overlay is not initialized"))?;
        let image = state
            .annotated
            .as_ref()
            .or(state.image.as_ref())
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
    let radius_sq = radius * radius;
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy > radius_sq {
                continue;
            }
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
    // Large brushes overlap heavily, so stamping every pixel is wasted work and
    // makes fast drags quadratic. Space stamps by half the radius instead.
    let stamp_step = (radius / 2).max(1);
    for pair in points.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let dy = pair[1].y - pair[0].y;
        let steps = (dx.abs().max(dy.abs()).max(1) / stamp_step).max(1);
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

fn copy_image_to_clipboard(crop: &RgbaImage) -> Result<CopyStats> {
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

pub fn copy_current_selection() -> Result<CopyStats> {
    let (crop, _) = cropped_selection()?;
    copy_image_to_clipboard(&crop)
}

fn pin_and_close() {
    let confirmed = with_state(|state| state.pending_pin_confirm).unwrap_or(false);
    if !confirmed && crate::pin::is_full() {
        let _ = with_state_mut(|state| {
            state.pending_pin_confirm = true;
            Ok(())
        });
        let anchor = with_state(|state| {
            let mut window = RECT::default();
            let _ = unsafe { GetWindowRect(state.hwnd, &mut window) };
            toolbar_button_rects(state)
                .into_iter()
                .find(|(button, _)| *button == ToolbarButton::Pin)
                .map(|(_, rect)| {
                    (
                        window.left + (rect.left + rect.right) / 2,
                        window.top + rect.top,
                        window.top + rect.bottom,
                    )
                })
        })
        .ok()
        .flatten()
        .unwrap_or((0, 0, 0));
        crate::bubble::show_pin_limit(crate::pin::limit(), anchor.0, anchor.1, anchor.2);
        log::info!("pin limit reached; waiting for confirmation");
        return;
    }

    let result = (|| -> Result<()> {
        let (crop, rect) = cropped_selection()?;
        // Pinning also copies the same flattened image to the clipboard.
        let stats = copy_image_to_clipboard(&crop)?;
        let origin = with_state(|state| {
            let mut window = RECT::default();
            let _ = unsafe { GetWindowRect(state.hwnd, &mut window) };
            POINT {
                x: window.left,
                y: window.top,
            }
        })
        .unwrap_or(POINT { x: 0, y: 0 });
        crate::pin::create(
            crop,
            origin.x + rect.left + 12,
            origin.y + rect.top + 12,
        )?;
        report_current("pin", Some(&stats));
        Ok(())
    })();

    match result {
        Ok(()) => hide_overlay(),
        Err(e) => {
            let _ = with_state_mut(|state| {
                state.pending_pin_confirm = false;
                Ok(())
            });
            log::error!("pin failed: {e}");
            show_error(&format!("贴图失败：{e}"));
        }
    }
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
    crate::bubble::dismiss();
    release_cursor_clip();
    let hwnd = with_state_mut(|state| {
        let hwnd = state.hwnd;
        unsafe {
            let _ = KillTimer(hwnd, PREVIEW_TIMER_ID);
            let _ = KillTimer(hwnd, TOOLBAR_TIMER_ID);
        }
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
        state.pending_pin_confirm = false;
        state.annotated = None;
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
    crate::bubble::dismiss();
    release_cursor_clip();
    let (hwnd, prev) = with_state_mut(|state| {
        let hwnd = state.hwnd;
        let prev = state.prev_foreground;
        unsafe {
            let _ = KillTimer(hwnd, PREVIEW_TIMER_ID);
            let _ = KillTimer(hwnd, TOOLBAR_TIMER_ID);
        }
        state.preview_start = None;
        state.toolbar_motion = None;
        state.toolbar_has_appeared = false;
        state.toolbar_freeze = None;
        state.toolbar_last = None;
        state.selection_pending_start = None;
        state.color_copied = None;
        state.magnifier_morph = None;
        state.magnifier_consumed = false;
        state.magnifier_side = MagnifierSide::BottomRight;
        state.magnifier_side_since = Instant::now();
        state.magnifier_motion = None;
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
        state.pending_pin_confirm = false;
        state.annotated = None;
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

fn cursor_color() -> Option<(u8, u8, u8)> {
    with_state(|state| {
        let cursor = state.cursor?;
        let image = state.original.as_ref()?;
        image.pixel_rgb(cursor.x, cursor.y)
    })
    .ok()
    .flatten()
}

fn copy_cursor_color() {
    let Some((r, g, b)) = cursor_color() else {
        return;
    };
    let hex = format!("#{r:02X}{g:02X}{b:02X}");
    match Clipboard::new() {
        Ok(mut clipboard) => match clipboard.set_text(hex.clone()) {
            Ok(()) => {
                log::info!("copied color {hex}");
                let hwnd = with_state_mut(|state| {
                    state.color_copied = Some(Instant::now());
                    Ok(state.hwnd)
                })
                .unwrap_or(HWND(0));
                if hwnd.0 != 0 {
                    unsafe {
                        let _ = SetTimer(hwnd, COLOR_COPY_TIMER_ID, 1200, None);
                    }
                    gdi::invalidate(hwnd);
                }
            }
            Err(e) => log::error!("copy color failed: {e}"),
        },
        Err(e) => log::error!("open clipboard for color copy failed: {e}"),
    }
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

/// Chinese IMEs report VK_PROCESSKEY for letter shortcuts. Recover the physical
/// key so Ctrl+C/S/T and the color-picker C keep working with an IME active.
fn normalize_ime_key(raw: u32) -> u32 {
    if raw != 0xE5 {
        return raw;
    }
    for vk in [0x43u32, 0x53, 0x54] {
        if unsafe { GetKeyState(vk as i32) } < 0 {
            return vk;
        }
    }
    raw
}

fn active_modifiers() -> u32 {    let mut mods = 0;
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
                        draw_magnifier(hdc, state)?;
                        draw_handles(hdc, state)?;
                        draw_eraser_cursor(hdc, state)?;
                        if let Some(morph) = state.magnifier_morph.as_ref() {
                            if morph.start.elapsed().as_millis() < TOOLBAR_ANIM_MS {
                                draw_magnifier_morph(hdc, back, state, morph)?;
                            } else {
                                draw_toolbar(hdc, back, state)?;
                            }
                        } else {
                            draw_toolbar(hdc, back, state)?;
                        }
                        Ok(())
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
        WM_TIMER => {
            if wparam.0 == PREVIEW_TIMER_ID {
                let done = with_state(|state| {
                    state
                        .preview_start
                        .map(|start| start.elapsed().as_millis() >= PREVIEW_ANIM_MS)
                        .unwrap_or(true)
                })
                .unwrap_or(true);
                if done {
                    unsafe {
                        let _ = KillTimer(hwnd, PREVIEW_TIMER_ID);
                    }
                }
                gdi::invalidate(hwnd);
            } else if wparam.0 == TOOLBAR_TIMER_ID {
                let done = with_state(|state| {
                    state
                        .toolbar_motion
                        .map(|motion| motion.start.elapsed().as_millis() >= TOOLBAR_ANIM_MS)
                        .unwrap_or(true)
                })
                .unwrap_or(true);
                let dirty = with_state(toolbar_dirty_rect).ok().flatten();
                if done {
                    unsafe {
                        let _ = KillTimer(hwnd, TOOLBAR_TIMER_ID);
                    }
                    let _ = with_state_mut(|state| {
                        state.magnifier_morph = None;
                        Ok(())
                    });
                }
                invalidate_dirty(hwnd, dirty);
            } else if wparam.0 == COLOR_COPY_TIMER_ID {
                unsafe {
                    let _ = KillTimer(hwnd, COLOR_COPY_TIMER_ID);
                }
                let _ = with_state_mut(|state| {
                    state.color_copied = None;
                    Ok(())
                });
                gdi::invalidate(hwnd);
            } else if wparam.0 == HOTKEY_RETRY_TIMER_ID {
                retry_startup_hotkey(hwnd);
            } else if wparam.0 == MAGNIFIER_TIMER_ID {
                let done = with_state(|state| {
                    state
                        .magnifier_motion
                        .as_ref()
                        .map(|motion| {
                            motion.start.elapsed().as_millis() >= MAGNIFIER_SLIDE_MS
                        })
                        .unwrap_or(true)
                })
                .unwrap_or(true);
                let dirty = with_state(magnifier_dirty_rect).ok().flatten();
                if done {
                    unsafe {
                        let _ = KillTimer(hwnd, MAGNIFIER_TIMER_ID);
                    }
                    let _ = with_state_mut(|state| {
                        state.magnifier_motion = None;
                        Ok(())
                    });
                }
                invalidate_dirty(hwnd, dirty);
            } else if wparam.0 == FOCUS_RETRY_TIMER_ID {
                unsafe {
                    let _ = KillTimer(hwnd, FOCUS_RETRY_TIMER_ID);
                }
                let visible = with_state(|state| state.visible).unwrap_or(false);
                if visible && unsafe { GetForegroundWindow() } != hwnd {
                    force_foreground(hwnd);
                    unsafe {
                        let _ = KillTimer(hwnd, FOCUS_RETRY_TIMER_ID);
                    }
                }
            }
            LRESULT(0)
        }
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
            let frozen_toolbar = with_state(|state| {
                toolbar_rect(state).or(state.toolbar_last)
            })
            .ok()
            .flatten();
            let _ = SetCapture(hwnd);
            clip_cursor_to_window(hwnd);
            with_state_mut(|state| {
                if !state.visible {
                    return Ok(());
                }
                let clamped = clamp_to_image(state, pt);
                state.selection_pending_start = Some(clamped);
                state.toolbar_freeze = frozen_toolbar;
                state.toolbar_motion = None;
                state.magnifier_morph = None;
                state.cursor = Some(pt);
                state.hover_button = None;
                state.resize_drag = None;
                state.move_drag = None;
                state.active_stroke = None;
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
                    let mut applied_after_split = None;
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
                                applied_after_split = Some(1);
                            } else {
                                applied_after_split = Some(0);
                            }
                            stroke.width = next;
                        }
                    }
                    if let Some(applied) = applied_after_split {
                        state.active_applied = applied;
                    }
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
                if let Some(pending) = state.selection_pending_start {
                    let dx = pt.x - pending.x;
                    let dy = pt.y - pending.y;
                    if dx * dx + dy * dy <= 9 {
                        return Ok(());
                    }
                    state.selection_pending_start = None;
                    state.dragging = true;
                    state.start = Some(pending);
                    state.end = Some(clamp_to_image(state, pt));
                    state.active_stroke = None;
                    state.strokes.clear();
                    state.redo.clear();
                    state.pending_pin_confirm = false;
                    let _ = rebuild_annotated(state);
                    return Ok(());
                }
                if state.active_stroke.is_some() {
                    let mut changed = false;
                    if let Some(point) = clamp_to_selection(state, pt) {
                        if let Some(stroke) = state.active_stroke.as_mut() {
                            if stroke.points.last() != Some(&point) {
                                stroke.points.push(point);
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        let _ = apply_active_points(state);
                    }
                    return Ok(());
                }
                if let Some(drag) = state.resize_drag {
                    if let Some(bounds) = image_bounds(state) {
                        let next = resize_selection(drag.start_rect, drag.handle, pt, bounds);
                        set_selection(state, next);
                        sync_toolbar_motion(state);
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
                        sync_toolbar_motion(state);
                    }
                    return Ok(());
                }
                if state.dragging {
                    state.end = Some(clamp_to_image(state, pt));
                    if let (Some(freeze), Some(selection), Some(image)) = (
                        state.toolbar_freeze,
                        selection_rect(state),
                        state.image.as_ref(),
                    ) {
                        if rects_intersect(freeze, selection) {
                            let target = toolbar_outside_selection(
                                selection,
                                image.width() as i32,
                                image.height() as i32,
                                freeze,
                            );
                            if target.left != freeze.left
                                || target.top != freeze.top
                                || target.right != freeze.right
                                || target.bottom != freeze.bottom
                            {
                                let from = toolbar_rect(state).unwrap_or(freeze);
                                state.toolbar_freeze = Some(target);
                                state.toolbar_motion = Some(ToolbarMotion {
                                    from: POINT {
                                        x: from.left,
                                        y: from.top,
                                    },
                                    to: POINT {
                                        x: target.left,
                                        y: target.top,
                                    },
                                    start: Instant::now(),
                                });
                                unsafe {
                                    let _ = SetTimer(state.hwnd, TOOLBAR_TIMER_ID, 16, None);
                                }
                            }
                        }
                    }
                    return Ok(());
                }
                let previous = state.hover_button;
                let next = toolbar_button_at(state, pt);
                state.hover_button = next;
                if next == Some(ToolbarButton::Pen) && previous != Some(ToolbarButton::Pen) {
                    state.preview_start = Some(Instant::now());
                    unsafe {
                        let _ = SetTimer(state.hwnd, PREVIEW_TIMER_ID, 15, None);
                    }
                } else if previous == Some(ToolbarButton::Pen)
                    && next != Some(ToolbarButton::Pen)
                {
                    state.preview_start = None;
                    unsafe {
                        let _ = KillTimer(state.hwnd, PREVIEW_TIMER_ID);
                    }
                }
                Ok(())
            })
            .ok();
            let _ = with_state_mut(|state| {
                update_magnifier_side(state);
                Ok(())
            });
            // Always repaint on mouse movement: the crosshair guide must track
            // the cursor even when no selection/toolbar state changed.
            gdi::invalidate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = with_state_mut(|state| {
                state.mouse_up_releasing = true;
                Ok(())
            });
            let _ = ReleaseCapture();
            release_cursor_clip();
            let pending = with_state(|state| state.selection_pending_start)
                .unwrap_or(None);
            if pending.is_some() {
                let _ = with_state_mut(|state| {
                    state.selection_pending_start = None;
                    state.toolbar_freeze = None;
                    state.mouse_up_releasing = false;
                    Ok(())
                });
                gdi::invalidate(hwnd);
                update_cursor(hwnd);
                return LRESULT(0);
            }
            if finish_stroke() {
                let _ = with_state_mut(|state| {
                    state.mouse_up_releasing = false;
                    Ok(())
                });
                gdi::invalidate(hwnd);
                update_cursor(hwnd);
                return LRESULT(0);
            }
            let was_adjusting = with_state_mut(|state| {
                let adjusting = state.resize_drag.is_some() || state.move_drag.is_some();
                let frozen_visual = if state.dragging {
                    toolbar_rect(state)
                } else {
                    None
                };
                let morph_from = magnifier_panel_rect(state);
                let frozen = state.toolbar_freeze.take();
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
                if let Some(frozen) = frozen {
                    state.toolbar_has_appeared = true;
                    if let Some(target) = toolbar_target_rect(state) {
                        let from = frozen_visual.unwrap_or(frozen);
                        state.toolbar_motion = Some(ToolbarMotion {
                            from: POINT {
                                x: from.left,
                                y: from.top,
                            },
                            to: POINT {
                                x: target.left,
                                y: target.top,
                            },
                            start: Instant::now(),
                        });
                        if !adjusting
                            && !state.magnifier_consumed
                            && current_options().magnifier_enabled
                        {
                            if let Some(from) = morph_from {
                                state.magnifier_morph = Some(MagnifierMorph {
                                    from,
                                    to: target,
                                    start: Instant::now(),
                                });
                                state.magnifier_consumed = true;
                            }
                        }
                        unsafe {
                            let _ = SetTimer(state.hwnd, TOOLBAR_TIMER_ID, 16, None);
                        }
                    } else {
                        state.toolbar_motion = None;
                    }
                } else {
                    if !adjusting
                        && !state.magnifier_consumed
                        && current_options().magnifier_enabled
                    {
                        if let (Some(from), Some(target)) =
                            (morph_from, toolbar_target_rect(state))
                        {
                            state.magnifier_morph = Some(MagnifierMorph {
                                from,
                                to: target,
                                start: Instant::now(),
                            });
                            state.magnifier_consumed = true;
                        }
                    }
                    sync_toolbar_motion(state);
                }
                Ok(adjusting)
            })
            .unwrap_or(false);
            let _ = was_adjusting;
            let _ = with_state_mut(|state| {
                state.mouse_up_releasing = false;
                Ok(())
            });
            gdi::invalidate(hwnd);
            update_cursor(hwnd);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            let releasing =
                with_state(|state| state.mouse_up_releasing).unwrap_or(false);
            if releasing {
                return LRESULT(0);
            }
            release_cursor_clip();
            finish_stroke();
            with_state_mut(|state| {
                state.selection_pending_start = None;
                state.toolbar_freeze = None;
                state.dragging = false;
                state.hover_button = None;
                state.resize_drag = None;
                state.move_drag = None;
                Ok(())
            })
            .ok();
            LRESULT(0)
        }
        WM_SYSKEYDOWN => {
            // Holding Alt after the global hotkey makes Esc arrive as
            // WM_SYSKEYDOWN. Without this, Alt+X → Esc never closes the overlay.
            if wparam.0 as u32 == VK_ESCAPE.0 as u32 {
                cancel_and_close();
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_KEYDOWN => {
            let key = normalize_ime_key(wparam.0 as u32);
            let options = current_options();
            let mods = active_modifiers();
            log::debug!(
                "overlay key {key:#x} mods={mods:#x} magnifier={}",
                options.magnifier_enabled
            );
            if key == VK_ESCAPE.0 as u32 {
                cancel_and_close();
            } else if key == VK_RETURN.0 as u32 {
                copy_and_close();
            } else if options.copy_hotkey.matches(key, mods) {
                copy_and_close();
            } else if options.save_hotkey.matches(key, mods) {
                save_and_close();
            } else if options.pin_hotkey.matches(key, mods) {
                pin_and_close();
            } else if key == 0x43 && mods == 0 && options.magnifier_enabled {
                copy_cursor_color();
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
        WM_APP_PIN_LIMIT => {
            crate::pin::apply_limit();
            LRESULT(0)
        }
        WM_APP_CONFIRM_PIN => {
            let pending = with_state(|state| state.pending_pin_confirm).unwrap_or(false);
            if pending {
                pin_and_close();
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
    fn eraser_brush_is_circular() {
        let base = RgbaImage::from_pixel(12, 12, Rgba([0, 255, 0, 255]));
        let mut canvas = RgbaImage::from_pixel(12, 12, Rgba([255, 0, 0, 255]));
        erase_brush(&mut canvas, &base, 5, 5, 5);
        assert_eq!(canvas.get_pixel(5, 5), &Rgba([0, 255, 0, 255]));
        // Square-corner pixel: distance sqrt(32) > 5, so it must remain untouched.
        assert_eq!(canvas.get_pixel(9, 9), &Rgba([255, 0, 0, 255]));
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
