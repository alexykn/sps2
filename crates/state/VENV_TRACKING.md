# Python Virtual Environment Tracking

This document describes the venv tracking functionality added to the state management system.

## Database Schema Changes

Added in migration `0004_add_venv_tracking.sql`:
- Added `venv_path TEXT` column to the `packages` table
- Added index on packages with venvs for efficient queries

## API Changes

### StateManager Methods

1. **`add_package_ref_with_venv`** - Add a package with optional venv path
   ```rust
   state_manager.add_package_ref_with_venv(
       &package_ref,
       Some("/opt/pm/venvs/pytest-7.4.0")
   ).await?;
   ```

2. **`get_package_venv_path`** - Get venv path for a specific package
   ```rust
   let venv_path = state_manager
       .get_package_venv_path("pytest", "7.4.0")
       .await?;
   ```

3. **`get_packages_with_venvs`** - List all packages that have venvs
   ```rust
   let packages = state_manager.get_packages_with_venvs().await?;
   // Returns Vec<(name, version, venv_path)>
   ```

4. **`update_package_venv_path`** - Update or remove venv path
   ```rust
   // Update path
   state_manager.update_package_venv_path(
       "pytest", "7.4.0", 
       Some("/new/path")
   ).await?;
   
   // Remove path
   state_manager.update_package_venv_path(
       "pytest", "7.4.0", 
       None
   ).await?;
   ```

## Usage in Install/Uninstall Operations

During package installation:
- Python packages should call `add_package_ref_with_venv` with the venv path
- Non-Python packages use the regular `add_package_ref` method

During package uninstallation:
- Query `get_package_venv_path` to check if a venv exists
- If venv exists, remove the venv directory at the returned path
- The venv_path in the database will be removed when the package is removed from the state

## Venv Path Convention

Python venvs are stored at: `/opt/pm/venvs/<package>-<version>/`

Example: `/opt/pm/venvs/pytest-7.4.0/`