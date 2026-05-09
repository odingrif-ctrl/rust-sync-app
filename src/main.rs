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
    println!("Polling interval: {} ms", config.polling_interval_ms);
    
    watchdog::run_with_watchdog(&config);
    
    Ok(())
}