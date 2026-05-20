use crate::capture::screen::{default_capture, Rect, ScreenCapture};
use crate::error::GifError;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────

/// Maximum GIF recording duration in seconds.
const MAX_RECORDING_SECONDS: u16 = 30;
/// Milliseconds per second.
const MS_PER_SECOND: u64 = 1000;
/// GIF frame delay is expressed in hundredths of a second.
const CENTISECONDS_PER_SECOND: u16 = 100;
/// Minimum output dimension to avoid zero-size GIF frames.
const MIN_OUTPUT_SIZE: f32 = 1.0;
/// Resize filter for downscaling frames.
const RESIZE_FILTER: image::imageops::FilterType = image::imageops::FilterType::Triangle;

// ─────────────────────────────────────────────────────────────
// State / Command types
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecorderState {
    Idle,
    Recording,
    Encoding,
}

enum GifCommand {
    Stop(PathBuf),
}

// ─────────────────────────────────────────────────────────────
// GifRecorder
// ─────────────────────────────────────────────────────────────

/// GIF region recorder.
///
/// Uses the existing `screenshots` capture backend for frame acquisition
/// and the `gif` crate for encoding. Frames are written to the GIF file
/// incrementally so that memory usage stays bounded regardless of
/// recording length.
pub struct GifRecorder {
    state: Arc<Mutex<RecorderState>>,
    tx: Arc<Mutex<Option<mpsc::Sender<GifCommand>>>>,
    handle: Arc<Mutex<Option<JoinHandle<Result<PathBuf, GifError>>>>>,
}

impl Default for GifRecorder {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecorderState::Idle)),
            tx: Arc::new(Mutex::new(None)),
            handle: Arc::new(Mutex::new(None)),
        }
    }
}

impl GifRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start recording a rectangular region.
    ///
    /// # Arguments
    /// * `rect` - Screen region to capture
    /// * `fps`  - Target frame rate (5, 10, or 15)
    /// * `scale` - Output scale factor (0.5 = half size)
    /// * `quality` - Number of colors for quantization (2-256)
    pub fn start(
        &mut self,
        rect: Rect,
        fps: u8,
        scale: f32,
        quality: u8,
    ) -> Result<(), GifError> {
        let mut st = self.state.lock().map_err(|_| {
            GifError::External("Recorder state mutex poisoned".to_string())
        })?;

        if *st != RecorderState::Idle {
            return Err(GifError::AlreadyRecording);
        }

        let (tx, rx) = mpsc::channel::<GifCommand>();
        *self.tx.lock().map_err(|_| GifError::External("tx lock poisoned".to_string()))? = Some(tx);

        let state_clone = self.state.clone();
        let handle = std::thread::spawn(move || {
            record_thread(rect, fps, scale, quality, rx, state_clone)
        });
        *self.handle.lock().map_err(|_| GifError::External("handle lock poisoned".to_string()))? = Some(handle);

        *st = RecorderState::Recording;
        log::info!("GIF recording started ({} fps, {}x scale, {} colors)", fps, scale, quality);
        Ok(())
    }

    /// Stop recording and encode the GIF.
    ///
    /// Returns the path to the saved GIF file.
    pub fn stop(&mut self, output_path: PathBuf) -> Result<PathBuf, GifError> {
        let mut st = self.state.lock().map_err(|_| {
            GifError::External("Recorder state mutex poisoned".to_string())
        })?;

        if *st != RecorderState::Recording {
            return Err(GifError::NotRecording);
        }

        *st = RecorderState::Encoding;
        drop(st);

        let tx = self
            .tx
            .lock()
            .map_err(|_| GifError::External("tx lock poisoned".to_string()))?
            .take();

        if let Some(tx) = tx {
            let _ = tx.send(GifCommand::Stop(output_path));
        }

        let handle = self
            .handle
            .lock()
            .map_err(|_| GifError::External("handle lock poisoned".to_string()))?
            .take();

        if let Some(handle) = handle {
            let path = handle.join().map_err(|e| {
                GifError::External(format!("Record thread panicked: {:?}", e))
            })??;

            let mut st = self.state.lock().map_err(|_| {
                GifError::External("Recorder state mutex poisoned".to_string())
            })?;
            *st = RecorderState::Idle;
            log::info!("GIF recording stopped and saved to {:?}", path);
            Ok(path)
        } else {
            Err(GifError::NotRecording)
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Recording thread
// ─────────────────────────────────────────────────────────────

fn record_thread(
    rect: Rect,
    fps: u8,
    scale: f32,
    quality: u8,
    rx: mpsc::Receiver<GifCommand>,
    state: Arc<Mutex<RecorderState>>,
) -> Result<PathBuf, GifError> {
    let frame_duration = Duration::from_millis(MS_PER_SECOND / fps.max(1) as u64);
    let max_frames = (MAX_RECORDING_SECONDS as usize) * (fps as usize);

    let (mut encoder, temp_path) = create_gif_encoder(rect, scale)?;
    let mut frame_count = 0usize;
    let mut next_frame_time = Instant::now();

    loop {
        match check_stop_command(&rx)? {
            StopCheck::Stop(final_path) => {
                let _ = encoder.into_inner().map_err(|e| {
                    GifError::Encode(format!("Finalize GIF failed: {:?}", e))
                })?;
                std::fs::rename(&temp_path, &final_path).map_err(|e| {
                    GifError::External(format!("Rename temp file failed: {}", e))
                })?;
                return Ok(final_path);
            }
            StopCheck::Disconnected => {
                let _ = encoder.into_inner();
                let _ = std::fs::remove_file(&temp_path);
                return Ok(temp_path.clone());
            }
            StopCheck::None => {}
        }

        if frame_count >= max_frames {
            let _ = state.lock().map(|mut s| *s = RecorderState::Encoding);
            return Err(GifError::Timeout(MAX_RECORDING_SECONDS));
        }

        sleep_until(next_frame_time);
        next_frame_time += frame_duration;

        let capture_start = Instant::now();
        match capture_and_encode_frame(rect, scale, quality, fps, &mut encoder)? {
            true => frame_count += 1,
            false => log::warn!("Frame capture failed, skipping"),
        }

        // If capture overran the frame slot, skip ahead to avoid cascading delay.
        if capture_start.elapsed() > frame_duration {
            next_frame_time = Instant::now() + frame_duration;
        }
    }
}

/// Create the GIF encoder and temp file.
fn create_gif_encoder(
    rect: Rect,
    scale: f32,
) -> Result<(gif::Encoder<std::io::BufWriter<std::fs::File>>, PathBuf), GifError> {
    let temp_path = std::env::temp_dir().join(format!("jpixel-gif-{}.gif", timestamp_ms()));
    let file = std::fs::File::create(&temp_path).map_err(|e| {
        GifError::External(format!("Create temp file failed: {}", e))
    })?;
    let writer = std::io::BufWriter::new(file);

    let out_w = (rect.width as f32 * scale).max(MIN_OUTPUT_SIZE) as u16;
    let out_h = (rect.height as f32 * scale).max(MIN_OUTPUT_SIZE) as u16;

    let encoder = gif::Encoder::new(writer, out_w, out_h, &[]).map_err(|e| {
        GifError::Encode(format!("GIF encoder init failed: {}", e))
    })?;

    Ok((encoder, temp_path))
}

enum StopCheck {
    Stop(PathBuf),
    Disconnected,
    None,
}

/// Check for a stop command from the control channel.
///
/// Returns `Stop(final_path)` if stop was requested, `Disconnected` if the
/// channel closed, or `None` if no command is pending. The caller is
/// responsible for finalizing the encoder because `into_inner` consumes it.
fn check_stop_command(
    rx: &mpsc::Receiver<GifCommand>,
) -> Result<StopCheck, GifError> {
    match rx.try_recv() {
        Ok(GifCommand::Stop(final_path)) => Ok(StopCheck::Stop(final_path)),
        Err(mpsc::TryRecvError::Disconnected) => Ok(StopCheck::Disconnected),
        Err(mpsc::TryRecvError::Empty) => Ok(StopCheck::None),
    }
}

/// Sleep (without busy-waiting) until the target instant.
fn sleep_until(target: Instant) {
    let now = Instant::now();
    if target > now {
        std::thread::sleep(target - now);
    }
}

/// Capture one frame, optionally resize, quantize, and write to the encoder.
///
/// Returns `true` on success, `false` if capture failed.
fn capture_and_encode_frame(
    rect: Rect,
    scale: f32,
    quality: u8,
    fps: u8,
    encoder: &mut gif::Encoder<std::io::BufWriter<std::fs::File>>,
) -> Result<bool, GifError> {
    let image = match default_capture().capture_region(rect) {
        Ok(img) => img,
        Err(e) => {
            log::warn!("Frame capture failed: {}", e);
            return Ok(false);
        }
    };

    let frame = resize_frame_if_needed(&image, rect, scale);
    let max_colors = (quality as usize).clamp(2, 256);
    let (quantized, palette) = quantize_frame(&frame, max_colors);

    let gif_frame = gif::Frame {
        width: frame.width() as u16,
        height: frame.height() as u16,
        buffer: std::borrow::Cow::Owned(quantized),
        palette: Some(palette),
        delay: (CENTISECONDS_PER_SECOND / fps.max(1) as u16),
        ..Default::default()
    };

    encoder.write_frame(&gif_frame).map_err(|e| {
        GifError::Encode(format!("GIF encode frame failed: {}", e))
    })?;

    Ok(true)
}

/// Resize the frame if `scale` differs significantly from 1.0.
fn resize_frame_if_needed(
    image: &image::RgbaImage,
    rect: Rect,
    scale: f32,
) -> image::RgbaImage {
    if (scale - 1.0).abs() < f32::EPSILON {
        image.clone()
    } else {
        let new_w = (rect.width as f32 * scale).max(MIN_OUTPUT_SIZE) as u32;
        let new_h = (rect.height as f32 * scale).max(MIN_OUTPUT_SIZE) as u32;
        image::imageops::resize(image, new_w, new_h, RESIZE_FILTER)
    }
}

// ─────────────────────────────────────────────────────────────
// Color quantization
// ─────────────────────────────────────────────────────────────

/// Quantize an RGBA image to a fixed uniform palette.
///
/// Returns the index buffer and the flattened RGB palette.
fn quantize_frame(image: &image::RgbaImage, max_colors: usize) -> (Vec<u8>, Vec<u8>) {
    let max_colors = max_colors.min(256).max(2);
    let palette = build_uniform_palette(max_colors);
    let indices = map_pixels_to_palette(image, &palette);
    (indices, palette)
}

/// Build a uniform RGB palette with `max_colors` entries.
fn build_uniform_palette(max_colors: usize) -> Vec<u8> {
    let max_colors = max_colors.min(256).max(2);
    let palette_size = ((max_colors as f32).cbrt() as usize).clamp(2, 6);
    let mut palette = vec![0u8; max_colors * 3];
    let mut idx = 0usize;

    for r in 0..palette_size {
        for g in 0..palette_size {
            for b in 0..palette_size {
                if idx >= max_colors {
                    break;
                }
                let divisor = (palette_size - 1).max(1);
                palette[idx * 3] = (r * 255 / divisor) as u8;
                palette[idx * 3 + 1] = (g * 255 / divisor) as u8;
                palette[idx * 3 + 2] = (b * 255 / divisor) as u8;
                idx += 1;
            }
        }
    }
    palette
}

/// Map each pixel to the nearest color in the uniform palette.
fn map_pixels_to_palette(image: &image::RgbaImage, palette: &[u8]) -> Vec<u8> {
    let max_colors = palette.len() / 3;
    let palette_size = ((max_colors as f32).cbrt() as usize).clamp(2, 6);
    let mut indices = vec![0u8; image.width() as usize * image.height() as usize];

    for (i, pixel) in image.pixels().enumerate() {
        if i >= indices.len() {
            break;
        }
        let divisor = (palette_size - 1).max(1);
        let r_idx = ((pixel[0] as usize) * divisor / 255).min(palette_size - 1);
        let g_idx = ((pixel[1] as usize) * divisor / 255).min(palette_size - 1);
        let b_idx = ((pixel[2] as usize) * divisor / 255).min(palette_size - 1);
        indices[i] = (r_idx * palette_size * palette_size + g_idx * palette_size + b_idx) as u8;
    }
    indices
}

fn timestamp_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    // ── build_uniform_palette ─────────────────────────────────

    #[test]
    fn palette_length_is_colors_times_three() {
        for colors in [2, 64, 128, 256] {
            let p = build_uniform_palette(colors);
            assert_eq!(p.len(), colors * 3, "palette length mismatch for {} colors", colors);
        }
    }

    #[test]
    fn palette_clamps_to_valid_range() {
        assert_eq!(build_uniform_palette(300).len(), 256 * 3);
        assert_eq!(build_uniform_palette(1).len(), 2 * 3);
    }

    // ── map_pixels_to_palette ─────────────────────────────────

    #[test]
    fn map_solid_color_uniform_indices() {
        let palette = build_uniform_palette(128);
        let img = RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        let indices = map_pixels_to_palette(&img, &palette);
        assert_eq!(indices.len(), 16);
        let first = indices[0];
        assert!(indices.iter().all(|&i| i == first), "solid red image produced varying indices");
    }

    #[test]
    fn map_output_size_equals_pixel_count() {
        let palette = build_uniform_palette(128);
        let img = RgbaImage::new(10, 10);
        let indices = map_pixels_to_palette(&img, &palette);
        assert_eq!(indices.len(), 100);
    }

    // ── resize_frame_if_needed ────────────────────────────────

    #[test]
    fn resize_scale_one_returns_same_dimensions() {
        let img = RgbaImage::new(100, 80);
        let rect = Rect { x: 0, y: 0, width: 100, height: 80 };
        let out = resize_frame_if_needed(&img, rect, 1.0);
        assert_eq!(out.width(), 100);
        assert_eq!(out.height(), 80);
    }

    #[test]
    fn resize_half_scale_reduces_dimensions() {
        let img = RgbaImage::new(100, 80);
        let rect = Rect { x: 0, y: 0, width: 100, height: 80 };
        let out = resize_frame_if_needed(&img, rect, 0.5);
        assert_eq!(out.width(), 50);
        assert_eq!(out.height(), 40);
    }

    #[test]
    fn resize_min_output_size_enforced() {
        let img = RgbaImage::new(1, 1);
        let rect = Rect { x: 0, y: 0, width: 1, height: 1 };
        let out = resize_frame_if_needed(&img, rect, 0.1);
        assert!(out.width() >= 1, "width should be at least 1");
        assert!(out.height() >= 1, "height should be at least 1");
    }

    // ── timestamp_ms ──────────────────────────────────────────

    #[test]
    fn timestamp_is_monotonic() {
        let t1 = timestamp_ms();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let t2 = timestamp_ms();
        assert!(t2 > t1, "timestamp should increase over time");
    }

    #[test]
    fn timestamp_within_reasonable_range() {
        let ts = timestamp_ms();
        let since_2024 = 1_704_067_200_000u128; // 2024-01-01 00:00:00 UTC
        let before_2030 = 1_893_456_000_000u128; // 2030-01-01 00:00:00 UTC
        assert!(
            ts > since_2024 && ts < before_2030,
            "timestamp {} outside expected range",
            ts
        );
    }
}
