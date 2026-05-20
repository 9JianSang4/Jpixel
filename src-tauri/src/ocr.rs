use crate::error::OcrError;
use image::RgbaImage;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Abstraction over OCR engines.
pub trait OcrEngine: Send + Sync {
    /// Recognize text from an image.
    fn recognize(&self, image: &RgbaImage) -> Result<String, OcrError>;

}

/// Tesseract-based OCR implementation.
///
/// This implementation shells out to the system `tesseract` executable.
/// On Windows it first looks for `tesseract.exe` on PATH; if not found
/// it falls back to a bundled copy in the application directory.
pub struct TesseractOcr {
    executable: Option<PathBuf>,
}

impl Default for TesseractOcr {
    fn default() -> Self {
        Self::new()
    }
}

impl TesseractOcr {
    pub fn new() -> Self {
        let executable = Self::find_executable();
        if let Some(ref path) = executable {
            log::info!("Tesseract found at: {:?}", path);
        } else {
            log::warn!("Tesseract executable not found. OCR will be unavailable.");
        }
        Self { executable }
    }

    /// Attempt to locate the tesseract binary.
    fn find_executable() -> Option<PathBuf> {
        // 1. Try `tesseract` / `tesseract.exe` on PATH
        let cmd = if cfg!(windows) { "tesseract.exe" } else { "tesseract" };
        if let Ok(output) = std::process::Command::new("where")
            .arg(cmd)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout);
                let first = path.lines().next()?;
                let p = PathBuf::from(first.trim());
                if p.exists() {
                    return Some(p);
                }
            }
        }

        // 2. Try `which` on Unix-like systems
        #[cfg(not(windows))]
        if let Ok(output) = std::process::Command::new("which").arg("tesseract").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout);
                let p = PathBuf::from(path.trim());
                if p.exists() {
                    return Some(p);
                }
            }
        }

        // 3. Fallback to bundled path (relative to executable)
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let bundled = dir.join("tesseract").join(cmd);
                if bundled.exists() {
                    return Some(bundled);
                }
            }
        }

        None
    }

    /// Write image to a temporary file and run tesseract on it.
    fn run_tesseract(&self,
        image: &RgbaImage,
    ) -> Result<String, OcrError> {
        let exe = self
            .executable
            .as_ref()
            .ok_or(OcrError::NotInitialized)?;

        let temp_dir = std::env::temp_dir().join("jpixel-ocr");
        std::fs::create_dir_all(&temp_dir).map_err(|e| {
            OcrError::Recognition(format!("Temp dir creation failed: {}", e))
        })?;

        let img_path = temp_dir.join("ocr-input.png");
        let out_base = temp_dir.join("ocr-output");
        let out_path = out_base.with_extension("txt");

        image.save(&img_path).map_err(|e| {
            OcrError::Recognition(format!("Failed to write temp image: {}", e))
        })?;

        let mut cmd = std::process::Command::new(exe);
        cmd.arg(&img_path)
            .arg(&out_base)
            .arg("-l")
            .arg("chi_sim+eng")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().map_err(|e| {
            OcrError::Recognition(format!("Failed to spawn tesseract: {}", e))
        })?;

        let status = wait_child_with_timeout(&mut child, std::time::Duration::from_secs(30))
            .map_err(|e| OcrError::Recognition(e))?;

        if !status.success() {
            return Err(OcrError::Recognition(format!(
                "Tesseract exited with code {:?}",
                status.code()
            )));
        }

        let text = std::fs::read_to_string(&out_path).unwrap_or_default();

        // Cleanup temp files (best-effort)
        let _ = std::fs::remove_file(&img_path);
        let _ = std::fs::remove_file(&out_path);

        Ok(text.trim().to_string())
    }
}

impl OcrEngine for TesseractOcr {
    fn recognize(&self, image: &RgbaImage) -> Result<String, OcrError> {
        self.run_tesseract(image)
    }

}

/// Wait for a child process with a timeout using `try_wait` polling.
fn wait_child_with_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Result<std::process::ExitStatus, String> {
    let start = std::time::Instant::now();
    let poll = std::time::Duration::from_millis(100);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    return Err("Tesseract timed out after 30 seconds".to_string());
                }
                std::thread::sleep(poll);
            }
            Err(e) => return Err(format!("Failed to wait for tesseract: {}", e)),
        }
    }
}

static OCR_ENGINE: OnceLock<TesseractOcr> = OnceLock::new();

pub fn default_ocr() -> &'static TesseractOcr {
    OCR_ENGINE.get_or_init(TesseractOcr::new)
}
