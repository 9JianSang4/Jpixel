//! Anti-aliased rounded-rectangle drawing through GDI+.
//!
//! GDI's RoundRect is pixel aligned and jagged at small radii. These small
//! native popups use GDI+ paths for their card/border so they stay visually
//! consistent with the rest of the UI.

use std::ptr::null_mut;
use std::sync::Once;
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::GdiPlus::{
    GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1,
    GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen,
    GdipDrawPath, GdipFillPath, GdipSetSmoothingMode, GdiplusStartup, GdiplusStartupInput,
    FillModeAlternate, GpBrush, GpGraphics, GpPath, GpPen, GpSolidFill, SmoothingModeAntiAlias,
    UnitPixel,
};

static STARTUP: Once = Once::new();

fn ensure_started() {
    STARTUP.call_once(|| unsafe {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0usize;
        let _ = GdiplusStartup(&mut token, &input, null_mut());
    });
}

/// Draw a rounded rectangle. `fill`/`border` are 0xAARRGGBB values.
pub fn draw_round_rect(
    hdc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: f32,
    fill: Option<u32>,
    border: Option<(u32, f32)>,
) {
    ensure_started();
    unsafe {
        let mut graphics: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(hdc, &mut graphics).0 != 0 || graphics.is_null() {
            return;
        }
        let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);

        let mut path: *mut GpPath = null_mut();
        if GdipCreatePath(FillModeAlternate, &mut path).0 != 0 || path.is_null() {
            let _ = GdipDeleteGraphics(graphics);
            return;
        }
        let d = (radius * 2.0).max(0.0);
        let l = left as f32;
        let t = top as f32;
        let r = right as f32;
        let b = bottom as f32;
        let _ = GdipAddPathArc(path, l, t, d, d, 180.0, 90.0);
        let _ = GdipAddPathArc(path, r - d, t, d, d, 270.0, 90.0);
        let _ = GdipAddPathArc(path, r - d, b - d, d, d, 0.0, 90.0);
        let _ = GdipAddPathArc(path, l, b - d, d, d, 90.0, 90.0);
        let _ = GdipClosePathFigure(path);

        if let Some(color) = fill {
            let mut brush: *mut GpSolidFill = null_mut();
            if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
                let _ = GdipFillPath(graphics, brush as *mut GpBrush, path);
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
        }
        if let Some((color, width)) = border {
            let mut pen: *mut GpPen = null_mut();
            if GdipCreatePen1(color, width, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
                let _ = GdipDrawPath(graphics, pen, path);
                let _ = GdipDeletePen(pen);
            }
        }

        let _ = GdipDeletePath(path);
        let _ = GdipDeleteGraphics(graphics);
    }
}
