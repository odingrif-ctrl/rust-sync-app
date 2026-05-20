mod config;
mod sync;
mod watchdog;
mod logger;
mod timeout;

use anyhow::Result;

fn main() -> Result<()> {
    let config = config::Config::from_file()?;
    
    // Создаём логгер
    let logger = logger::Logger::new(&config.log_path);
    
    println!("=== Rust Sync App ===");
    println!("✅ Config loaded successfully!");
    println!("Sync mode: {}", config.sync_mode);
    println!("Local path: {:?}", config.local_path);
    println!("Polling interval: {} ms", config.polling_interval_ms);
    
    logger.log("App started");
    logger.log(&format!("Sync mode: {}", config.sync_mode));
    
    watchdog::run_with_watchdog(&config, &logger);
    
    Ok(())
}