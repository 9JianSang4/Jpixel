use std::path::PathBuf;
use thiserror::Error;

/// Unified application error type.
///
/// Every fallible operation in the backend should return this error
/// (or a more specific sub-error that implements `Into<JpixelError>`)
/// so that the frontend receives structured, actionable error messages.
#[derive(Error, Debug)]
pub enum JpixelError {
    #[error("屏幕捕获失败: {0}")]
    Capture(#[from] CaptureError),

    #[error("OCR 识别失败: {0}")]
    Ocr(#[from] OcrError),

    #[error("剪贴板操作失败: {0}")]
    Clipboard(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("窗口操作失败: {0}")]
    Window(String),

    #[error("无效的快捷键: {0}")]
    InvalidHotkey(String),

    #[error("图像处理失败: {0}")]
    Image(String),

    #[error("对话框被取消")]
    DialogCancelled,
}

impl serde::Serialize for JpixelError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("无法获取屏幕 ({0}, {1}): {2}")]
    ScreenNotFound(i32, i32, String),

    #[error("区域捕获失败 ({0}, {1}, {2}x{3}): {4}")]
    AreaFailed(i32, i32, u32, u32, String),
}

#[derive(Error, Debug)]
pub enum OcrError {
    #[error("OCR 引擎未初始化")]
    NotInitialized,

    #[error("识别失败: {0}")]
    Recognition(String),
}
/// Helper to map generic errors into `JpixelError::Io` with context.
pub fn io_err(path: impl Into<PathBuf>, err: impl std::fmt::Display) -> JpixelError {
    let path = path.into();
    JpixelError::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        format!("{}: {}", path.display(), err),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_err_includes_path() {
        let err = io_err("/tmp/test.png", "permission denied");
        let msg = err.to_string();
        assert!(msg.contains("/tmp/test.png"), "path missing in error: {}", msg);
        assert!(msg.contains("permission denied"), "cause missing in error: {}", msg);
    }

    #[test]
    fn io_err_accepts_different_error_types() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err = io_err("/tmp/foo", io);
        let msg = err.to_string();
        assert!(msg.contains("file missing"), "nested io error not formatted: {}", msg);
    }
}
