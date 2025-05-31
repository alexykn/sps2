//! Integration tests for root crate

#[cfg(test)]
mod tests {
    use sps2_root::*;
    use tempfile::tempdir;
    use tokio::fs;

    #[tokio::test]
    async fn test_ensure_empty_dir() {
        let temp = tempdir().unwrap();
        let test_dir = temp.path().join("ensure_test");

        // Create dir with content
        fs::create_dir(&test_dir).await.unwrap();
        fs::write(test_dir.join("file.txt"), b"content")
            .await
            .unwrap();

        // Ensure empty should remove content
        ensure_empty_dir(&test_dir).await.unwrap();
        assert!(test_dir.exists());

        // Check that directory is empty by counting entries
        let mut entries = fs::read_dir(&test_dir).await.unwrap();
        let entry_count = {
            let mut count = 0;
            while entries.next_entry().await.unwrap().is_some() {
                count += 1;
            }
            count
        };
        assert_eq!(entry_count, 0);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_clone_directory() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        // Create source structure
        fs::create_dir_all(&src.join("subdir")).await.unwrap();
        fs::write(src.join("file1.txt"), b"content1").await.unwrap();
        fs::write(src.join("subdir/file2.txt"), b"content2")
            .await
            .unwrap();

        // Clone
        clone_directory(&src, &dst).await.unwrap();

        // Verify
        assert_eq!(fs::read(dst.join("file1.txt")).await.unwrap(), b"content1");
        assert_eq!(
            fs::read(dst.join("subdir/file2.txt")).await.unwrap(),
            b"content2"
        );
    }

    #[tokio::test]
    async fn test_copy_directory() {
        let temp = tempdir().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");

        // Create source structure
        fs::create_dir_all(&src.join("nested/deep")).await.unwrap();
        fs::write(src.join("root.txt"), b"root").await.unwrap();
        fs::write(src.join("nested/mid.txt"), b"middle")
            .await
            .unwrap();
        fs::write(src.join("nested/deep/deep.txt"), b"deep")
            .await
            .unwrap();

        // Copy
        copy_directory(&src, &dst).await.unwrap();

        // Verify all files copied
        assert_eq!(fs::read(dst.join("root.txt")).await.unwrap(), b"root");
        assert_eq!(
            fs::read(dst.join("nested/mid.txt")).await.unwrap(),
            b"middle"
        );
        assert_eq!(
            fs::read(dst.join("nested/deep/deep.txt")).await.unwrap(),
            b"deep"
        );
    }
}
