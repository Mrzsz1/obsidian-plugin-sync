use crate::errors::{AppError, AppResult};
use std::process::Command;

pub fn obsidian_is_running() -> AppResult<bool> {
    let output = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq Obsidian.exe", "/FO", "CSV", "/NH"])
        .output()
        .map_err(|error| {
            AppError::from(error).with_details("无法执行 tasklist 检测 Obsidian.exe")
        })?;

    if !output.status.success() {
        return Err(
            AppError::new("process_check_failed", "检测 Obsidian 进程失败")
                .with_details(String::from_utf8_lossy(&output.stderr).to_string()),
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.to_ascii_lowercase().contains("obsidian.exe"))
}
