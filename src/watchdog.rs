// src/watchdog.rs
use crate::config::Config;
use crate::logger::Logger;
use std::fs::File;
use std::io::{self, Read, Write};
use std::time::Duration;

fn check_single_instance(config: &Config, logger: &Logger) -> io::Result<()> {
    let mut pid_path = std::env::temp_dir();
    let key = format!(
        "rust_sync_{}_{}.pid",
        config.local_path.display(),
        config.remote_path
    );
    pid_path.push(key.replace('/', "_").replace('\\', "_"));
    
    if let Ok(mut existing_file) = File::open(&pid_path) {
        let mut pid_str = String::new();
        if existing_file.read_to_string(&mut pid_str).is_ok() {
            if let Ok(old_pid) = pid_str.trim().parse::<u32>() {
                let alive = std::process::Command::new("kill")
                    .arg("-0")
                    .arg(&old_pid.to_string())
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false);
                
                if alive {
                    let msg = format!("❌ Another instance is already running (PID: {})", old_pid);
                    println!("{}", msg);
                    logger.log(&msg);
                    std::process::exit(1);
                }
            }
        }
    }
    
    let mut pid_file = File::create(&pid_path)?;
    let current_pid = std::process::id();
    pid_file.write_all(current_pid.to_string().as_bytes())?;
    pid_file.sync_all()?;
    
    let msg = format!("✅ Single instance check passed (PID: {})", current_pid);
    println!("{}", msg);
    logger.log(&msg);
    Ok(())
}

pub fn run_with_watchdog(config: &Config, logger: &Logger) {
    println!("🛡️ Watchdog starting with PID-file lock");
    logger.log("Watchdog starting with PID-file lock");
    
    if let Err(e) = check_single_instance(config, logger) {
        let msg = format!("❌ Failed to acquire lock: {}", e);
        println!("{}", msg);
        logger.log(&msg);
        std::process::exit(1);
    }
    
    let interval = Duration::from_millis(config.polling_interval_ms);
    let local_path = config.local_path.clone();
    let remote_path = config.remote_path.clone();
    let sync_mode = config.sync_mode.clone();

    let mut cycle_count = 0;
    
    loop {
        cycle_count += 1;
        let msg = format!("🔄 Sync cycle #{} started at {:?}", cycle_count, std::time::SystemTime::now());
        println!("{}", msg);
        logger.log(&msg);
        
        match crate::sync::run_sync_cycle_safe(&local_path, &remote_path, &sync_mode, logger) {
            Ok(_) => {
                let msg = format!("✅ Sync cycle #{} completed successfully", cycle_count);
                println!("{}", msg);
                logger.log(&msg);
            }
            Err(e) => {
                let msg = format!("❌ Sync cycle #{} failed: {}", cycle_count, e);
                eprintln!("{}", msg);
                logger.log(&msg);
            }
        }
        
        let msg = format!("⏳ Waiting {} ms...", config.polling_interval_ms);
        println!("{}", msg);
        logger.log(&msg);
        std::thread::sleep(interval);
    }
}