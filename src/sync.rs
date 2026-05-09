// src/sync.rs
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;

/// Основная функция синхронизации (вызывается из watchdog)
pub fn run_sync_cycle_safe(local_path: &PathBuf, remote_path: &str, sync_mode: &str) -> Result<()> {
    println!("   Local:  {:?}", local_path);
    println!("   Remote: {}", remote_path);
    println!("   Mode:   {}", sync_mode);
    
    // Имитация работы: подождём 1 секунду
    std::thread::sleep(Duration::from_secs(1));
    
    Ok(())
}