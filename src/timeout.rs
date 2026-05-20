// src/timeout.rs
use std::time::Duration;
use tokio::time::timeout;
use anyhow::Result;

/// Оборачивает асинхронную операцию в таймаут.
/// Если таймаут истекает — возвращает ошибку.
pub async fn with_timeout<T, F, Fut>(timeout_ms: u64, operation: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let duration = Duration::from_millis(timeout_ms);
    match timeout(duration, operation()).await {
        Ok(result) => result,
        Err(_) => {
            anyhow::bail!("Operation timeout after {} ms", timeout_ms)
        }
    }
}