//! Integration tests for store crate

#[cfg(test)]
mod tests {
    use sps2_hash::Hash;
    use sps2_manifest::ManifestBuilder;
    use sps2_store::*;
    use sps2_types::{Arch, Version};
    use tempfile::tempdir;
    use tokio::fs;

    async fn create_test_package(dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        // Create the directory first
        fs::create_dir_all(dir).await?;

        // Create manifest
        let manifest = ManifestBuilder::new(
            "test-pkg".to_string(),
            &Version::parse("1.0.0")?,
            &Arch::Arm64,
        )
        .description("Test package".to_string())
        .depends_on("libtest>=1.0.0")
        .build()?;

        manifest.write_to_file(&dir.join("manifest.toml")).await?;

        // Create files directory with some content
        let files_dir = dir.join("files");
        fs::create_dir_all(&files_dir.join("bin")).await?;
        fs::write(files_dir.join("bin/test"), b"#!/bin/sh\necho test\n").await?;

        fs::create_dir_all(&files_dir.join("lib")).await?;
        fs::write(files_dir.join("lib/libtest.so"), b"binary content").await?;

        // Create blobs directory
        fs::create_dir_all(&dir.join("blobs")).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_package_archive_roundtrip() {
        let temp = tempdir().unwrap();
        let src_dir = temp.path().join("src");
        let sp_file = temp.path().join("test.sp");
        let extract_dir = temp.path().join("extracted");

        // Create test package structure
        create_test_package(&src_dir).await.unwrap();

        // Create .sp archive
        create_package(&src_dir, &sp_file).await.unwrap();
        assert!(sp_file.exists());

        // Extract archive
        extract_package(&sp_file, &extract_dir).await.unwrap();

        // Verify contents
        assert!(extract_dir.join("manifest.toml").exists());
        assert!(extract_dir.join("files/bin/test").exists());
        assert!(extract_dir.join("files/lib/libtest.so").exists());

        // Verify file contents
        let test_script = fs::read_to_string(extract_dir.join("files/bin/test"))
            .await
            .unwrap();
        assert_eq!(test_script, "#!/bin/sh\necho test\n");
    }

    #[tokio::test]
    async fn test_store_add_package() {
        let temp = tempdir().unwrap();
        let store_dir = temp.path().join("store");
        let pkg_dir = temp.path().join("pkg");
        let sp_file = temp.path().join("test.sp");

        // Create store
        let store = PackageStore::new(store_dir.clone());

        // Create and archive test package
        create_test_package(&pkg_dir).await.unwrap();
        create_package(&pkg_dir, &sp_file).await.unwrap();

        // Add to store
        let stored = store.add_package(&sp_file).await.unwrap();

        // Verify it's in the store
        let hash = Hash::hash_file(&sp_file).await.unwrap();
        assert!(store.has_package(&hash).await);

        // Verify stored package
        assert_eq!(stored.manifest().package.name, "test-pkg");
        assert!(stored.files_path().join("bin/test").exists());
    }

    #[tokio::test]
    async fn test_store_link_package() {
        let temp = tempdir().unwrap();
        let store_dir = temp.path().join("store");
        let pkg_dir = temp.path().join("pkg");
        let sp_file = temp.path().join("test.sp");
        let dest_dir = temp.path().join("dest");

        // Create store and package
        let store = PackageStore::new(store_dir.clone());
        create_test_package(&pkg_dir).await.unwrap();
        create_package(&pkg_dir, &sp_file).await.unwrap();

        // Add to store
        let _stored = store.add_package(&sp_file).await.unwrap();
        let hash = Hash::hash_file(&sp_file).await.unwrap();

        // Link to destination
        store.link_package(&hash, &dest_dir).await.unwrap();

        // Verify links
        assert!(dest_dir.join("bin/test").exists());
        assert!(dest_dir.join("lib/libtest.so").exists());

        // Verify content through hard link
        let content = fs::read_to_string(dest_dir.join("bin/test")).await.unwrap();
        assert_eq!(content, "#!/bin/sh\necho test\n");
    }

    #[tokio::test]
    async fn test_stored_package_operations() {
        let temp = tempdir().unwrap();
        let pkg_dir = temp.path().join("pkg");

        // Create test package
        create_test_package(&pkg_dir).await.unwrap();

        // Load as stored package
        let stored = StoredPackage::load(&pkg_dir).await.unwrap();

        // Test manifest access
        assert_eq!(stored.manifest().package.name, "test-pkg");

        // Test file listing
        let files = stored.list_files().await.unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p == std::path::Path::new("bin/test")));
        assert!(files
            .iter()
            .any(|p| p == std::path::Path::new("lib/libtest.so")));

        // Test verification
        stored.verify().await.unwrap();
    }
}
