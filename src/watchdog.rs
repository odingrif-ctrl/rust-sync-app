// src/watchdog.rs
use crate::config::Config;
use std::fs::File;
use std::io::{self, Write};
use std::time::Duration;

/// Проверяет, не запущен ли уже другой экземпляр, используя PID-файл
fn check_single_instance(config: &Config) -> io::Result<()> {
    let mut pid_path = std::env::temp_dir();
    let key = format!(
        "rust_sync_{}_{}.pid",
        config.local_path.display(),
        config.remote_path
    );
    pid_path.push(key.replace('/', "_").replace('\\', "_"));
    
    // Пытаемся открыть существующий PID-файл
    if let Ok(mut existing_file) = File::open(&pid_path) {
        let mut pid_str = String::new();
        use std::io::Read;
        if existing_file.read_to_string(&mut pid_str).is_ok() {
            if let Ok(old_pid) = pid_str.trim().parse::<u32>() {
                // Проверяем, жив ли процесс с таким PID
                if std::process::Command::new("kill")
                    .arg("-0")
                    .arg(&old_pid.to_string())
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false)
                {
                    eprintln!("❌ Another instance is already running (PID: {})", old_pid);
                    std::process::exit(1);
                }
            }
        }
    }
    
    // Создаём новый PID-файл
    let mut pid_file = File::create(&pid_path)?;
    let current_pid = std::process::id();
    pid_file.write_all(current_pid.to_string().as_bytes())?;
    pid_file.sync_all()?;
    
    println!("✅ Single instance check passed (PID: {})", current_pid);
    Ok(())
}

pub fn run_with_watchdog(config: &Config) {
    println!("🛡️ Watchdog v3: starting with PID-file lock");
    
    // Проверяем, нет ли уже запущенного экземпляра
    if let Err(e) = check_single_instance(config) {
        eprintln!("❌ Failed to create PID file: {}", e);
        std::process::exit(1);
    }
    
    let interval = Duration::from_millis(config.polling_interval_ms);
    let local_path = config.local_path.clone();
    let remote_path = config.remote_path.clone();
    let sync_mode = config.sync_mode.clone();
    
    loop {
        println!(
            "\n🔄 Sync cycle started at {:?}",
            std::time::SystemTime::now()
        );
        
        if let Err(e) = crate::sync::run_sync_cycle_safe(&local_path, &remote_path, &sync_mode) {
            eprintln!("❌ Sync cycle failed: {}", e);
        } else {
            println!("✅ Sync cycle completed successfully");
        }
        
        println!("⏳ Waiting {} ms...", config.polling_interval_ms);
        std::thread::sleep(interval);
    }
}