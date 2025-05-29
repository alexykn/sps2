//! Integration tests for error types

#[cfg(test)]
mod tests {
    use spsv2_errors::*;

    #[test]
    fn test_error_conversion() {
        let net_err = NetworkError::Timeout {
            url: "https://example.com".into(),
        };
        let err: Error = net_err.into();
        assert!(matches!(err, Error::Network(_)));
    }

    #[test]
    fn test_error_display() {
        let err = StorageError::DiskFull {
            path: "/opt/pm".into(),
        };
        assert_eq!(err.to_string(), "disk full: /opt/pm");
    }

    #[test]
    fn test_error_clone() {
        let err = PackageError::NotFound { name: "jq".into() };
        let cloned = err.clone();
        assert_eq!(err.to_string(), cloned.to_string());
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "test");
        let storage_err: StorageError = io_err.into();
        assert!(matches!(storage_err, StorageError::PermissionDenied { .. }));
    }
}
