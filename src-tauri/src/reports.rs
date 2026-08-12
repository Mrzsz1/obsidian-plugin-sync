use crate::{
    errors::{AppError, AppResult},
    models::{OperationStatus, SyncSummary},
};
use std::{fs, path::Path};

pub fn write_sync_reports(
    backup_dir: &Path,
    summary: &SyncSummary,
    json_name: &str,
    markdown_name: &str,
) -> AppResult<()> {
    fs::create_dir_all(backup_dir).map_err(|error| AppError::from(error).with_path(backup_dir))?;
    let json_path = backup_dir.join(json_name);
    let markdown_path = backup_dir.join(markdown_name);
    let json = serde_json::to_string_pretty(summary)
        .map_err(|error| AppError::from(error).with_path(&json_path))?;
    fs::write(&json_path, json).map_err(|error| AppError::from(error).with_path(&json_path))?;
    fs::write(&markdown_path, render_markdown(summary))
        .map_err(|error| AppError::from(error).with_path(&markdown_path))?;
    Ok(())
}

fn render_markdown(summary: &SyncSummary) -> String {
    let success = summary
        .results
        .iter()
        .filter(|result| matches!(result.status, OperationStatus::Success))
        .count();
    let skipped = summary
        .results
        .iter()
        .filter(|result| matches!(result.status, OperationStatus::Skipped))
        .count();
    let failed = summary
        .results
        .iter()
        .filter(|result| matches!(result.status, OperationStatus::Failed))
        .count();

    let mut output = String::new();
    output.push_str("# Obsidian 插件同步报告\n\n");
    output.push_str(&format!("- 开始时间：{}\n", summary.started_at));
    output.push_str(&format!("- 结束时间：{}\n", summary.finished_at));
    if !summary.app_version.is_empty() {
        output.push_str(&format!("- 应用版本：{}\n", summary.app_version));
    }
    if let Some(source) = &summary.source_vault_path {
        output.push_str(&format!("- 源知识库：{}\n", source));
    }
    output.push_str(&format!(
        "- 目标知识库数量：{}\n",
        summary.target_vault_paths.len()
    ));
    output.push_str(&format!(
        "- 成功：{}，跳过：{}，失败：{}\n\n",
        success, skipped, failed
    ));

    output.push_str("## 备份目录\n\n");
    for backup in &summary.backup_paths {
        output.push_str(&format!("- `{}`\n", backup));
    }
    if summary.backup_paths.is_empty() {
        output.push_str("- 无\n");
    }

    output.push_str("\n## 操作明细\n\n");
    for result in &summary.results {
        output.push_str(&format!(
            "- [{}] {} / {}：{}",
            status_label(&result.status),
            result.target_vault_path,
            result.plugin_id.as_deref().unwrap_or("全局"),
            result.message
        ));
        if let Some(path) = &result.path {
            output.push_str(&format!(" (`{}`)", path));
        }
        output.push('\n');
    }

    output
}

fn status_label(status: &OperationStatus) -> &'static str {
    match status {
        OperationStatus::Success => "成功",
        OperationStatus::Skipped => "跳过",
        OperationStatus::Failed => "失败",
    }
}
