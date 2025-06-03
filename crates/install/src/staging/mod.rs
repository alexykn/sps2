//! Staging directory management for secure package extraction
//!
//! This module provides secure staging directory creation, validation, and cleanup
//! for package installation. It ensures that packages are extracted to temporary
//! directories, validated, and then atomically moved to their final location.

pub mod directory;
pub mod guard;
pub mod manager;
pub mod utils;
pub mod validation;

// Re-export main types and functions for external usage
pub use directory::StagingDirectory;
pub use guard::StagingGuard;
pub use manager::StagingManager;

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_resolver::PackageId;
    use sps2_store::PackageStore;
    use tempfile::tempdir;
    use tokio::fs;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_staging_directory_creation() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        // Create staging manager with custom base path for testing
        let staging_base = temp.path().join("staging");

        let manager = StagingManager::new(store, staging_base.clone())
            .await
            .unwrap();

        let package_id = PackageId::new(
            "test-pkg".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );

        let staging_dir = manager.create_staging_dir(&package_id).await.unwrap();

        assert!(staging_dir.path.exists());
        assert!(staging_dir.path.starts_with(&staging_base));
        assert!(!staging_dir.is_validated());

        // Cleanup
        staging_dir.cleanup().await.unwrap();
        assert!(!staging_dir.path.exists());
    }

    #[tokio::test]
    async fn test_staging_guard() {
        let temp = tempdir().unwrap();
        let staging_path = temp.path().join("test-staging");
        fs::create_dir_all(&staging_path).await.unwrap();

        let package_id = PackageId::new(
            "test-pkg".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );

        let staging_dir = StagingDirectory::new(staging_path.clone(), package_id, Uuid::new_v4());

        // Test auto-cleanup with guard
        {
            let _guard = StagingGuard::new(staging_dir);
            assert!(staging_path.exists());
        }

        // Give async cleanup time to run
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn test_file_counting() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        let staging_base = temp.path().join("staging");

        let _manager = StagingManager::new(store, staging_base).await.unwrap();

        // Create test directory structure
        let test_dir = temp.path().join("test");
        fs::create_dir_all(&test_dir).await.unwrap();
        fs::write(test_dir.join("file1.txt"), b"content1")
            .await
            .unwrap();
        fs::write(test_dir.join("file2.txt"), b"content2")
            .await
            .unwrap();

        let sub_dir = test_dir.join("subdir");
        fs::create_dir_all(&sub_dir).await.unwrap();
        fs::write(sub_dir.join("file3.txt"), b"content3")
            .await
            .unwrap();

        let count = utils::count_files(&test_dir).await.unwrap();
        assert_eq!(count, 4); // 2 files + 1 subdir + 1 file in subdir
    }

    #[tokio::test]
    async fn test_directory_structure_validation() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        let staging_base = temp.path().join("staging");

        let _manager = StagingManager::new(store, staging_base).await.unwrap();

        // Test valid structure
        let valid_dir = temp.path().join("valid");
        fs::create_dir_all(&valid_dir).await.unwrap();
        fs::write(valid_dir.join("normal.txt"), b"content")
            .await
            .unwrap();
        fs::write(valid_dir.join(".gitkeep"), b"").await.unwrap();

        let result = validation::validate_directory_structure(&valid_dir).await;
        assert!(result.is_ok());

        // Test invalid structure (suspicious hidden file)
        let invalid_dir = temp.path().join("invalid");
        fs::create_dir_all(&invalid_dir).await.unwrap();
        fs::write(invalid_dir.join(".suspicious"), b"content")
            .await
            .unwrap();

        let result = validation::validate_directory_structure(&invalid_dir).await;
        assert!(result.is_err());
    }
}
