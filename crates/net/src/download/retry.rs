//! Retry logic and backoff calculations for downloads

use super::config::RetryConfig;
use std::time::Duration;

/// Calculate exponential backoff delay with jitter
///
/// This function implements exponential backoff with proper overflow protection
/// and jitter to prevent thundering herd problems.
pub(super) fn calculate_backoff_delay(retry_config: &RetryConfig, attempt: u32) -> Duration {
    // Cap attempt at a reasonable value to prevent overflow (2^30 is already huge)
    let attempt = attempt.saturating_sub(1).min(30);

    // Calculate exponential backoff: base_delay * multiplier^attempt
    // Precision loss is acceptable for backoff calculations
    #[allow(clippy::cast_precision_loss)]
    let base_ms = retry_config.initial_delay.as_millis() as f64;
    #[allow(clippy::cast_precision_loss)]
    let max_ms = retry_config.max_delay.as_millis() as f64;
    let multiplier = retry_config.backoff_multiplier;

    // Use floating point for exponential calculation, clamped to max_delay
    // Cast is safe: attempt is capped at 30, which fits in i32
    #[allow(clippy::cast_possible_wrap)]
    let delay_ms = (base_ms * multiplier.powi(attempt as i32))
        .min(max_ms)
        .max(0.0);

    // Add jitter: random value in range [-jitter_factor/2, +jitter_factor/2]
    // This prevents thundering herd when multiple clients retry simultaneously
    let jitter_factor = retry_config.jitter_factor.clamp(0.0, 1.0);
    let jitter_ms = delay_ms * jitter_factor * (rand::random::<f64>() - 0.5);
    let final_delay_ms = (delay_ms + jitter_ms).max(0.0);

    // Convert to Duration, clamping at u64::MAX milliseconds
    #[allow(clippy::cast_precision_loss)]
    let final_delay_ms = if final_delay_ms > u64::MAX as f64 {
        u64::MAX
    } else {
        // Safe: value is positive (max'd with 0) and already range-checked
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            final_delay_ms as u64
        }
    };

    Duration::from_millis(final_delay_ms)
}
