//! Clipboard abstraction so the overlay can be tested around a seam.

use anyhow::{anyhow, Result};
use arboard::{Clipboard, ImageData};
use image::RgbaImage;

pub trait ClipboardService: Send + Sync {
    fn copy_image(&self, image: &RgbaImage) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ArboardClipboard;

impl ClipboardService for ArboardClipboard {
    fn copy_image(&self, image: &RgbaImage) -> Result<()> {
        copy_image(image)
    }
}

pub fn copy_image(image: &RgbaImage) -> Result<()> {
    let width = usize::try_from(image.width()).map_err(|_| anyhow!("image width too large"))?;
    let height = usize::try_from(image.height()).map_err(|_| anyhow!("image height too large"))?;
    let mut clipboard = Clipboard::new().map_err(|e| anyhow!("cannot open clipboard: {e}"))?;
    clipboard
        .set_image(ImageData {
            width,
            height,
            bytes: std::borrow::Cow::Borrowed(image.as_raw()),
        })
        .map_err(|e| anyhow!("cannot write clipboard image: {e}"))?;
    Ok(())
}
