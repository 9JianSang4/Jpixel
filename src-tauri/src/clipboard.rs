use crate::capture::screen::ScreenCapture;
use crate::error::JpixelError;
use arboard::Clipboard;
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;

/// Shared clipboard singleton — prevents multi-thread `OpenClipboard` contention.
///
/// On Windows the system clipboard can be held by other processes, causing
/// transient `OpenClipboard` failures.  We retry up to 3 times with a 50 ms
/// back-off to ride out brief contention.
pub fn with_clipboard<T>(f: impl Fn(&mut Clipboard) -> Result<T, String>) -> Result<T, JpixelError> {
    static CLIPBOARD: OnceLock<Mutex<Clipboard>> = OnceLock::new();

    let mut last_err = String::new();
    for attempt in 0..3 {
        let mutex = CLIPBOARD.get_or_init(|| {
            Mutex::new(Clipboard::new().expect("Failed to create clipboard"))
        });
        let mut cb = mutex.lock().map_err(|e| JpixelError::Clipboard(e.to_string()))?;
        match f(&mut cb) {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e;
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
    Err(JpixelError::Clipboard(last_err))
}

/// Copy a screen region to the clipboard as an image.
#[tauri::command]
pub fn copy_region_to_clipboard(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    strokes: Option<String>,
) -> Result<(), JpixelError> {
    let t0 = std::time::Instant::now();

    crate::window::hide_capture_windows(&app);
    log::info!("[perf] hide took {:?}", t0.elapsed());

    let t1 = std::time::Instant::now();
    let mut image = crate::capture::default_capture()
        .capture_region(crate::capture::Rect { x, y, width, height })
        .map_err(JpixelError::Capture)?;
    log::info!("[perf] capture_region({}x{}) took {:?}", width, height, t1.elapsed());

    let app_close = app.clone();
    std::thread::spawn(move || {
        crate::window::close_capture_windows(app_close);
    });

    if let Some(ref s) = strokes {
        if !s.is_empty() {
            let t2 = std::time::Instant::now();
            crate::capture::composite_strokes(&mut image, s)?;
            log::info!("[perf] composite_strokes took {:?}", t2.elapsed());
        }
    }

    let t3 = std::time::Instant::now();
    let img_data = arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: std::borrow::Cow::Borrowed(image.as_raw()),
    };
    with_clipboard(|cb| {
        cb.set_image(img_data.clone()).map_err(|e| e.to_string())
    })?;
    log::info!("[perf] clipboard took {:?}, total {:?}", t3.elapsed(), t0.elapsed());

    log::info!("Region copied to clipboard (x={}, y={}, w={}, h={})", x, y, width, height);
    Ok(())
}

/// Copy plain text to the clipboard.
#[tauri::command]
pub fn copy_text_to_clipboard(text: String) -> Result<(), JpixelError> {
    with_clipboard(|cb| {
        cb.set_text(text.clone()).map_err(|e| e.to_string())
    })?;
    log::debug!("Text copied to clipboard: {}", text);
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Stress tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod stress_tests {
    use super::*;

    /// Hammer the shared clipboard singleton concurrently.
    /// Each thread writes unique text to force `OpenClipboard` / `CloseClipboard`
    /// cycling under contention.
    #[test]
    #[ignore]
    fn clipboard_concurrent_stress() {
        const THREADS: usize = 10;
        const ITER: usize = 50;

        let handles: Vec<_> = (0..THREADS)
            .map(|tid| {
                std::thread::spawn(move || {
                    for i in 0..ITER {
                        with_clipboard(|cb| {
                            cb.set_text(format!("t{}-{}", tid, i))
                                .map_err(|e| e.to_string())
                        })
                        .expect("clipboard write should succeed");
                    }
                })
            })
            .collect();

        for (i, h) in handles.into_iter().enumerate() {
            h.join().expect(&format!("thread {} panicked", i));
        }
    }

    /// Same as above but set image data (transparent 1×1) to exercise
    /// the `set_image` code path.
    #[test]
    #[ignore]
    fn clipboard_concurrent_image_stress() {
        const THREADS: usize = 8;
        const ITER: usize = 30;

        let img = arboard::ImageData {
            width: 1,
            height: 1,
            bytes: std::borrow::Cow::Borrowed(&[0, 0, 0, 0]),
        };

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let img = img.clone();
                std::thread::spawn(move || {
                    for _ in 0..ITER {
                        with_clipboard(|cb| {
                            cb.set_image(img.clone()).map_err(|e| e.to_string())
                        })
                        .expect("clipboard image write should succeed");
                    }
                })
            })
            .collect();

        for (i, h) in handles.into_iter().enumerate() {
            h.join().expect(&format!("thread {} panicked", i));
        }
    }

    /// Mixed text / image workload, simulating real usage patterns.
    #[test]
    #[ignore]
    fn clipboard_mixed_workload() {
        const THREADS: usize = 6;
        const ITER: usize = 40;

        let img = arboard::ImageData {
            width: 1,
            height: 1,
            bytes: std::borrow::Cow::Borrowed(&[255, 0, 0, 255]),
        };

        let handles: Vec<_> = (0..THREADS)
            .map(|tid| {
                let img = img.clone();
                std::thread::spawn(move || {
                    for i in 0..ITER {
                        if i % 2 == 0 {
                            with_clipboard(|cb| {
                                cb.set_text(format!("mixed-{}-{}", tid, i))
                                    .map_err(|e| e.to_string())
                            })
                            .expect("text write should succeed");
                        } else {
                            with_clipboard(|cb| {
                                cb.set_image(img.clone()).map_err(|e| e.to_string())
                            })
                            .expect("image write should succeed");
                        }
                    }
                })
            })
            .collect();

        for (i, h) in handles.into_iter().enumerate() {
            h.join().expect(&format!("thread {} panicked", i));
        }
    }
}
