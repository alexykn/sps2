//! Integration tests for config

#[cfg(test)]
mod tests {
    use spsv2_config::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_load_config_from_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
[general]
parallel_downloads = 8
color = "never"

[build]
build_jobs = 4
network_access = true

[security]
verify_signatures = false
        "#
        )
        .unwrap();

        let config = Config::load_from_file(temp_file.path()).await.unwrap();
        assert_eq!(config.general.parallel_downloads, 8);
        assert_eq!(config.general.color, ColorChoice::Never);
        assert_eq!(config.build.build_jobs, 4);
        assert!(config.build.network_access);
        assert!(!config.security.verify_signatures);
    }

    #[test]
    fn test_merge_env() {
        std::env::set_var("SPSV2_OUTPUT", "json");
        std::env::set_var("SPSV2_COLOR", "always");

        let mut config = Config::default();
        config.merge_env().unwrap();

        assert_eq!(config.general.default_output, OutputFormat::Json);
        assert_eq!(config.general.color, ColorChoice::Always);

        // Clean up
        std::env::remove_var("SPSV2_OUTPUT");
        std::env::remove_var("SPSV2_COLOR");
    }

    #[test]
    fn test_invalid_env_value() {
        std::env::set_var("SPSV2_OUTPUT", "invalid");

        let mut config = Config::default();
        let result = config.merge_env();
        assert!(result.is_err());

        // Clean up
        std::env::remove_var("SPSV2_OUTPUT");
    }
}
