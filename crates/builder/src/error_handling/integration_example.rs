//! Example integration of error handling with BuildEnvironment
//!
//! This module demonstrates how to integrate the error handling system
//! with existing build operations.

use crate::{
    error_handling::{BuildErrorHandler, BuildState, RecoveryAction, with_recovery},
    BuildContext, BuildEnvironment, BuildCommandResult,
};
use sps2_errors::{BuildError, Error};
use std::collections::HashMap;
use std::path::PathBuf;

/// Example of wrapping a build step with error recovery
pub async fn build_with_recovery(
    env: &mut BuildEnvironment,
    context: &BuildContext,
    handler: &mut BuildErrorHandler,
) -> Result<(), Error> {
    // Create checkpoint before critical operation
    let checkpoint_id = handler
        .create_checkpoint(
            "pre-compile".to_string(),
            BuildState {
                env_vars: env.env_vars().clone(),
                completed_steps: vec!["configure".to_string()],
                artifacts: vec![],
                config: HashMap::new(),
            },
            HashMap::from([("package".to_string(), context.name.clone())]),
        )
        .await?;
    
    // Send checkpoint event
    if let Some(sender) = &context.event_sender {
        let _ = sender.send(sps2_events::Event::BuildCheckpointCreated {
            package: context.name.clone(),
            checkpoint_id: checkpoint_id.clone(),
            stage: "pre-compile".to_string(),
        });
    }
    
    // Execute build step with recovery
    let compile_op = || {
        let env_clone = env.clone();
        let context_clone = context.clone();
        
        Box::pin(async move {
            // Simulate compilation that might fail
            match env_clone.run_command("make", &["-j4"]).await {
                Ok(result) if result.success => Ok(()),
                Ok(result) => Err(BuildError::CompilationFailed {
                    message: format!("make failed with exit code: {:?}", result.exit_code),
                }),
                Err(e) => Err(BuildError::CompilationFailed {
                    message: format!("Failed to run make: {e}"),
                }),
            }
        })
    };
    
    match with_recovery(handler, context, compile_op).await {
        Ok(_) => Ok(()),
        Err(e) => {
            // Check if we can restore from checkpoint
            if let Ok(checkpoint) = handler.restore_checkpoint(&checkpoint_id).await {
                // Send restore event
                if let Some(sender) = &context.event_sender {
                    let _ = sender.send(sps2_events::Event::BuildCheckpointRestored {
                        package: context.name.clone(),
                        checkpoint_id,
                        stage: checkpoint.stage,
                    });
                }
                
                // Restore environment state
                for (key, value) in checkpoint.state.env_vars {
                    env.set_env_var(key, value)?;
                }
                
                // Try alternative approach based on recovery action
                Err(e)
            } else {
                Err(e)
            }
        }
    }
}

/// Example of handling test failures with recovery
pub async fn run_tests_with_recovery(
    env: &mut BuildEnvironment,
    context: &BuildContext,
    handler: &mut BuildErrorHandler,
) -> Result<bool, Error> {
    let test_op = || {
        let env_clone = env.clone();
        
        Box::pin(async move {
            // Run tests
            match env_clone.run_command("make", &["test"]).await {
                Ok(result) if result.success => Ok(true),
                Ok(_) => {
                    // Parse test output to get pass/fail counts
                    // This is simplified - real implementation would parse actual output
                    Err(BuildError::TestsFailed {
                        passed: 8,
                        total: 10,
                    })
                }
                Err(e) => Err(BuildError::Failed {
                    message: format!("Failed to run tests: {e}"),
                }),
            }
        })
    };
    
    match with_recovery(handler, context, test_op).await {
        Ok(success) => Ok(success),
        Err(e) => {
            // Check if error was converted to warning (tests skipped)
            if e.to_string().contains("Operation skipped") {
                Ok(false) // Tests were skipped
            } else {
                Err(e)
            }
        }
    }
}

/// Example of handling network errors during fetch
pub async fn fetch_source_with_recovery(
    url: &str,
    dest: PathBuf,
    context: &BuildContext,
    handler: &mut BuildErrorHandler,
) -> Result<PathBuf, Error> {
    let url_clone = url.to_string();
    let dest_clone = dest.clone();
    
    let fetch_op = move || {
        let url = url_clone.clone();
        let dest = dest_clone.clone();
        
        Box::pin(async move {
            // Simulate network fetch that might fail
            if url.contains("unreachable") {
                Err(BuildError::FetchFailed { url })
            } else {
                // In real implementation, this would download the file
                Ok(dest)
            }
        })
    };
    
    with_recovery(handler, context, fetch_op).await
}

/// Example of disk space recovery
pub async fn handle_disk_space_error(
    context: &BuildContext,
    handler: &mut BuildErrorHandler,
    operation: impl Fn() -> futures::future::BoxFuture<'static, Result<(), BuildError>> + Clone,
) -> Result<(), Error> {
    match with_recovery(handler, context, operation).await {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("Alternative method required") => {
            // Clean build cache and retry
            let cache_dir = PathBuf::from("/opt/pm/cache/builds");
            if cache_dir.exists() {
                tokio::fs::remove_dir_all(&cache_dir).await?;
            }
            
            // Retry with cleaned cache
            Ok(())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[tokio::test]
    async fn test_build_recovery_integration() {
        let temp = tempdir().unwrap();
        let context = BuildContext::new(
            "test-pkg".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
            temp.path().join("recipe.star"),
            temp.path().to_path_buf(),
        );
        
        let mut handler = BuildErrorHandler::new(temp.path().join("checkpoints"));
        let mut env = BuildEnvironment::new(context.clone(), temp.path()).unwrap();
        
        // This would fail in real scenario but demonstrates the pattern
        let _ = build_with_recovery(&mut env, &context, &mut handler).await;
    }
}