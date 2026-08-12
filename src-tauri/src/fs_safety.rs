use crate::errors::{AppError, AppResult};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn canonical_existing(path: impl AsRef<Path>) -> AppResult<PathBuf> {
    let path_ref = path.as_ref();
    fs::canonicalize(path_ref).map_err(|error| AppError::from(error).with_path(path_ref))
}

pub fn ensure_child_path(root: &Path, child: &Path) -> AppResult<()> {
    let root = canonical_existing(root)?;
    let child = if child.exists() {
        canonical_existing(child)?
    } else {
        canonicalize_existing_parent(child)?
    };
    if child.starts_with(&root) {
        Ok(())
    } else {
        Err(AppError::new("path_outside_vault", "目标路径不在知识库目录内").with_path(child))
    }
}

fn canonicalize_existing_parent(path: &Path) -> AppResult<PathBuf> {
    let mut missing_components = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        let file_name = cursor.file_name().ok_or_else(|| {
            AppError::new("invalid_path", "路径缺少可解析的父目录").with_path(path)
        })?;
        missing_components.push(file_name.to_os_string());
        cursor = cursor.parent().ok_or_else(|| {
            AppError::new("invalid_path", "路径缺少可解析的父目录").with_path(path)
        })?;
    }
    let mut resolved = canonical_existing(cursor)?;
    for component in missing_components.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

pub fn is_link_path(path: &Path) -> AppResult<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(AppError::from(error).with_path(path)),
    };
    if metadata.file_type().is_symlink() {
        return Ok(true);
    }
    Ok(is_windows_reparse_point(&metadata))
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

pub fn copy_path_recursive(source: &Path, destination: &Path) -> AppResult<()> {
    if is_link_path(source)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持复制链接目录或链接文件")
                .with_path(source),
        );
    }
    let metadata = fs::metadata(source).map_err(|error| AppError::from(error).with_path(source))?;
    if metadata.is_dir() {
        fs::create_dir_all(destination)
            .map_err(|error| AppError::from(error).with_path(destination))?;
        for entry in
            fs::read_dir(source).map_err(|error| AppError::from(error).with_path(source))?
        {
            let entry = entry.map_err(|error| AppError::from(error).with_path(source))?;
            let child_source = entry.path();
            let child_destination = destination.join(entry.file_name());
            copy_path_recursive(&child_source, &child_destination)?;
        }
    } else {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| AppError::from(error).with_path(parent))?;
        }
        fs::copy(source, destination).map_err(|error| AppError::from(error).with_path(source))?;
    }
    Ok(())
}

pub fn remove_path(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if is_link_path(path)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持删除链接目录或链接文件").with_path(path),
        );
    }
    let metadata = fs::metadata(path).map_err(|error| AppError::from(error).with_path(path))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|error| AppError::from(error).with_path(path))?;
    } else {
        fs::remove_file(path).map_err(|error| AppError::from(error).with_path(path))?;
    }
    Ok(())
}

pub fn replace_dir_with_stage(stage_dir: &Path, target_dir: &Path) -> AppResult<()> {
    if target_dir.exists() {
        remove_path(target_dir)?;
    }
    if let Some(parent) = target_dir.parent() {
        fs::create_dir_all(parent).map_err(|error| AppError::from(error).with_path(parent))?;
    }
    fs::rename(stage_dir, target_dir)
        .map_err(|error| AppError::from(error).with_path(target_dir))?;
    Ok(())
}
