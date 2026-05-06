// config.rs - Чтение конфигурации из TOML файла

use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::fs;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub endpoint: Option<String>,      // URL S3 (опционально)
    pub token: Option<String>,         // Токен для S3 (опционально)
    pub remote_path: String,           // Путь к удалённому хранилищу
    pub local_path: PathBuf,           // Локальный путь
    pub log_path: PathBuf,             // Путь к файлу логов
    pub max_file_age_ms: u64,          // Максимальный возраст файла в мс
    pub delete_old_files: bool,        // Удалять старые файлы?
    pub sync_mode: String,             // "pull" или "push"
    pub polling_interval_ms: u64,      // Интервал синхронизации
    pub timeout_ms: u64,               // Таймаут операции
    pub match_regex: String,           // Regex для фильтрации файлов
    pub refetch_deleted_file: bool,    // Перекачивать удалённые файлы?
    pub version: u32,                  // Версия конфига
}

impl Config {
    /// Загружает конфиг из файла config.toml, который лежит рядом с исполняемым файлом
    pub fn from_file() -> Result<Self> {
        // Получаем путь к исполняемому файлу
        let exe_path = std::env::current_exe()
            .context("Failed to get executable path")?;
        
        // Формируем путь к config.toml в той же папке
        let config_path = exe_path
            .parent()
            .context("Executable has no parent directory")?
            .join("config.toml");

        // Читаем файл
        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file at {:?}", config_path))?;

        // Парсим TOML в структуру Config
        let config: Config = toml::from_str(&contents)
            .context("Failed to parse config file (invalid TOML)")?;

        Ok(config)
    }
}