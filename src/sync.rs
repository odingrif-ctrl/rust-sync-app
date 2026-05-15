// src/sync.rs — упрощённая S3 синхронизация (без сравнения по дате)
use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use crate::logger::Logger;

// S3 dependencies
use aws_config::BehaviorVersion;
use aws_sdk_s3::{Client, primitives::ByteStream};

/// Проверка, является ли путь S3
fn is_s3_path(path: &str) -> bool {
    path.starts_with("s3://")
}

/// Парсит bucket и prefix из s3://bucket-name/path/to/folder
fn parse_s3_path(path: &str) -> Result<(String, String)> {
    let without_prefix = path.strip_prefix("s3://")
        .context("Invalid S3 path format")?;
    let parts: Vec<&str> = without_prefix.splitn(2, '/').collect();
    if parts.is_empty() {
        anyhow::bail!("Invalid S3 path: missing bucket name");
    }
    let bucket = parts[0].to_string();
    let prefix = if parts.len() > 1 { parts[1].to_string() } else { String::new() };
    Ok((bucket, prefix))
}

/// Создаёт S3 клиента (один раз на цикл)
async fn create_s3_client() -> Result<Client> {
    let config = aws_config::defaults(BehaviorVersion::latest())
        .load()
        .await;
    Ok(Client::new(&config))
}

/// Получает список ключей объектов из S3 (без метаданных)
async fn list_s3_keys(client: &Client, bucket: &str, prefix: &str, regex: &Regex, logger: &Logger) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    let mut continuation_token = None;
    
    loop {
        let mut request = client.list_objects_v2().bucket(bucket).prefix(prefix);
        if let Some(token) = continuation_token {
            request = request.continuation_token(token);
        }
        
        let response = request.send().await?;
        
        for obj in response.contents() {
            let key = obj.key().unwrap_or_default();
            let file_name = key.split('/').last().unwrap_or("");
            if regex.is_match(file_name) {
                keys.push(key.to_string());
                logger.log(&format!("   S3 found: {}", key));
            }
        }
        
        continuation_token = response.next_continuation_token().map(String::from);
        if continuation_token.is_none() {
            break;
        }
    }
    
    Ok(keys)
}

/// Скачивает файл из S3 атомарно
async fn download_from_s3(client: &Client, bucket: &str, key: &str, local_path: &Path, logger: &Logger) -> Result<()> {
    let temp_path = local_path.with_extension("tmp");
    let response = client.get_object().bucket(bucket).key(key).send().await?;
    let data = response.body.collect().await?.into_bytes();
    fs::write(&temp_path, &data)?;
    let temp_file = fs::File::open(&temp_path)?;
    temp_file.sync_all()?;
    fs::rename(&temp_path, local_path)?;
    logger.log(&format!("   Downloaded: {} -> {:?}", key, local_path));
    Ok(())
}

/// Загружает файл в S3 атомарно
async fn upload_to_s3(client: &Client, bucket: &str, key: &str, local_path: &Path, logger: &Logger) -> Result<()> {
    let body = ByteStream::from_path(local_path).await?;
    client.put_object().bucket(bucket).key(key).body(body).send().await?;
    logger.log(&format!("   Uploaded: {:?} -> {}", local_path, key));
    Ok(())
}

/// Сканирует локальную папку (рекурсивно, с учётом regex)
fn scan_local_keys(dir: &Path, regex: &Regex, logger: &Logger) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        fs::create_dir_all(dir)?;
        return Ok(files);
    }
    
    for entry in WalkDir::new(dir) {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if regex.is_match(&file_name) {
                files.push(path.to_path_buf());
                logger.log(&format!("   Local found: {:?}", path));
            }
        }
    }
    Ok(files)
}

/// Синхронизация S3 -> локальная папка (pull) — простая версия
async fn sync_s3_to_local(
    client: &Client,
    bucket: &str,
    prefix: &str,
    local_dir: &Path,
    regex: &Regex,
    logger: &Logger,
) -> Result<()> {
    let s3_keys = list_s3_keys(client, bucket, prefix, regex, logger).await?;
    let local_keys = scan_local_keys(local_dir, regex, logger)?;
    
    // Карта локальных файлов (относительный путь)
    let local_set: std::collections::HashSet<PathBuf> = local_keys
        .iter()
        .filter_map(|p| p.strip_prefix(local_dir).ok().map(|p| p.to_path_buf()))
        .collect();
    
    // Скачиваем файлы из S3
    for key in &s3_keys {
        let rel_path = Path::new(key).strip_prefix(prefix).unwrap_or(Path::new(key));
        let local_path = local_dir.join(rel_path);
        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Если файл уже существует локально, пропускаем (для простоты)
        if !local_path.exists() {
            logger.log(&format!("   Downloading: {} -> {:?}", key, rel_path));
            download_from_s3(client, bucket, key, &local_path, logger).await?;
        }
    }
    
    // Удаляем локальные файлы, которых нет в S3
    for rel_path in local_set {
        let s3_key = if prefix.is_empty() {
            rel_path.to_string_lossy().to_string()
        } else {
            format!("{}/{}", prefix, rel_path.display())
        };
        if !s3_keys.contains(&s3_key) {
            let local_path = local_dir.join(&rel_path);
            logger.log(&format!("   Deleting local orphan: {:?}", rel_path));
            let _ = fs::remove_file(&local_path);
        }
    }
    
    Ok(())
}

/// Синхронизация локальная папка -> S3 (push) — простая версия
async fn sync_local_to_s3(
    client: &Client,
    bucket: &str,
    prefix: &str,
    local_dir: &Path,
    regex: &Regex,
    logger: &Logger,
) -> Result<()> {
    let local_keys = scan_local_keys(local_dir, regex, logger)?;
    let s3_keys = list_s3_keys(client, bucket, prefix, regex, logger).await?;
    
    let s3_set: std::collections::HashSet<String> = s3_keys.into_iter().collect();
    
    for local_path in &local_keys {
        let rel_path = local_path.strip_prefix(local_dir)?;
        let s3_key = if prefix.is_empty() {
            rel_path.to_string_lossy().to_string()
        } else {
            format!("{}/{}", prefix, rel_path.display())
        };
        
        if !s3_set.contains(&s3_key) {
            logger.log(&format!("   Uploading: {:?} -> {}", rel_path, s3_key));
            upload_to_s3(client, bucket, &s3_key, local_path, logger).await?;
        }
    }
    
    // Удаляем из S3 объекты, которых нет локально
    for s3_key in s3_set {
        let rel_path = Path::new(&s3_key).strip_prefix(prefix).unwrap_or(Path::new(&s3_key));
        let local_path = local_dir.join(rel_path);
        if !local_path.exists() {
            logger.log(&format!("   Deleting S3 orphan: {}", s3_key));
            let _ = client.delete_object().bucket(bucket).key(&s3_key).send().await;
        }
    }
    
    Ok(())
}

/// Основная публичная функция (вызывается из watchdog)
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
    
    if is_s3_path(remote_path) {
        let (bucket, prefix) = parse_s3_path(remote_path)?;
        logger.log(&format!("   S3 bucket: {}, prefix: {}", bucket, prefix));
        
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let client = create_s3_client().await?;
            match sync_mode {
                "pull" => sync_s3_to_local(&client, &bucket, &prefix, local_path, &regex, logger).await,
                "push" => sync_local_to_s3(&client, &bucket, &prefix, local_path, &regex, logger).await,
                _ => anyhow::bail!("Unknown sync_mode: {}", sync_mode),
            }
        })?;
    } else {
        logger.log("   Local sync mode (placeholder)");
    }
    
    Ok(())
}