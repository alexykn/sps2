//! Integration tests for types

#[cfg(test)]
mod tests {
    use spsv2_types::package::*;
    use spsv2_types::version::*;
    use spsv2_types::*;
    use std::str::FromStr;

    #[test]
    fn test_version_spec_complex() {
        let spec = VersionSpec::from_str(">=1.2.0,<2.0.0,!=1.5.0").unwrap();

        assert!(!spec.matches(&Version::parse("1.1.9").unwrap()));
        assert!(spec.matches(&Version::parse("1.2.0").unwrap()));
        assert!(spec.matches(&Version::parse("1.4.9").unwrap()));
        assert!(!spec.matches(&Version::parse("1.5.0").unwrap())); // Excluded
        assert!(spec.matches(&Version::parse("1.5.1").unwrap()));
        assert!(spec.matches(&Version::parse("1.9.9").unwrap()));
        assert!(!spec.matches(&Version::parse("2.0.0").unwrap()));
    }

    #[test]
    fn test_package_spec_with_complex_version() {
        let spec = PackageSpec::parse("libfoo>=2.0,<3.0,!=2.5.0").unwrap();
        assert_eq!(spec.name, "libfoo");

        let v = Version::parse("2.5.0").unwrap();
        assert!(!spec.version_spec.matches(&v));

        let v = Version::parse("2.4.9").unwrap();
        assert!(spec.version_spec.matches(&v));
    }

    #[test]
    fn test_arch_serialization() {
        let arch = Arch::Arm64;
        let json = serde_json::to_string(&arch).unwrap();
        assert_eq!(json, r#""arm64""#);

        let deserialized: Arch = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, arch);
    }

    #[test]
    fn test_output_format_default() {
        let fmt = OutputFormat::default();
        assert_eq!(fmt, OutputFormat::Tty);
    }
}
