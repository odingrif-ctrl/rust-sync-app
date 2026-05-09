// src/sync.rs — локальная синхронизация (pull/push) с атомарным копированием
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

//// Рекурсивно обходит папку и возвращает все файлы с их метаданными (размер, время модификации)
fn scan_directory(dir: &Path) -> Result<Vec<(PathBuf, SystemTime, u64)>> {
    let mut files = Vec::new();
    if !dir.exists() {
        fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory: {:?}", dir))?;
        return Ok(files);
    }

    // Используем walkdir вместо ручной рекурсии (надёжнее)
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.file_type().is_file() || e.file_type().is_dir())
    {
        let entry = entry.with_context(|| format!("Failed to read directory entry in {:?}", dir))?;
        let path = entry.path();
        if path.is_file() {
            let metadata = fs::metadata(&path)?;
            let modified = metadata.modified()?;
            let size = metadata.len();
            files.push((path.to_path_buf(), modified, size));
        }
    }
    Ok(files)
}

/// Копирует файл атомарно: во временный файл -> fsync -> переименование
fn atomic_copy(src: &Path, dst: &Path) -> Result<()> {
    // 1. Создаём временный файл рядом с целевым
    let temp_path = dst.with_extension("tmp");
    
    // 2. Копируем содержимое
    fs::copy(src, &temp_path)
        .with_context(|| format!("Failed to copy from {:?} to {:?}", src, temp_path))?;
    
    // 3. Принудительно сбрасываем данные на диск (атомарность)
    let temp_file = fs::File::open(&temp_path)?;
    temp_file.sync_all()?;
    
    // 4. Переименовываем (атомарная операция в большинстве файловых систем)
    fs::rename(&temp_path, dst)
        .with_context(|| format!("Failed to rename {:?} to {:?}", temp_path, dst))?;
    
    Ok(())
}

/// Синхронизирует две папки: source -> target (только новые/изменённые файлы)
/// Удаляет в target файлы, которых нет в source (если delete_orphans = true)
fn sync_direction(
    source_dir: &Path,
    target_dir: &Path,
    delete_orphans: bool,
) -> Result<()> {
    // Сканируем обе папки
    let source_files = scan_directory(source_dir)?;
    let target_files = scan_directory(target_dir)?;
    
    // Преобразуем target в карту для быстрого поиска: относительный путь -> (modified, size)
    let mut target_map = std::collections::HashMap::new();
    for (path, modified, size) in target_files {
        if let Ok(rel_path) = path.strip_prefix(target_dir) {
            target_map.insert(rel_path.to_path_buf(), (modified, size));
        }
    }
    
    // Для каждого файла в source: копируем, если новее или отсутствует
    for (src_path, src_modified, _src_size) in source_files {
        let rel_path = src_path.strip_prefix(source_dir)
            .with_context(|| format!("Failed to strip prefix from {:?}", src_path))?;
        let target_path = target_dir.join(rel_path);
        
        // Создаём целевую папку, если её нет
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let need_copy = match target_map.get(rel_path) {
            Some((target_modified, _target_size)) => {
                // Копируем, если исходник новее
                src_modified > *target_modified
            }
            None => true, // Файла нет в target
        };
        
        if need_copy {
            println!("   Copying: {:?} -> {:?}", src_path.file_name().unwrap(), rel_path);
            atomic_copy(&src_path, &target_path)?;
        }
    }
    
    // Удаляем файлы в target, которых нет в source (осиротевшие)
    if delete_orphans {
        for (rel_path, _) in target_map {
            let source_path = source_dir.join(&rel_path);
            if !source_path.exists() {
                let target_path = target_dir.join(&rel_path);
                println!("   Deleting orphan: {:?}", rel_path);
                fs::remove_file(&target_path)
                    .with_context(|| format!("Failed to delete {:?}", target_path))?;
            }
        }
    }
    
    Ok(())
}

/// Основная функция синхронизации (вызывается из watchdog)
pub fn run_sync_cycle_safe(local_path: &PathBuf, remote_path: &str, sync_mode: &str) -> Result<()> {
    let local = local_path.as_path();
    let remote = Path::new(remote_path);
    
    println!("   Local:  {:?}", local);
    println!("   Remote: {:?}", remote);
    println!("   Mode:   {}", sync_mode);
    
    match sync_mode {
        "pull" => {
            // Копируем из remote в local, удаляем orphan в local
            sync_direction(remote, local, true)?;
        }
        "push" => {
            // Копируем из local в remote, удаляем orphan в remote
            sync_direction(local, remote, true)?;
        }
        _ => anyhow::bail!("Unknown sync_mode: {}", sync_mode),
    }
    
    Ok(())
}