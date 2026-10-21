//! Small native toast used for the pin-limit confirmation prompt.
//!
//! It is a layered popup anchored above the tray area, so it can fade/slide
//! without involving the WebView or a notification service.

use anyhow::{anyhow, Result};
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, GetStockObject,
    InvalidateRect, RoundRect, SelectObject, SetBkMode, SetTextColor, UpdateWindow,
    CLEARTYPE_QUALITY, DEFAULT_CHARSET, DEFAULT_PITCH, DT_LEFT, DT_SINGLELINE, DT_VCENTER,
    DT_WORDBREAK, FW_BOLD, FW_NORMAL, HDC, HGDIOBJ, HFONT, NULL_PEN, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetSystemMetrics,
    GetWindowLongPtrW, KillTimer, LoadCursorW, RegisterClassW, SetLayeredWindowAttributes,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, SystemParametersInfoW,
    CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SPI_GETWORKAREA,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SW_SHOWNOACTIVATE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const BUBBLE_CLASS: PCWSTR = w!("JpixelBubble");
const BUBBLE_WIDTH: i32 = 420;
const BUBBLE_HEIGHT: i32 = 96;
const BUBBLE_FADE_MS: u128 = 200;
const BUBBLE_LIFETIME_MS: u128 = 4500;
const BUBBLE_TIMER_ID: usize = 1;

static BUBBLE_HWND: AtomicIsize = AtomicIsize::new(0);

struct BubbleState {
    title: String,
    body: String,
    x: i32,
    target_y: i32,
    started: Instant,
    title_font: HFONT,
    body_font: HFONT,
}

pub fn dismiss() {
    let hwnd = BUBBLE_HWND.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe {
            let _ = DestroyWindow(HWND(hwnd));
        }
    }
}

pub fn register_class() -> Result<()> {    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(bubble_wndproc),
            hInstance: HINSTANCE(module.0),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: BUBBLE_CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(anyhow!("RegisterClassW(JpixelBubble) failed"));
        }
    }
    Ok(())
}

pub fn show_pin_limit(limit: usize) {
    let previous = BUBBLE_HWND.load(Ordering::SeqCst);
    if previous != 0 {
        unsafe {
            let _ = DestroyWindow(HWND(previous));
        }
    }

    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let mut work = RECT::default();
        if SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut _ as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_err()
        {
            let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            work = RECT {
                left,
                top,
                right: left + GetSystemMetrics(SM_CXVIRTUALSCREEN),
                bottom: top + GetSystemMetrics(SM_CYVIRTUALSCREEN),
            };
        }
        let x = work.right - BUBBLE_WIDTH - 12;
        let target_y = work.bottom - BUBBLE_HEIGHT - 12;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | WS_EX_TOOLWINDOW
                | WS_EX_TOPMOST
                | WS_EX_NOACTIVATE
                | WS_EX_TRANSPARENT,
            BUBBLE_CLASS,
            w!("Jpixel"),
            WS_POPUP,
            x,
            target_y + 14,
            BUBBLE_WIDTH,
            BUBBLE_HEIGHT,
            None,
            None,
            HINSTANCE(module.0),
            None,
        );
        if hwnd.0 == 0 {
            return;
        }

        let title_font = CreateFontW(
            -16,
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
        let body_font = CreateFontW(
            -13,
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
        let state = Box::new(BubbleState {
            title: "贴图数量已达上限".to_string(),
            body: format!(
                "当前上限 {limit} 张。再按一次贴图快捷键，或点击托盘图标，确认关闭最早的一张并继续。"
            ),
            x,
            target_y,
            started: Instant::now(),
            title_font,
            body_font,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
        BUBBLE_HWND.store(hwnd.0, Ordering::SeqCst);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetTimer(hwnd, BUBBLE_TIMER_ID, 15, None);
        let _ = UpdateWindow(hwnd);
    }
}

unsafe fn bubble_state(hwnd: HWND) -> Option<&'static mut BubbleState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BubbleState;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

unsafe extern "system" fn bubble_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            if let Some(state) = bubble_state(hwnd) {
                paint_bubble(hwnd, hdc, state);
            }
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            if wparam.0 == BUBBLE_TIMER_ID {
                if let Some(state) = bubble_state(hwnd) {
                    let elapsed = state.started.elapsed().as_millis();
                    if elapsed >= BUBBLE_LIFETIME_MS {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    let progress = (elapsed as f32 / BUBBLE_FADE_MS as f32).min(1.0);
                    let eased = 1.0 - (1.0 - progress).powi(3);
                    let y = state.target_y + ((1.0 - eased) * 14.0) as i32;
                    let alpha = (235.0 * eased) as u8;
                    let _ = SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        state.x,
                        y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, BUBBLE_TIMER_ID);
            if BUBBLE_HWND.load(Ordering::SeqCst) == hwnd.0 {
                BUBBLE_HWND.store(0, Ordering::SeqCst);
            }
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BubbleState;
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let state = Box::from_raw(ptr);
                let _ = DeleteObject(HGDIOBJ(state.title_font.0));
                let _ = DeleteObject(HGDIOBJ(state.body_font.0));
                drop(state);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn paint_bubble(hwnd: HWND, hdc: HDC, state: &BubbleState) {
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);

    let background = CreateSolidBrush(rgb(32, 32, 36));
    let old_brush = SelectObject(hdc, HGDIOBJ(background.0));
    let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
    let _ = RoundRect(hdc, 0, 0, client.right, client.bottom, 12, 12);
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(HGDIOBJ(background.0));

    let _ = SetBkMode(hdc, TRANSPARENT);

    let old_font = SelectObject(hdc, HGDIOBJ(state.title_font.0));
    let _ = SetTextColor(hdc, rgb(245, 245, 248));
    let mut title: Vec<u16> = state.title.encode_utf16().collect();
    let mut title_rect = RECT {
        left: 18,
        top: 12,
        right: client.right - 18,
        bottom: 34,
    };
    let _ = DrawTextW(
        hdc,
        &mut title,
        &mut title_rect,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );

    let _ = SelectObject(hdc, HGDIOBJ(state.body_font.0));
    let _ = SetTextColor(hdc, rgb(184, 184, 192));
    let mut body: Vec<u16> = state.body.encode_utf16().collect();
    let mut body_rect = RECT {
        left: 18,
        top: 38,
        right: client.right - 18,
        bottom: client.bottom - 10,
    };
    let _ = DrawTextW(hdc, &mut body, &mut body_rect, DT_LEFT | DT_WORDBREAK);

    SelectObject(hdc, old_font);
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}
