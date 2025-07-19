//! Retry logic and backoff calculations for downloads

use super::config::RetryConfig;
use std::time::Duration;

/// Calculate exponential backoff delay with jitter
pub(super) fn calculate_backoff_delay(retry_config: &RetryConfig, attempt: u32) -> Duration {
    let base_delay = {
        // Precision loss acceptable for backoff calculations - we don't need nanosecond precision
        #[allow(clippy::cast_precision_loss)]
        {
            retry_config
                .initial_delay
                .as_millis()
                .min(u128::from(u64::MAX)) as f64
        }
    };
    let multiplier = retry_config.backoff_multiplier;
    let max_delay = {
        // Precision loss acceptable for backoff calculations - we don't need nanosecond precision
        #[allow(clippy::cast_precision_loss)]
        {
            retry_config.max_delay.as_millis().min(u128::from(u64::MAX)) as f64
        }
    };

    let delay = base_delay
        * multiplier.powi({
            // Retry attempts are typically small (< 10), so this cast is safe
            #[allow(clippy::cast_possible_wrap)]
            {
                attempt as i32 - 1
            }
        });
    let delay = delay.min(max_delay);

    // Add jitter
    let jitter = delay * retry_config.jitter_factor * (rand::random::<f64>() - 0.5);
    let final_delay = {
        // Safe cast: max(0.0) ensures non-negative, round() handles fractional part
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (delay + jitter).max(0.0).round() as u64
        }
    };

    Duration::from_millis(final_delay)
}
