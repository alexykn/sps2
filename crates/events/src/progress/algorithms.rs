#![deny(clippy::pedantic, unsafe_code)]
#![allow(
    clippy::module_name_repetitions,
    clippy::cast_precision_loss,        // Mathematical calculations require f64
    clippy::cast_possible_truncation,   // Intentional for progress calculations
    clippy::cast_sign_loss,            // Weights are always positive
    clippy::similar_names,              // Mathematical variable naming is clear
    clippy::missing_panics_doc,         // Mutex::lock panics are documented as safe
    clippy::must_use_candidate,         // Many builder methods are self-evident
    clippy::uninlined_format_args       // Format args are clear in context
)]

//! Mathematical algorithms for progress tracking
//!
//! This module contains statistical functions and mathematical utilities
//! used by the progress tracking system for sophisticated calculations.

/// Calculate the standard deviation of a dataset
#[allow(dead_code)] // Reserved for future statistical enhancements
pub fn standard_deviation(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mean: f64 = values.iter().sum::<f64>() / values.len() as f64;
    let variance: f64 = values
        .iter()
        .map(|x| (x - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    
    variance.sqrt()
}

/// Calculate weighted average of values
#[allow(dead_code)] // Reserved for future weighted calculations
pub fn weighted_average(values: &[f64], weights: &[f64]) -> Option<f64> {
    if values.len() != weights.len() || values.is_empty() {
        return None;
    }

    let weighted_sum: f64 = values
        .iter()
        .zip(weights.iter())
        .map(|(value, weight)| value * weight)
        .sum();
    
    let total_weight: f64 = weights.iter().sum();
    
    if total_weight > 0.0 {
        Some(weighted_sum / total_weight)
    } else {
        None
    }
}

/// Calculate linear regression slope for trend analysis
#[allow(dead_code)] // Used in speed buffer trend calculation
pub fn linear_regression_slope(x_values: &[f64], y_values: &[f64]) -> Option<f64> {
    if x_values.len() != y_values.len() || x_values.len() < 2 {
        return None;
    }

    let n = x_values.len() as f64;
    let sum_x: f64 = x_values.iter().sum();
    let sum_y: f64 = y_values.iter().sum();
    let sum_x_y: f64 = x_values
        .iter()
        .zip(y_values.iter())
        .map(|(x, y)| x * y)
        .sum();
    let sum_x_squared: f64 = x_values.iter().map(|x| x.powi(2)).sum();

    let denominator = n * sum_x_squared - sum_x.powi(2);
    if denominator.abs() < f64::EPSILON {
        return None; // Avoid division by zero
    }

    Some((n * sum_x_y - sum_x * sum_y) / denominator)
}

/// Calculate exponential moving average
#[allow(dead_code)] // Used for trend smoothing in trackers
pub fn exponential_moving_average(values: &[f64], alpha: f64) -> Option<f64> {
    if values.is_empty() || !(0.0..=1.0).contains(&alpha) {
        return None;
    }

    let mut ema = values[0];
    for &value in values.iter().skip(1) {
        ema = alpha * value + (1.0 - alpha) * ema;
    }

    Some(ema)
}
