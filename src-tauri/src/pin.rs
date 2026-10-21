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
    BeginPaint, BitBlt, CreateSolidBrush, DeleteObject, EndPaint, FillRect, FrameRect, HGDIOBJ,
    PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, LoadCursorW, RegisterClassW, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, CS_DBLCLKS, GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOSIZE, SW_SHOW,
    WM_CAPTURECHANGED, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const PIN_CLASS: PCWSTR = w!("JpixelPin");
const PIN_BORDER: i32 = 1;

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
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
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
            dragging: false,
            drag_cursor: POINT::default(),
            drag_window: POINT::default(),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
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
                let _ = BitBlt(
                    hdc,
                    PIN_BORDER,
                    PIN_BORDER,
                    state.width,
                    state.height,
                    state.image.hdc(),
                    0,
                    0,
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
