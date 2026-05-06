// src/config.rs
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::fs;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub endpoint: Option<String>,
    pub token: Option<String>,
    pub remote_path: String,
    pub local_path: PathBuf,
    pub log_path: PathBuf,
    pub max_file_age_ms: u64,
    pub delete_old_files: bool,
    pub sync_mode: String,
    pub polling_interval_ms: u64,
    pub timeout_ms: u64,
    pub match_regex: String,
    pub refetch_deleted_file: bool,
    pub version: u32,
}

impl Config {
    /// Загружает конфиг из файла config.toml, который лежит рядом с исполняемым файлом
    pub fn from_file() -> Result<Self> {
        let exe_path = std::env::current_exe()
            .context("Failed to get executable path")?;
        
        let config_path = exe_path
            .parent()
            .context("Executable has no parent directory")?
            .join("config.toml");

        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file at {:?}", config_path))?;

        let config: Config = toml::from_str(&contents)
            .context("Failed to parse config file (invalid TOML)")?;

        Ok(config)
    }
}