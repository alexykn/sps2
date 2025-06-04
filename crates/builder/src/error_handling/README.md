# Build Error Handling and Recovery System

This module provides advanced error handling and recovery capabilities for the sps2 build system.

## Features

### 1. **Recovery Strategies**
The system includes multiple built-in recovery strategies:

- **DependencyConflictRecovery**: Handles dependency version conflicts by trying alternative versions
- **CompilationFailedRecovery**: Addresses compilation failures with different flags or reduced parallelism
- **TestsFailedRecovery**: Allows skipping minor test failures or retrying with different settings
- **NetworkErrorRecovery**: Retries network operations with backoff and mirror fallback
- **DiskSpaceRecovery**: Cleans build cache and temporary files before retrying

### 2. **Build Checkpoints**
Save and restore build state at critical stages:

```rust
// Create checkpoint before compilation
let checkpoint_id = handler
    .create_checkpoint(
        "pre-compile".to_string(),
        BuildState {
            env_vars: env.env_vars().clone(),
            completed_steps: vec!["configure".to_string()],
            artifacts: vec![],
            config: HashMap::new(),
        },
        metadata,
    )
    .await?;

// Restore if needed
let checkpoint = handler.restore_checkpoint(&checkpoint_id).await?;
```

### 3. **Recovery Actions**
The system supports various recovery actions:

- **Retry**: Retry with modified configuration
- **Skip**: Skip failing component with warning
- **Alternative**: Use alternative approach
- **CleanRetry**: Clean directories and retry
- **Abort**: Fail with detailed suggestions

### 4. **Integration with Events**
All recovery attempts are reported via the event system:

- `BuildRetrying`: Emitted when retrying after failure
- `BuildWarning`: Emitted for non-critical issues
- `BuildCheckpointCreated`: Checkpoint creation events
- `BuildCheckpointRestored`: Checkpoint restoration events

## Usage Example

```rust
use sps2_builder::{BuildErrorHandler, with_recovery};

// Create error handler
let mut handler = BuildErrorHandler::new(checkpoint_dir);

// Wrap operations with recovery
let result = with_recovery(&mut handler, &context, || {
    Box::pin(async {
        // Build operation that might fail
        compile_package().await
    })
})
.await?;
```

## Custom Recovery Strategies

You can add custom recovery strategies:

```rust
#[derive(Clone)]
struct MyCustomRecovery;

impl RecoveryStrategy for MyCustomRecovery {
    fn clone_box(&self) -> Box<dyn RecoveryStrategy> {
        Box::new(self.clone())
    }
    
    fn can_handle(&self, error: &BuildError) -> bool {
        // Check if this strategy applies
        error.to_string().contains("my_error")
    }
    
    fn recover(&self, error: &BuildError, context: &BuildContext) -> RecoveryAction {
        // Define recovery action
        RecoveryAction::Retry {
            config_changes: HashMap::new(),
            delay: Duration::from_secs(5),
        }
    }
    
    fn description(&self) -> &'static str {
        "My custom recovery"
    }
}

// Register the strategy
handler.register_strategy("my_custom".to_string(), Box::new(MyCustomRecovery));
```

## Configuration

- **Max Retries**: Default 3, configurable via `with_max_retries()`
- **Retry Delay**: Default 5 seconds, configurable via `with_retry_delay()`
- **Max Checkpoints**: Default 10, old checkpoints are automatically cleaned

## Best Practices

1. Create checkpoints before critical operations
2. Use appropriate recovery strategies for different error types
3. Provide detailed error messages and suggestions
4. Monitor recovery statistics through events
5. Clean up checkpoints after successful builds