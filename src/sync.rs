// src/sync.rs
use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;
use crate::logger::Logger;

/// Рекурсивно обходит папку и возвращает все файлы, подходящие под regex
fn scan_directory(dir: &Path, regex: &Regex, logger: &Logger) -> Result<Vec<(PathBuf, SystemTime, u64)>> {
    let mut files = Vec::new();
    if !dir.exists() {
        fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory: {:?}", dir))?;
        return Ok(files);
    }

    for entry in WalkDir::new(dir) {
        let entry = entry.with_context(|| format!("Failed to read entry in {:?}", dir))?;
        let path = entry.path();
        if path.is_file() {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if !regex.is_match(&file_name) {
                continue;
            }
            let metadata = fs::metadata(&path)?;
            let modified = metadata.modified()?;
            let size = metadata.len();
            files.push((path.to_path_buf(), modified, size));
            logger.log(&format!("   Found file: {:?}", path));
        }
    }
    Ok(files)
}

/// Атомарное копирование с fsync
fn atomic_copy(src: &Path, dst: &Path, logger: &Logger) -> Result<()> {
    let temp_path = dst.with_extension("tmp");
    fs::copy(src, &temp_path)
        .with_context(|| format!("Failed to copy from {:?} to {:?}", src, temp_path))?;
    let temp_file = fs::File::open(&temp_path)?;
    temp_file.sync_all()?;
    fs::rename(&temp_path, dst)
        .with_context(|| format!("Failed to rename {:?} to {:?}", temp_path, dst))?;
    logger.log(&format!("   Atomic copy: {:?} -> {:?}", src.file_name().unwrap(), dst.file_name().unwrap()));
    Ok(())
}

/// Удаляет старые файлы в target_dir, если включено
fn delete_old_files_in_dir(target_dir: &Path, max_age_ms: u64, delete_old: bool, logger: &Logger) -> Result<()> {
    if !delete_old {
        return Ok(());
    }
    let now = SystemTime::now();
    for entry in WalkDir::new(target_dir) {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let metadata = fs::metadata(path)?;
            if let Ok(modified) = metadata.modified() {
                if let Ok(age) = now.duration_since(modified) {
                    if age.as_millis() > max_age_ms as u128 {
                        logger.log(&format!("   Deleting old file: {:?} (age: {} ms)", path, age.as_millis()));
                        fs::remove_file(path)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Синхронизирует source_dir -> target_dir
fn sync_direction(
    source_dir: &Path,
    target_dir: &Path,
    delete_orphans: bool,
    regex: &Regex,
    max_age_ms: u64,
    delete_old: bool,
    logger: &Logger,
) -> Result<()> {
    logger.log(&format!("   Scanning source: {:?}", source_dir));
    let source_files = scan_directory(source_dir, regex, logger)?;
    let target_files = scan_directory(target_dir, regex, logger)?;
    
    let mut target_map = std::collections::HashMap::new();
    for (path, modified, _) in target_files {
        if let Ok(rel_path) = path.strip_prefix(target_dir) {
            target_map.insert(rel_path.to_path_buf(), modified);
        }
    }
    
    // Копируем новые/изменённые
    for (src_path, src_modified, _) in source_files {
        let rel_path = src_path.strip_prefix(source_dir)?;
        let target_path = target_dir.join(rel_path);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let need_copy = match target_map.get(rel_path) {
            Some(target_modified) => src_modified > *target_modified,
            None => true,
        };
        if need_copy {
            logger.log(&format!("   Copying: {:?} -> {:?}", src_path.file_name().unwrap(), rel_path));
            atomic_copy(&src_path, &target_path, logger)?;
        }
    }
    
    // Удаляем осиротевшие файлы в target
    if delete_orphans {
        for (rel_path, _) in target_map {
            let source_path = source_dir.join(&rel_path);
            if !source_path.exists() {
                let target_path = target_dir.join(&rel_path);
                logger.log(&format!("   Deleting orphan: {:?}", rel_path));
                fs::remove_file(&target_path)?;
            }
        }
    }
    
    // Удаляем старые файлы в target_dir
    delete_old_files_in_dir(target_dir, max_age_ms, delete_old, logger)?;
    
    Ok(())
}

/// Основная функция синхронизации (вызывается из watchdog)
pub fn run_sync_cycle_safe(
    local_path: &PathBuf,
    remote_path: &str,
    sync_mode: &str,
    logger: &Logger,
) -> Result<()> {
    let config = crate::config::Config::from_file()?;
    let regex = Regex::new(&config.match_regex)?;
    
    logger.log(&format!("   Local: {:?}", local_path));
    logger.log(&format!("   Remote: {:?}", remote_path));
    logger.log(&format!("   Mode: {}", sync_mode));
    logger.log(&format!("   Regex: {}", config.match_regex));
    
    match sync_mode {
        "pull" => sync_direction(
            Path::new(remote_path),
            local_path,
            true,
            &regex,
            config.max_file_age_ms,
            config.delete_old_files,
            logger,
        )?,
        "push" => sync_direction(
            local_path,
            Path::new(remote_path),
            true,
            &regex,
            config.max_file_age_ms,
            config.delete_old_files,
            logger,
        )?,
        _ => anyhow::bail!("Unknown sync_mode: {}", sync_mode),
    }
    
    Ok(())
}