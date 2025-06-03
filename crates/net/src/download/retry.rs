//! Retry logic and backoff calculations for downloads

use super::config::RetryConfig;
use std::time::Duration;

/// Calculate exponential backoff delay with jitter
pub(super) fn calculate_backoff_delay(retry_config: &RetryConfig, attempt: u32) -> Duration {
    #[allow(clippy::cast_precision_loss)]
    let base_delay = retry_config
        .initial_delay
        .as_millis()
        .min(u128::from(u64::MAX)) as f64;
    let multiplier = retry_config.backoff_multiplier;
    #[allow(clippy::cast_precision_loss)]
    let max_delay = retry_config.max_delay.as_millis().min(u128::from(u64::MAX)) as f64;

    #[allow(clippy::cast_possible_wrap)]
    let delay = base_delay * multiplier.powi(attempt as i32 - 1);
    let delay = delay.min(max_delay);

    // Add jitter
    let jitter = delay * retry_config.jitter_factor * (rand::random::<f64>() - 0.5);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let final_delay = (delay + jitter).max(0.0).round() as u64;

    Duration::from_millis(final_delay)
}
