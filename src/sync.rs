// src/sync.rs — полная версия с delete_old_files и refetch_deleted_file
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime};
use walkdir::WalkDir;
use crate::logger::Logger;
use crate::timeout::with_timeout;
use aws_config::BehaviorVersion;
use aws_sdk_s3::{Client, primitives::ByteStream};
//use std::time::Duration;

// ---------- S3 helpers ----------
fn is_s3_path(path: &str) -> bool {
    path.starts_with("s3://")
}

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

async fn create_s3_client() -> Result<Client> {
    let config = aws_config::defaults(BehaviorVersion::latest())
        .load()
        .await;
    Ok(Client::new(&config))
}

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

async fn upload_to_s3(client: &Client, bucket: &str, key: &str, local_path: &Path, logger: &Logger) -> Result<()> {
    let body = ByteStream::from_path(local_path).await?;
    client.put_object().bucket(bucket).key(key).body(body).send().await?;
    logger.log(&format!("   Uploaded: {:?} -> {}", local_path, key));
    Ok(())
}

// ---------- локальное сканирование с учётом возраста ----------
fn scan_local_directory_with_age(
    dir: &Path,
    regex: &Regex,
    max_age_ms: u64,
    delete_old: bool,
    logger: &Logger,
) -> Result<Vec<(PathBuf, SystemTime, u64)>> {
    let mut files = Vec::new();
    if !dir.exists() {
        fs::create_dir_all(dir)?;
        return Ok(files);
    }

    let now = SystemTime::now();
    for entry in WalkDir::new(dir) {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if !regex.is_match(&file_name) {
                continue;
            }
            let metadata = fs::metadata(path)?;
            let modified = metadata.modified()?;
            let size = metadata.len();

            // проверяем возраст
            if delete_old {
                if let Ok(age) = now.duration_since(modified) {
                    if age.as_millis() > max_age_ms as u128 {
                        logger.log(&format!("   Deleting old file: {:?} (age: {} ms)", path, age.as_millis()));
                        fs::remove_file(path)?;
                        continue;
                    }
                }
            }
            files.push((path.to_path_buf(), modified, size));
            logger.log(&format!("   Local found: {:?}", path));
        }
    }
    Ok(files)
}

// ---------- S3 → локальная (pull) с учётом refetch_deleted_file ----------
async fn sync_s3_to_local(
    client: &Client,
    bucket: &str,
    prefix: &str,
    local_dir: &Path,
    regex: &Regex,
    refetch_deleted: bool,
    max_age_ms: u64,
    delete_old: bool,
    logger: &Logger,
) -> Result<()> {
    let s3_keys = list_s3_keys(client, bucket, prefix, regex, logger).await?;
    let local_files = scan_local_directory_with_age(local_dir, regex, max_age_ms, delete_old, logger)?;

    // множество существующих локальных относительных путей
    let mut existing_local = HashSet::new();
    for (path, _, _) in &local_files {
        if let Ok(rel) = path.strip_prefix(local_dir) {
            existing_local.insert(rel.to_path_buf());
        }
    }

    let mut downloaded = HashSet::new();

    for key in &s3_keys {
        let rel_path = Path::new(key).strip_prefix(prefix).unwrap_or(Path::new(key));
        let local_path = local_dir.join(rel_path);
        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let exists_locally = existing_local.contains(rel_path);
        let already_downloaded = downloaded.contains(key);

        let should_download = if refetch_deleted {
            // режим "перекачивать удалённые": качаем, если файла нет локально И он ещё не скачан в этом цикле
            !exists_locally && !already_downloaded
        } else {
            // режим "не перекачивать": качаем только если файл уже существует локально
            exists_locally
        };

        if should_download {
            logger.log(&format!("   Downloading: {} -> {:?}", key, rel_path));
            download_from_s3(client, bucket, key, &local_path, logger).await?;
            downloaded.insert(key.clone());
        } else {
            logger.log(&format!("   Skipping (refetch_deleted_file={}): {}", refetch_deleted, key));
        }
    }

    // orphan deletion: удаляем локальные файлы, которых нет в S3
    for (path, _, _) in local_files {
        if let Ok(rel) = path.strip_prefix(local_dir) {
            let s3_key = if prefix.is_empty() {
                rel.to_string_lossy().to_string()
            } else {
                format!("{}/{}", prefix, rel.display())
            };
            if !s3_keys.contains(&s3_key) {
                logger.log(&format!("   Deleting local orphan: {:?}", rel));
                let _ = fs::remove_file(&path);
            }
        }
    }

    Ok(())
}

// ---------- локальная → S3 (push) с поддержкой delete_old_files ----------
async fn sync_local_to_s3(
    client: &Client,
    bucket: &str,
    prefix: &str,
    local_dir: &Path,
    regex: &Regex,
    max_age_ms: u64,
    delete_old: bool,
    logger: &Logger,
) -> Result<()> {
    let local_files = scan_local_directory_with_age(local_dir, regex, max_age_ms, delete_old, logger)?;
    let s3_keys = list_s3_keys(client, bucket, prefix, regex, logger).await?;

    for (local_path, _, _) in &local_files {
        let rel_path = local_path.strip_prefix(local_dir)?;
        let s3_key = if prefix.is_empty() {
            rel_path.to_string_lossy().to_string()
        } else {
            format!("{}/{}", prefix, rel_path.display())
        };
        logger.log(&format!("   Uploading: {:?} -> {}", rel_path, s3_key));
        upload_to_s3(client, bucket, &s3_key, local_path, logger).await?;
    }

    // orphan deletion в S3 (удаляем объекты, которых нет локально)
    for s3_key in s3_keys {
        let rel_path = Path::new(&s3_key).strip_prefix(prefix).unwrap_or(Path::new(&s3_key));
        let local_path = local_dir.join(rel_path);
        if !local_path.exists() {
            logger.log(&format!("   Deleting S3 orphan: {}", s3_key));
            let _ = client.delete_object().bucket(bucket).key(&s3_key).send().await;
        }
    }

    Ok(())
}

// ---------- главная точка входа ----------
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
    logger.log(&format!("   delete_old_files: {}", config.delete_old_files));
    logger.log(&format!("   max_file_age_ms: {}", config.max_file_age_ms));
    logger.log(&format!("   refetch_deleted_file: {}", config.refetch_deleted_file));
    logger.log(&format!("   timeout_ms: {}", config.timeout_ms));

    if is_s3_path(remote_path) {
        let (bucket, prefix) = parse_s3_path(remote_path)?;
        logger.log(&format!("   S3 bucket: {}, prefix: {}", bucket, prefix));

        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let client = create_s3_client().await?;

            match sync_mode {
                "pull" => {
                    with_timeout(config.timeout_ms, || async {
                        sync_s3_to_local(
                            &client,
                            &bucket,
                            &prefix,
                            local_path,
                            &regex,
                            config.refetch_deleted_file,
                            config.max_file_age_ms,
                            config.delete_old_files,
                            logger,
                        )
                        .await
                    })
                    .await
                }
                "push" => {
                    with_timeout(config.timeout_ms, || async {
                        sync_local_to_s3(
                            &client,
                            &bucket,
                            &prefix,
                            local_path,
                            &regex,
                            config.max_file_age_ms,
                            config.delete_old_files,
                            logger,
                        )
                        .await
                    })
                    .await
                }
                _ => anyhow::bail!("Unknown sync_mode: {}", sync_mode),
            }
        })
    } else {
        logger.log("   Local sync mode (not implemented in this version)");
        Ok(())
    }
}