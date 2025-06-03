//! Atomic installation operations using APFS clonefile and state transitions
//!
//! This module provides atomic installation capabilities with:
//! - APFS-optimized file operations for instant, space-efficient copies
//! - Hard link creation for efficient package linking
//! - State transitions with rollback support
//! - Platform-specific filesystem optimizations

pub mod filesystem;
pub mod installer;
pub mod linking;
pub mod rollback;
pub mod transition;

// Re-export main public API
pub use installer::AtomicInstaller;
pub use transition::StateTransition;

// Internal modules - functions used internally but not exposed as part of public API
// Advanced users can access these through the module path if needed

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

    /// Integration test to verify all atomic operations work together
    #[tokio::test]
    async fn test_atomic_operations_integration() {
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        let source_dir = base_path.join("source");
        let dest_dir = base_path.join("dest");
        let staging_dir = base_path.join("staging");

        // Create source structure
        fs::create_dir_all(source_dir.join("bin")).await.unwrap();
        fs::write(source_dir.join("bin/app"), b"app content")
            .await
            .unwrap();
        fs::write(source_dir.join("README.md"), b"readme content")
            .await
            .unwrap();

        // Test staging directory creation (platform-optimized)
        #[cfg(target_os = "macos")]
        {
            filesystem::create_staging_directory(&source_dir, &staging_dir).unwrap();
            assert!(staging_dir.exists());
            assert!(staging_dir.join("bin/app").exists());
        }

        // Test hard link creation
        let mut file_paths = Vec::new();
        linking::create_hardlinks_recursive_with_tracking(
            &source_dir,
            &dest_dir,
            &source_dir,
            &mut file_paths,
        )
        .await
        .unwrap();

        // Verify structure was linked correctly
        assert!(dest_dir.join("bin/app").exists());
        assert!(dest_dir.join("README.md").exists());

        // Verify content integrity
        let app_content = fs::read(dest_dir.join("bin/app")).await.unwrap();
        assert_eq!(app_content, b"app content");

        let readme_content = fs::read(dest_dir.join("README.md")).await.unwrap();
        assert_eq!(readme_content, b"readme content");

        // Verify tracking
        assert!(file_paths
            .iter()
            .any(|(path, is_dir)| path == "bin" && *is_dir));
        assert!(file_paths
            .iter()
            .any(|(path, is_dir)| path == "bin/app" && !*is_dir));
        assert!(file_paths
            .iter()
            .any(|(path, is_dir)| path == "README.md" && !*is_dir));
    }

    /// Test platform compatibility across different filesystem operations
    #[tokio::test]
    async fn test_platform_compatibility() {
        let temp = tempdir().unwrap();
        let source_file = temp.path().join("source.txt");
        let dest_file = temp.path().join("dest.txt");

        // Create test file
        fs::write(&source_file, b"test content").await.unwrap();

        // Test hard link creation (should work on all platforms)
        linking::create_hard_link(&source_file, &dest_file).unwrap();

        // Verify hard link was created
        assert!(dest_file.exists());
        let content = fs::read(&dest_file).await.unwrap();
        assert_eq!(content, b"test content");

        // Test that both files reference the same inode (when supported)
        let source_metadata = fs::metadata(&source_file).await.unwrap();
        let dest_metadata = fs::metadata(&dest_file).await.unwrap();

        // On Unix systems, hard links should have the same inode
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(source_metadata.ino(), dest_metadata.ino());
        }
    }

    /// Test error handling across atomic operations
    #[tokio::test]
    async fn test_error_handling() {
        let temp = tempdir().unwrap();
        let nonexistent_source = temp.path().join("nonexistent");
        let dest_file = temp.path().join("dest.txt");

        // Test hard link creation with nonexistent source
        let result = linking::create_hard_link(&nonexistent_source, &dest_file);
        assert!(result.is_err());

        // Test recursive hardlinks with nonexistent source
        let mut file_paths = Vec::new();
        let result = linking::create_hardlinks_recursive_with_tracking(
            &nonexistent_source,
            &temp.path().join("dest"),
            &nonexistent_source,
            &mut file_paths,
        )
        .await;
        assert!(result.is_err());
    }
}
