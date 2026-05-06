mod config;
mod sync;
mod watchdog;

use anyhow::Result;

fn main() -> Result<()> {
    println!("=== Rust Sync App ===");
    
    let config = config::Config::from_file()?;
    println!("✅ Config loaded successfully!");
    println!("Sync mode: {}", config.sync_mode);
    println!("Local path: {:?}", config.local_path);
    
    watchdog::run_with_watchdog(&config);
    
    // Пока вызываем синхронизацию как обычную функцию
    sync::run_sync_cycle(&config)?;
    
    Ok(())
}