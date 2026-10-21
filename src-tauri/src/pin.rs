//! Native always-on-top pin windows.
//!
//! Pins are plain Win32 popup windows owned by the overlay thread. Keeping them
//! out of WebView avoids another renderer process and lets the image be a single
//! GDI DIB.

use crate::overlay::gdi::Dib;
use anyhow::{anyhow, Result};
use image::RgbaImage;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, FrameRect, GetDC, HGDIOBJ, HALFTONE, PAINTSTRUCT, ReleaseDC,
    SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt, AC_SRC_OVER, BLENDFUNCTION,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, LoadCursorW, RegisterClassW, SetWindowLongPtrW, SetWindowPos,
    SetTimer, ShowWindow, UpdateLayeredWindow, CS_DBLCLKS, GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW,
    KillTimer, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SWP_NOSIZE, SW_SHOW, ULW_ALPHA, WM_CAPTURECHANGED, WM_DESTROY,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const PIN_CLASS: PCWSTR = w!("JpixelPin");
const PIN_BORDER: i32 = 1;
const MIN_SCALE: f32 = 0.1;
const MAX_SCALE: f32 = 4.0;
const SCALE_STEP: f32 = 0.1;
const ZOOM_MS: u64 = 140;
const ZOOM_TIMER_ID: usize = 0x62;
const OPACITY_STEP: i32 = 26;
const MIN_OPACITY: i32 = 26;

static PIN_LIMIT: AtomicUsize = AtomicUsize::new(8);

thread_local! {
    /// Creation order, oldest first. Only touched on the overlay thread.
    static PINS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

pub fn set_limit(limit: usize) {
    PIN_LIMIT.store(limit.max(1), Ordering::SeqCst);
    crate::overlay::notify_pin_limit();
}

pub fn is_full() -> bool {
    let limit = PIN_LIMIT.load(Ordering::SeqCst).max(1);
    PINS.with(|pins| pins.borrow().len() >= limit)
}

pub fn limit() -> usize {
    PIN_LIMIT.load(Ordering::SeqCst).max(1)
}

pub fn apply_limit() {
    let limit = PIN_LIMIT.load(Ordering::SeqCst).max(1);
    loop {
        let oldest = PINS.with(|pins| {
            let mut pins = pins.borrow_mut();
            if pins.len() > limit {
                Some(pins.remove(0))
            } else {
                None
            }
        });
        let Some(hwnd) = oldest else {
            break;
        };
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

struct PinState {
    image: Dib,
    width: i32,
    height: i32,
    scale: f32,
    target_scale: f32,
    zoom_from_scale: f32,
    zoom_from_left: i32,
    zoom_from_top: i32,
    zoom_source: Option<(f32, f32)>,
    zoom_start: Instant,
    opacity: u8,
    dragging: bool,
    drag_cursor: POINT,
    drag_window: POINT,
}

/// Register the pin window class once on the overlay thread.
pub fn register_class() -> Result<()> {
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(pin_wndproc),
            hInstance: HINSTANCE(module.0),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: PIN_CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(anyhow!("RegisterClassW(JpixelPin) failed"));
        }
    }
    Ok(())
}

/// Create a pin showing `image` at `(x, y)` in virtual-screen coordinates.
pub fn create(image: RgbaImage, x: i32, y: i32) -> Result<()> {
    let width = image.width().max(1) as i32;
    let height = image.height().max(1) as i32;
    let dib = Dib::from_rgba(&image)?;

    let virtual_left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let virtual_top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let virtual_right = virtual_left + unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let virtual_bottom = virtual_top + unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let window_width = width + PIN_BORDER * 2;
    let window_height = height + PIN_BORDER * 2;
    let x = x.clamp(
        virtual_left,
        (virtual_right - window_width).max(virtual_left),
    );
    let y = y.clamp(
        virtual_top,
        (virtual_bottom - window_height).max(virtual_top),
    );

    unsafe {
        let module = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            PIN_CLASS,
            w!("Jpixel Pin"),
            WS_POPUP,
            x,
            y,
            window_width,
            window_height,
            None,
            None,
            HINSTANCE(module.0),
            None,
        );
        if hwnd.0 == 0 {
            return Err(anyhow!("CreateWindowExW(JpixelPin) failed"));
        }
        let mut state = Box::new(PinState {
            image: dib,
            width,
            height,
            scale: 1.0,
            target_scale: 1.0,
            zoom_from_scale: 1.0,
            zoom_from_left: 0,
            zoom_from_top: 0,
            zoom_source: None,
            zoom_start: Instant::now(),
            opacity: 255,
            dragging: false,
            drag_cursor: POINT::default(),
            drag_window: POINT::default(),
        });
        render_pin(hwnd, &mut state, x, y, window_width, window_height);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
        PINS.with(|pins| pins.borrow_mut().push(hwnd));
        ShowWindow(hwnd, SW_SHOW);
        apply_limit();
    }
    Ok(())
}

unsafe fn pin_state(hwnd: HWND) -> Option<&'static mut PinState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

fn scaled_size(width: i32, height: i32, scale: f32) -> (i32, i32) {
    let scale = scale.clamp(MIN_SCALE, MAX_SCALE);
    (
        ((width as f32 * scale).round() as i32).max(1),
        ((height as f32 * scale).round() as i32).max(1),
    )
}

fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Atomically submit position, size and pixels. This is the key difference from
/// SetWindowPos + WM_PAINT: the compositor never sees a half-resized frame.
unsafe fn render_pin(
    hwnd: HWND,
    state: &mut PinState,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
) {
    let width = width.max(PIN_BORDER * 2 + 1);
    let height = height.max(PIN_BORDER * 2 + 1);
    let screen = GetDC(HWND(0));
    let memory = CreateCompatibleDC(screen);
    let bitmap = CreateCompatibleBitmap(screen, width, height);
    let old = SelectObject(memory, HGDIOBJ(bitmap.0));

    let client = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let background = CreateSolidBrush(rgb(24, 24, 27));
    let _ = FillRect(memory, &client, background);
    let _ = DeleteObject(HGDIOBJ(background.0));

    let draw_width = (width - PIN_BORDER * 2).max(1);
    let draw_height = (height - PIN_BORDER * 2).max(1);
    let _ = SetStretchBltMode(memory, HALFTONE);
    let _ = SetBrushOrgEx(memory, 0, 0, None);
    let _ = StretchBlt(
        memory,
        PIN_BORDER,
        PIN_BORDER,
        draw_width,
        draw_height,
        state.image.hdc(),
        0,
        0,
        state.width,
        state.height,
        windows::Win32::Graphics::Gdi::SRCCOPY,
    );
    let border = CreateSolidBrush(rgb(120, 120, 126));
    let _ = FrameRect(memory, &client, border);
    let _ = DeleteObject(HGDIOBJ(border.0));

    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: state.opacity,
        AlphaFormat: 0,
    };
    let dst = POINT { x: left, y: top };
    let size = SIZE {
        cx: width,
        cy: height,
    };
    let src = POINT { x: 0, y: 0 };
    let _ = UpdateLayeredWindow(
        hwnd,
        screen,
        Some(&dst),
        Some(&size),
        memory,
        Some(&src),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    let _ = SelectObject(memory, old);
    let _ = DeleteObject(HGDIOBJ(bitmap.0));
    let _ = DeleteDC(memory);
    let _ = ReleaseDC(HWND(0), screen);
}

unsafe fn apply_opacity(hwnd: HWND, state: &mut PinState, opacity: i32) {
    state.opacity = opacity.clamp(MIN_OPACITY, 255) as u8;
    let mut window = RECT::default();
    if GetWindowRect(hwnd, &mut window).is_ok() {
        render_pin(
            hwnd,
            state,
            window.left,
            window.top,
            window.right - window.left,
            window.bottom - window.top,
        );
    }
}

unsafe fn start_zoom(hwnd: HWND, state: &mut PinState, target: f32, anchor: Option<POINT>) {
    let mut window = RECT::default();
    if GetWindowRect(hwnd, &mut window).is_err() {
        return;
    }
    state.target_scale = target.clamp(MIN_SCALE, MAX_SCALE);
    state.zoom_from_scale = state.scale;
    state.zoom_from_left = window.left;
    state.zoom_from_top = window.top;
    state.zoom_start = Instant::now();
    state.zoom_source = anchor.and_then(|cursor| {
        if state.scale <= 0.0 {
            return None;
        }
        Some((
            ((cursor.x - window.left - PIN_BORDER) as f32 / state.scale)
                .clamp(0.0, state.width as f32),
            ((cursor.y - window.top - PIN_BORDER) as f32 / state.scale)
                .clamp(0.0, state.height as f32),
        ))
    });
    let _ = SetTimer(hwnd, ZOOM_TIMER_ID, 16, None);
}

unsafe fn zoom_frame(hwnd: HWND, state: &mut PinState) {
    let t = (state.zoom_start.elapsed().as_millis() as f32 / ZOOM_MS as f32).clamp(0.0, 1.0);
    let eased = ease_out_cubic(t);
    let next = if t >= 1.0 {
        state.target_scale
    } else {
        state.zoom_from_scale
            + (state.target_scale - state.zoom_from_scale) * eased
    };
    let (new_width, new_height) = scaled_size(state.width, state.height, next);
    let (left, top) = match state.zoom_source {
        Some((source_x, source_y)) => (
            state.zoom_from_left
                + (source_x * (state.zoom_from_scale - next)).round() as i32,
            state.zoom_from_top
                + (source_y * (state.zoom_from_scale - next)).round() as i32,
        ),
        None => (state.zoom_from_left, state.zoom_from_top),
    };
    state.scale = next;
    render_pin(
        hwnd,
        state,
        left,
        top,
        new_width + PIN_BORDER * 2,
        new_height + PIN_BORDER * 2,
    );
    if t >= 1.0 {
        state.scale = state.target_scale;
        state.zoom_source = None;
        let _ = KillTimer(hwnd, ZOOM_TIMER_ID);
    }
}

unsafe extern "system" fn pin_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER if wparam.0 == ZOOM_TIMER_ID => {
            if let Some(state) = pin_state(hwnd) {
                zoom_frame(hwnd, state);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut paint);
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            if let Some(state) = pin_state(hwnd) {
                state.target_scale = state.scale;
                state.zoom_source = None;
                let _ = KillTimer(hwnd, ZOOM_TIMER_ID);
                SetFocus(hwnd);
                SetCapture(hwnd);
                let mut cursor = POINT::default();
                let mut window = RECT::default();
                let _ = GetCursorPos(&mut cursor);
                let _ = GetWindowRect(hwnd, &mut window);
                state.dragging = true;
                state.drag_cursor = cursor;
                state.drag_window = POINT {
                    x: window.left,
                    y: window.top,
                };
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(state) = pin_state(hwnd) {
                if state.dragging {
                    let mut cursor = POINT::default();
                    if GetCursorPos(&mut cursor).is_ok() {
                        let x = state.drag_window.x + cursor.x - state.drag_cursor.x;
                        let y = state.drag_window.y + cursor.y - state.drag_cursor.y;
                        let _ = SetWindowPos(
                            hwnd,
                            HWND_TOPMOST,
                            x,
                            y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            if let Some(state) = pin_state(hwnd) {
                state.dragging = false;
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let raw_delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
            let delta = raw_delta / 120;
            if delta == 0 {
                return LRESULT(0);
            }
            if let Some(state) = pin_state(hwnd) {
                let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
                if control {
                    apply_opacity(
                        hwnd,
                        state,
                        state.opacity as i32 + delta * OPACITY_STEP,
                    );
                } else {
                    let target = (state.target_scale + delta as f32 * SCALE_STEP)
                        .clamp(MIN_SCALE, MAX_SCALE);
                    if (target - state.target_scale).abs() > f32::EPSILON {
                        let mut cursor = POINT::default();
                        let anchor = if GetCursorPos(&mut cursor).is_ok() {
                            Some(cursor)
                        } else {
                            None
                        };
                        start_zoom(hwnd, state, target, anchor);
                    }
                }
            }
            LRESULT(0)
        }
        WM_MBUTTONDOWN => {
            if let Some(state) = pin_state(hwnd) {
                apply_opacity(hwnd, state, 255);
                let _ = KillTimer(hwnd, ZOOM_TIMER_ID);
                if (state.scale - 1.0).abs() > f32::EPSILON {
                    start_zoom(hwnd, state, 1.0, None);
                } else {
                    state.target_scale = 1.0;
                    state.zoom_source = None;
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u32 == VK_ESCAPE.0 as u32 {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if let Some(state) = pin_state(hwnd) {
                state.dragging = false;
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PINS.with(|pins| {
                pins.borrow_mut().retain(|pin| *pin != hwnd);
            });
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn rgb(r: u8, g: u8, b: u8) -> windows::Win32::Foundation::COLORREF {
    windows::Win32::Foundation::COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_size_keeps_aspect_ratio() {
        assert_eq!(scaled_size(400, 200, 1.0), (400, 200));
        assert_eq!(scaled_size(400, 200, 2.0), (800, 400));
        assert_eq!(scaled_size(400, 200, 0.5), (200, 100));
    }

    #[test]
    fn scaled_size_is_clamped_and_never_zero() {
        assert_eq!(scaled_size(400, 200, 0.0), (40, 20));
        assert_eq!(scaled_size(400, 200, 99.0), (1600, 800));
        assert_eq!(scaled_size(1, 1, MIN_SCALE), (1, 1));
    }

    #[test]
    fn layered_window_accepts_constant_alpha_bitmap() {
        unsafe {
            let module = GetModuleHandleW(None).expect("module");
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                8,
                8,
                None,
                None,
                HINSTANCE(module.0),
                None,
            );
            assert!(hwnd.0 != 0, "CreateWindowExW failed");

            let screen = GetDC(HWND(0));
            let memory = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 8, 8);
            let old = SelectObject(memory, HGDIOBJ(bitmap.0));
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: 0,
            };
            let dst = POINT { x: 0, y: 0 };
            let size = SIZE { cx: 8, cy: 8 };
            let src = POINT { x: 0, y: 0 };
            let result = UpdateLayeredWindow(
                hwnd,
                screen,
                Some(&dst),
                Some(&size),
                memory,
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            assert!(result.is_ok(), "UpdateLayeredWindow failed: {result:?}");

            let _ = SelectObject(memory, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(HWND(0), screen);
            let _ = DestroyWindow(hwnd);
        }
    }
}
