//! CSV logging and process memory sampling for the spike.

use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::mem::size_of;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::GetCurrentProcess;

pub const LOG_FILE: &str = "spike-log.csv";

const HEADER: &str = "timestamp,mode,monitor,rect,scale,capture_ms,first_frame_ms,clipboard_ms,pixel_match,max_diff,working_set_mb,note";

#[derive(Debug, Clone)]
pub struct ReportRow {
    pub mode: String,
    pub monitor: String,
    pub rect: String,
    pub scale: f64,
    pub capture_ms: f64,
    pub first_frame_ms: f64,
    pub clipboard_ms: Option<f64>,
    pub pixel_match: Option<bool>,
    pub max_diff: Option<u8>,
    pub note: String,
}

impl ReportRow {
    fn format_optional_f64(value: Option<f64>) -> String {
        value.map(|v| format!("{:.2}", v)).unwrap_or_default()
    }

    fn format_optional_bool(value: Option<bool>) -> String {
        value
            .map(|v| if v { "true" } else { "false" })
            .unwrap_or("")
            .to_string()
    }

    fn format_optional_u8(value: Option<u8>) -> String {
        value.map(|v| v.to_string()).unwrap_or_default()
    }

    fn csv_line(&self) -> String {
        format!(
            "{},{},{},{},{:.3},{:.2},{:.2},{},{},{},{},{}",
            now_epoch_ms(),
            csv_escape(&self.mode),
            csv_escape(&self.monitor),
            csv_escape(&self.rect),
            self.scale,
            self.capture_ms,
            self.first_frame_ms,
            Self::format_optional_f64(self.clipboard_ms),
            Self::format_optional_bool(self.pixel_match),
            Self::format_optional_u8(self.max_diff),
            working_set_mb(),
            csv_escape(&self.note)
        )
    }
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn working_set_mb() -> u64 {
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    if ok.is_ok() {
        counters.WorkingSetSize as u64 / (1024 * 1024)
    } else {
        0
    }
}

pub fn log(row: &ReportRow) -> Result<()> {
    let path = Path::new(LOG_FILE);
    let needs_header = !path.exists()
        || std::fs::metadata(path)
            .map(|m| m.len() == 0)
            .unwrap_or(true);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", LOG_FILE))?;
    if needs_header {
        writeln!(file, "{}", HEADER).context("write CSV header")?;
    }
    let line = row.csv_line();
    writeln!(file, "{}", line).context("write CSV row")?;
    file.flush().context("flush CSV")?;
    println!(
        "[{}] {} {} capture={:.1}ms first_frame={:.1}ms clipboard={} pixel_match={} max_diff={} mem={}MB {}",
        row.mode,
        row.monitor,
        row.rect,
        row.capture_ms,
        row.first_frame_ms,
        row.clipboard_ms
            .map(|v| format!("{:.1}ms", v))
            .unwrap_or_else(|| "-".to_string()),
        row.pixel_match
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        row.max_diff
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        working_set_mb(),
        row.note
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_formatters_are_empty_when_missing() {
        assert_eq!(ReportRow::format_optional_f64(None), "");
        assert_eq!(ReportRow::format_optional_bool(None), "");
        assert_eq!(ReportRow::format_optional_u8(None), "");
    }

    #[test]
    fn optional_formatters_render_values() {
        assert_eq!(ReportRow::format_optional_f64(Some(12.345)), "12.35");
        assert_eq!(ReportRow::format_optional_bool(Some(true)), "true");
        assert_eq!(ReportRow::format_optional_u8(Some(0)), "0");
    }

    #[test]
    fn csv_escapes_commas_and_quotes() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
    }
}
