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
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, FrameRect, HGDIOBJ, HALFTONE,
    InvalidateRect, PAINTSTRUCT, SRCCOPY, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, LoadCursorW, RegisterClassW, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, CS_DBLCLKS, GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW,
    LWA_ALPHA, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SWP_NOSIZE, SW_SHOW, WM_CAPTURECHANGED, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const PIN_CLASS: PCWSTR = w!("JpixelPin");
const PIN_BORDER: i32 = 1;
const MIN_SCALE: f32 = 0.1;
const MAX_SCALE: f32 = 4.0;
const SCALE_STEP: f32 = 0.1;
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
        let state = Box::new(PinState {
            image: dib,
            width,
            height,
            scale: 1.0,
            opacity: 255,
            dragging: false,
            drag_cursor: POINT::default(),
            drag_window: POINT::default(),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
        let _ = SetLayeredWindowAttributes(hwnd, windows::Win32::Foundation::COLORREF(0), 255, LWA_ALPHA);
        PINS.with(|pins| pins.borrow_mut().push(hwnd));
        ShowWindow(hwnd, SW_SHOW);
        windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
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

fn display_size(state: &PinState) -> (i32, i32) {
    scaled_size(state.width, state.height, state.scale)
}

unsafe fn apply_opacity(hwnd: HWND, state: &mut PinState, opacity: i32) {
    state.opacity = opacity.clamp(MIN_OPACITY, 255) as u8;
    let _ = SetLayeredWindowAttributes(
        hwnd,
        windows::Win32::Foundation::COLORREF(0),
        state.opacity,
        LWA_ALPHA,
    );
}

unsafe fn resize_pin(hwnd: HWND, state: &mut PinState, scale: f32, anchor: Option<POINT>) {
    let mut window = RECT::default();
    if GetWindowRect(hwnd, &mut window).is_err() {
        return;
    }
    let old_scale = state.scale;
    let (new_width, new_height) = scaled_size(state.width, state.height, scale);
    let next_scale = scale.clamp(MIN_SCALE, MAX_SCALE);
    let (left, top) = if let Some(cursor) = anchor {
        let source_x = ((cursor.x - window.left - PIN_BORDER) as f32 / old_scale)
            .clamp(0.0, state.width as f32);
        let source_y = ((cursor.y - window.top - PIN_BORDER) as f32 / old_scale)
            .clamp(0.0, state.height as f32);
        (
            cursor.x - PIN_BORDER - (source_x * scale.clamp(MIN_SCALE, MAX_SCALE)).round() as i32,
            cursor.y - PIN_BORDER - (source_y * scale.clamp(MIN_SCALE, MAX_SCALE)).round() as i32,
        )
    } else {
        (window.left, window.top)
    };
    state.scale = next_scale;
    let _ = SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        left,
        top,
        new_width + PIN_BORDER * 2,
        new_height + PIN_BORDER * 2,
        SWP_NOACTIVATE,
    );
    let _ = InvalidateRect(hwnd, None, false);
}

unsafe extern "system" fn pin_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            if let Some(state) = pin_state(hwnd) {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let background = CreateSolidBrush(rgb(24, 24, 27));
                let _ = FillRect(hdc, &client, background);
                let _ = DeleteObject(HGDIOBJ(background.0));
                let (draw_width, draw_height) = display_size(state);
                let _ = SetStretchBltMode(hdc, HALFTONE);
                let _ = SetBrushOrgEx(hdc, 0, 0, None);
                let _ = StretchBlt(
                    hdc,
                    PIN_BORDER,
                    PIN_BORDER,
                    draw_width,
                    draw_height,
                    state.image.hdc(),
                    0,
                    0,
                    state.width,
                    state.height,
                    SRCCOPY,
                );
                let border = CreateSolidBrush(rgb(120, 120, 126));
                let _ = FrameRect(hdc, &client, border);
                let _ = DeleteObject(HGDIOBJ(border.0));
            }
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            if let Some(state) = pin_state(hwnd) {
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
                    let next = (state.scale + delta as f32 * SCALE_STEP)
                        .clamp(MIN_SCALE, MAX_SCALE);
                    if (next - state.scale).abs() > f32::EPSILON {
                        let mut cursor = POINT::default();
                        let _ = GetCursorPos(&mut cursor);
                        resize_pin(hwnd, state, next, Some(cursor));
                    }
                }
            }
            LRESULT(0)
        }
        WM_MBUTTONDOWN => {
            if let Some(state) = pin_state(hwnd) {
                apply_opacity(hwnd, state, 255);
                resize_pin(hwnd, state, 1.0, None);
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
}
