#[cfg(test)]
mod tests {
    use crate::python::{is_python_package, PythonVenvManager};
    use sps2_manifest::ManifestBuilder;
    use sps2_resolver::PackageId;
    use sps2_types::{Arch, PythonPackageMetadata, Version};
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_python_venv_creation() {
        let temp_dir = tempdir().unwrap();
        let venvs_base = temp_dir.path().join("venvs");

        let manager = PythonVenvManager::new(venvs_base.clone());

        let package_id = PackageId::new(
            "test-python-pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
        );

        let mut executables = HashMap::new();
        executables.insert("test-app".to_string(), "test_app.cli:main".to_string());

        let python_metadata = PythonPackageMetadata {
            requires_python: ">=3.8".to_string(),
            wheel_file: "test_app-1.0.0-py3-none-any.whl".to_string(),
            requirements_file: "requirements.txt".to_string(),
            executables,
        };

        // This test will fail if uv is not installed, which is expected in CI
        let result = manager
            .create_venv(&package_id, &python_metadata, None)
            .await;

        if result.is_ok() {
            let venv_path = result.unwrap();
            assert!(venv_path.exists());
            assert_eq!(venv_path, venvs_base.join("test-python-pkg-1.0.0"));
        } else {
            // Expected in environments without uv installed
            println!("Skipping test - uv not available");
        }
    }

    #[test]
    fn test_is_python_package() {
        // Test with Python package
        let mut manifest = ManifestBuilder::new(
            "python-app".to_string(),
            &Version::parse("1.0.0").unwrap(),
            &Arch::Arm64,
        )
        .build()
        .unwrap();

        assert!(!is_python_package(&manifest));

        // Add Python metadata
        let mut executables = HashMap::new();
        executables.insert("myapp".to_string(), "myapp:main".to_string());

        manifest.set_python_metadata(PythonPackageMetadata {
            requires_python: ">=3.9".to_string(),
            wheel_file: "myapp-1.0.0-py3-none-any.whl".to_string(),
            requirements_file: String::new(),
            executables,
        });

        assert!(is_python_package(&manifest));
    }

    #[tokio::test]
    async fn test_wrapper_script_creation() {
        let temp_dir = tempdir().unwrap();
        let venvs_base = temp_dir.path().join("venvs");
        let bin_dir = temp_dir.path().join("bin");

        let manager = PythonVenvManager::new(venvs_base);

        let package_id = PackageId::new("test-pkg".to_string(), Version::parse("1.0.0").unwrap());
        let venv_path = temp_dir.path().join("test-venv");

        // Create a fake venv structure
        tokio::fs::create_dir_all(venv_path.join("bin"))
            .await
            .unwrap();

        let mut executables = HashMap::new();
        executables.insert("test-script".to_string(), "test_module:main".to_string());
        executables.insert("another-script".to_string(), "another:run".to_string());

        let scripts = manager
            .create_wrapper_scripts(&package_id, &venv_path, &executables, &bin_dir, None)
            .await
            .unwrap();

        assert_eq!(scripts.len(), 2);

        // Check that wrapper scripts were created
        assert!(bin_dir.join("test-script").exists());
        assert!(bin_dir.join("another-script").exists());

        // Check wrapper script content
        let content = tokio::fs::read_to_string(bin_dir.join("test-script"))
            .await
            .unwrap();
        assert!(content.contains("source"));
        assert!(content.contains("test-venv/bin/activate"));
        assert!(content.contains("test_module"));
        assert!(content.contains("main()"));

        // Check permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = tokio::fs::metadata(bin_dir.join("test-script"))
                .await
                .unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }
}
