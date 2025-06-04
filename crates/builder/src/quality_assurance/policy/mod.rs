//! Policy validation for build artifacts

pub mod license;
pub mod permissions;
pub mod size;

use super::types::{PolicyRule, QaCheck, QaCheckType, QaSeverity};
use crate::events::send_event;
use crate::BuildContext;
use sps2_errors::Error;
use sps2_events::Event;
use std::collections::HashMap;
use std::path::Path;

/// Policy validator trait
#[async_trait::async_trait]
pub trait PolicyValidator: Send + Sync {
    /// Unique ID for this validator
    fn id(&self) -> &str;

    /// Human-readable name
    fn name(&self) -> &str;

    /// Validate against the policy
    async fn validate(
        &self,
        context: &BuildContext,
        path: &Path,
        rule: &PolicyRule,
    ) -> Result<Vec<QaCheck>, Error>;
}

/// Policy validator registry
pub struct PolicyValidatorRegistry {
    validators: HashMap<String, Box<dyn PolicyValidator>>,
}

impl PolicyValidatorRegistry {
    /// Create a new policy validator registry
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            validators: HashMap::new(),
        };

        // Register built-in validators
        registry.register(Box::new(license::LicenseValidator::new()));
        registry.register(Box::new(permissions::PermissionValidator::new()));
        registry.register(Box::new(size::SizeValidator::new()));

        registry
    }

    /// Register a custom validator
    pub fn register(&mut self, validator: Box<dyn PolicyValidator>) {
        self.validators
            .insert(validator.id().to_string(), validator);
    }

    /// Get a validator by ID
    pub fn get(&self, id: &str) -> Option<&dyn PolicyValidator> {
        self.validators.get(id).map(std::convert::AsRef::as_ref)
    }

    /// Run all policy validators
    pub async fn validate_all(
        &self,
        context: &BuildContext,
        path: &Path,
        rules: &[PolicyRule],
    ) -> Result<Vec<QaCheck>, Error> {
        let mut all_checks = Vec::new();

        for rule in rules {
            if !rule.enabled {
                continue;
            }

            // Extract validator ID from rule ID (e.g., "license-compliance" -> "license")
            let validator_id = rule.id.split('-').next().unwrap_or(&rule.id);

            if let Some(validator) = self.get(validator_id) {
                send_event(
                    context,
                    Event::OperationStarted {
                        operation: format!("Validating policy: {}", rule.name),
                    },
                );

                match validator.validate(context, path, rule).await {
                    Ok(checks) => {
                        let check_count = checks.len();
                        all_checks.extend(checks);

                        send_event(
                            context,
                            Event::OperationCompleted {
                                operation: format!(
                                    "Policy {} validated ({} issues)",
                                    rule.name, check_count
                                ),
                                success: true,
                            },
                        );
                    }
                    Err(e) => {
                        send_event(
                            context,
                            Event::BuildWarning {
                                package: context.name.clone(),
                                message: format!("Policy validator {} failed: {}", rule.name, e),
                            },
                        );

                        // Add a check for the validation failure
                        all_checks.push(QaCheck::new(
                            QaCheckType::PolicyValidator,
                            &rule.name,
                            QaSeverity::Warning,
                            format!("Policy validation failed: {}", e),
                        ));
                    }
                }
            } else {
                // Unknown validator
                all_checks.push(QaCheck::new(
                    QaCheckType::PolicyValidator,
                    &rule.name,
                    QaSeverity::Warning,
                    format!("Unknown policy validator: {}", validator_id),
                ));
            }
        }

        Ok(all_checks)
    }
}

impl Default for PolicyValidatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to check if a file should be excluded from validation
pub fn should_exclude_file(path: &Path, exclude_patterns: &[String]) -> bool {
    let path_str = path.to_string_lossy();

    for pattern in exclude_patterns {
        // Simple glob matching (could use glob crate for more complex patterns)
        if pattern.starts_with('*') && pattern.ends_with('*') {
            let substr = &pattern[1..pattern.len() - 1];
            if path_str.contains(substr) {
                return true;
            }
        } else if let Some(suffix) = pattern.strip_prefix('*') {
            if path_str.ends_with(suffix) {
                return true;
            }
        } else if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            if path_str.starts_with(prefix) {
                return true;
            }
        } else if &*path_str == pattern {
            return true;
        }
    }

    false
}
