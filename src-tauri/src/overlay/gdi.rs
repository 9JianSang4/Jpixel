//! Minimal GDI double-buffer helpers for the spike.
//!
//! The overlay is an opaque full-monitor window, so no layered-window
//! transparency is needed. We pre-compose a dimmed copy of the frozen
//! screenshot and blit the un-dimmed selection region on top.

use anyhow::{anyhow, Result};
use image::RgbaImage;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreatePen, DeleteDC, DeleteObject,
    EndPaint, GetStockObject, InvalidateRect, Rectangle, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, NULL_BRUSH, PAINTSTRUCT,
    PS_SOLID, SRCCOPY,
};

const DIM_FACTOR: f32 = 0.55;

fn blend_at(dst: &mut [u8], offset: usize, color: (u8, u8, u8), alpha: u8) {
    let a = alpha as u32;
    let inv = 255 - a;
    dst[offset] = ((color.2 as u32 * a + dst[offset] as u32 * inv) / 255) as u8;
    dst[offset + 1] = ((color.1 as u32 * a + dst[offset + 1] as u32 * inv) / 255) as u8;
    dst[offset + 2] = ((color.0 as u32 * a + dst[offset + 2] as u32 * inv) / 255) as u8;
}

fn color(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

pub struct Dib {
    hdc: HDC,
    hbitmap: HBITMAP,
    pub width: i32,
    pub height: i32,
    bits: *mut u8,
}

impl Dib {
    fn new(width: i32, height: i32) -> Result<Self> {
        if width <= 0 || height <= 0 {
            return Err(anyhow!("invalid DIB size {}x{}", width, height));
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = null_mut();
        let hbitmap = unsafe { CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)? };
        let hdc = unsafe { CreateCompatibleDC(None) };
        if hdc.0 == 0 {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(hbitmap.0));
            }
            return Err(anyhow!("CreateCompatibleDC failed"));
        }
        unsafe {
            SelectObject(hdc, HGDIOBJ(hbitmap.0));
        }
        if bits.is_null() {
            unsafe {
                let _ = DeleteDC(hdc);
                let _ = DeleteObject(HGDIOBJ(hbitmap.0));
            }
            return Err(anyhow!("CreateDIBSection returned null bits"));
        }
        Ok(Self {
            hdc,
            hbitmap,
            width,
            height,
            bits: bits as *mut u8,
        })
    }

    pub fn from_rgba(image: &RgbaImage) -> Result<Self> {
        let width = image.width() as i32;
        let height = image.height() as i32;
        let dib = Self::new(width, height)?;
        let len = (width as usize) * (height as usize) * 4;
        let dst = unsafe { std::slice::from_raw_parts_mut(dib.bits, len) };
        for (dst_px, src_px) in dst.chunks_exact_mut(4).zip(image.pixels()) {
            dst_px[0] = src_px[2];
            dst_px[1] = src_px[1];
            dst_px[2] = src_px[0];
            dst_px[3] = 255;
        }
        Ok(dib)
    }

    /// Replace the pixels of an existing DIB without reallocating GDI objects.
    pub fn replace_from_rgba(&mut self, image: &RgbaImage) -> Result<()> {
        if image.width() as i32 != self.width || image.height() as i32 != self.height {
            return Err(anyhow!("DIB size mismatch"));
        }
        let len = (self.width as usize) * (self.height as usize) * 4;
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, len) };
        for (dst_px, src_px) in dst.chunks_exact_mut(4).zip(image.pixels()) {
            dst_px[0] = src_px[2];
            dst_px[1] = src_px[1];
            dst_px[2] = src_px[0];
            dst_px[3] = 255;
        }
        Ok(())
    }

    /// Replace only a dirty rectangle of an existing DIB.
    pub fn replace_region_from_rgba(
        &mut self,
        image: &RgbaImage,
        left: i32,
        top: i32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        if image.width() as i32 != self.width || image.height() as i32 != self.height {
            return Err(anyhow!("DIB size mismatch"));
        }
        let left = left.max(0);
        let top = top.max(0);
        let right = (left + width.max(0)).min(self.width);
        let bottom = (top + height.max(0)).min(self.height);
        if right <= left || bottom <= top {
            return Ok(());
        }
        let len = (self.width as usize) * (self.height as usize) * 4;
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, len) };
        for y in top..bottom {
            for x in left..right {
                let src = image.get_pixel(x as u32, y as u32);
                let index = ((y as usize) * (self.width as usize) + x as usize) * 4;
                dst[index] = src[2];
                dst[index + 1] = src[1];
                dst[index + 2] = src[0];
                dst[index + 3] = 255;
            }
        }
        Ok(())
    }

    pub fn dimmed_from_rgba(image: &RgbaImage) -> Result<Self> {
        let width = image.width() as i32;
        let height = image.height() as i32;
        let dib = Self::new(width, height)?;
        let len = (width as usize) * (height as usize) * 4;
        let dst = unsafe { std::slice::from_raw_parts_mut(dib.bits, len) };
        for (dst_px, src_px) in dst.chunks_exact_mut(4).zip(image.pixels()) {
            dst_px[0] = (src_px[2] as f32 * DIM_FACTOR) as u8;
            dst_px[1] = (src_px[1] as f32 * DIM_FACTOR) as u8;
            dst_px[2] = (src_px[0] as f32 * DIM_FACTOR) as u8;
            dst_px[3] = 255;
        }
        Ok(dib)
    }

    pub fn blit(&self, dst: HDC, x: i32, y: i32) -> Result<()> {
        self.blit_region(dst, x, y, self.width, self.height, 0, 0)
    }

    pub fn hdc(&self) -> HDC {
        self.hdc
    }

    /// Mark every pixel fully opaque. GDI drawing leaves the alpha byte at zero
    /// on a blank DIB, which makes AlphaBlend show nothing.
    pub fn make_opaque(&mut self) {
        let len = (self.width as usize) * (self.height as usize) * 4;
        let bits = unsafe { std::slice::from_raw_parts_mut(self.bits, len) };
        for pixel in bits.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }

    /// Blend a horizontal 1px line into this BGRA DIB. Used for the soft
    /// crosshair guides; GDI pens cannot express per-pixel alpha here.
    pub fn blend_hline(&self, y: i32, x0: i32, x1: i32, color: (u8, u8, u8), alpha: u8) {
        if y < 0 || y >= self.height || x1 <= x0 {
            return;
        }
        let start = x0.max(0).min(self.width);
        let end = x1.max(0).min(self.width);
        if end <= start {
            return;
        }
        let stride = self.width as usize * 4;
        let row = y as usize * stride;
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, stride * self.height as usize) };
        for x in start..end {
            blend_at(dst, row + x as usize * 4, color, alpha);
        }
    }

    /// Blend a vertical 1px line into this BGRA DIB.
    pub fn blend_vline(&self, x: i32, y0: i32, y1: i32, color: (u8, u8, u8), alpha: u8) {
        if x < 0 || x >= self.width || y1 <= y0 {
            return;
        }
        let start = y0.max(0).min(self.height);
        let end = y1.max(0).min(self.height);
        if end <= start {
            return;
        }
        let stride = self.width as usize * 4;
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, stride * self.height as usize) };
        for y in start..end {
            blend_at(dst, y as usize * stride + x as usize * 4, color, alpha);
        }
    }

    pub fn blank(width: i32, height: i32) -> Result<Self> {
        Self::new(width, height)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn blit_region(
        &self,
        dst: HDC,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        src_x: i32,
        src_y: i32,
    ) -> Result<()> {
        if w <= 0 || h <= 0 {
            return Ok(());
        }
        let ok = unsafe { BitBlt(dst, x, y, w, h, self.hdc, src_x, src_y, SRCCOPY) };
        if ok.is_err() {
            return Err(anyhow!("BitBlt failed"));
        }
        Ok(())
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.hbitmap.0));
            let _ = DeleteDC(self.hdc);
        }
    }
}

pub fn paint_overlay<F>(
    hwnd: HWND,
    back: &Dib,
    dimmed: &Dib,
    original: &Dib,
    selection: Option<RECT>,
    draw_extra: F,
) -> Result<()>
where
    F: FnOnce(HDC) -> Result<()>,
{
    // Compose into the back buffer first: one BitBlt to the window DC avoids
    // the progressive repaint flicker caused by drawing the dim image, the
    // selection and the border directly to the window.
    dimmed.blit(back.hdc(), 0, 0)?;

    if let Some(rect) = selection {
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w > 0 && h > 0 {
            original.blit_region(back.hdc(), rect.left, rect.top, w, h, rect.left, rect.top)?;
            draw_border(back.hdc(), &rect);
        }
    }

    draw_extra(back.hdc())?;

    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    back.blit(hdc, 0, 0)?;
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
    Ok(())
}

fn draw_border(hdc: HDC, rect: &RECT) {
    unsafe {
        let pen = CreatePen(PS_SOLID, 2, color(64, 190, 255));
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
        let _ = Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

pub fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}
