# Python Virtual Environment Cleanup Implementation

## Summary

This implementation adds automatic cleanup of Python virtual environments when uninstalling Python packages in sps2.

## Changes Made

### 1. Updated `UninstallOperation` in `/Users/alxknt/Github/sps2/crates/install/src/operations.rs`

- Modified the `remove_packages` method to check for associated venvs when removing packages
- Added new method `remove_package_venv` that:
  - Removes the venv directory from `/opt/pm/venvs/<package>-<version>/`
  - Emits `PythonVenvRemoving` and `PythonVenvRemoved` events
  - Handles errors gracefully

### 2. Added new events in `/Users/alxknt/Github/sps2/crates/events/src/lib.rs`

- `PythonVenvRemoving`: Emitted when starting to remove a venv
- `PythonVenvRemoved`: Emitted after successfully removing a venv

### 3. Added comprehensive tests in `/Users/alxknt/Github/sps2/crates/install/tests/test_venv_cleanup.rs`

- `test_venv_cleanup_on_uninstall`: Verifies that venvs are removed when uninstalling Python packages
- `test_non_python_package_uninstall`: Ensures non-Python packages don't trigger venv cleanup

## How It Works

1. When uninstalling a package, the operation checks if the package has an associated venv path in the database
2. If a venv path exists, it calls `remove_package_venv` to clean it up
3. The venv directory is removed from disk using `tokio::fs::remove_dir_all`
4. Appropriate events are emitted for UI feedback
5. The venv path is automatically cleared from the database when the package is removed from state

## Benefits

- Prevents disk space waste from orphaned virtual environments
- Maintains clean system state
- Provides event feedback for monitoring venv cleanup
- Handles errors gracefully without failing the entire uninstall

## Testing

The implementation includes comprehensive tests that verify:
- Venv directories are properly removed
- Correct events are emitted
- Database state is updated
- Non-Python packages are unaffected

Run tests with:
```bash
cargo test -p sps2-install test_venv_cleanup
```