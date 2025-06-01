//! Integration tests for config

#[cfg(test)]
mod tests {
    use sps2_config::*;
    use sps2_types::{ColorChoice, OutputFormat};
    use std::io::Write;
    use std::sync::Mutex;
    use tempfile::NamedTempFile;

    // Mutex to ensure env var tests don't run concurrently
    static ENV_TEST_MUTEX: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn test_load_config_from_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
[general]
default_output = "plain"
parallel_downloads = 8
color = "never"

[build]
build_jobs = 4
network_access = true
compression_level = "balanced"

[security]
verify_signatures = false
allow_unsigned = true
index_max_age_days = 7
        "#
        )
        .unwrap();

        let config = Config::load_from_file(temp_file.path()).await.unwrap();
        assert_eq!(config.general.default_output, OutputFormat::Plain);
        assert_eq!(config.general.parallel_downloads, 8);
        assert_eq!(config.general.color, ColorChoice::Never);
        assert_eq!(config.build.build_jobs, 4);
        assert!(config.build.network_access);
        assert!(!config.security.verify_signatures);
    }

    #[test]
    fn test_merge_env() {
        let _guard = ENV_TEST_MUTEX.lock().unwrap();

        // Clean up any existing env vars first
        std::env::remove_var("SPS2_OUTPUT");
        std::env::remove_var("SPS2_COLOR");

        std::env::set_var("SPS2_OUTPUT", "json");
        std::env::set_var("SPS2_COLOR", "always");

        let mut config = Config::default();
        config.merge_env().unwrap();

        assert_eq!(config.general.default_output, OutputFormat::Json);
        assert_eq!(config.general.color, ColorChoice::Always);

        // Clean up
        std::env::remove_var("SPS2_OUTPUT");
        std::env::remove_var("SPS2_COLOR");
    }

    #[test]
    fn test_invalid_env_value() {
        let _guard = ENV_TEST_MUTEX.lock().unwrap();

        // Clean up any existing env vars first
        std::env::remove_var("SPS2_OUTPUT");
        std::env::remove_var("SPS2_COLOR");

        std::env::set_var("SPS2_OUTPUT", "invalid");

        let mut config = Config::default();
        let result = config.merge_env();
        assert!(result.is_err());

        // Clean up
        std::env::remove_var("SPS2_OUTPUT");
    }

    #[tokio::test]
    async fn test_partial_config_with_missing_fields() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
[general]
default_output = "plain"
# missing parallel_downloads and color fields

[build]
build_jobs = 4
network_access = true
# missing compression_level field

[security]
verify_signatures = false
# missing allow_unsigned and index_max_age_days fields
        "#
        )
        .unwrap();

        // After adding #[serde(default)] attributes, this should now succeed
        let config = Config::load_from_file(temp_file.path()).await.unwrap();

        // Verify that default values are used for missing fields
        assert_eq!(config.general.default_output, OutputFormat::Plain); // Explicitly set
        assert_eq!(config.general.color, ColorChoice::Auto); // Should use default
        assert_eq!(config.general.parallel_downloads, 4); // Should use default

        assert_eq!(config.build.build_jobs, 4); // Explicitly set
        assert!(config.build.network_access); // Explicitly set

        assert!(!config.security.verify_signatures); // Explicitly set
        assert!(!config.security.allow_unsigned); // Should use default
        assert_eq!(config.security.index_max_age_days, 7); // Should use default
    }
}
