// src/logger.rs
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Logger {
    file: Mutex<File>,
}

impl Logger {
    pub fn new(path: &PathBuf) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("Failed to open log file");
        Self {
            file: Mutex::new(file),
        }
    }

    pub fn log(&self, msg: &str) {
        let timestamp = chrono::Local::now().format("[%Y-%m-%d %H:%M:%S]");
        let line = format!("{} {}\n", timestamp, msg);
        let mut file = self.file.lock().unwrap();
        file.write_all(line.as_bytes()).unwrap();
    }
}