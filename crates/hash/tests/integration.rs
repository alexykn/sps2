//! Integration tests for hash crate

#[cfg(test)]
mod tests {
    use spsv2_hash::*;
    use tempfile::tempdir;
    use tokio::fs;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_verify_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");

        let data = b"verify this content";
        fs::write(&file_path, data).await.unwrap();

        let hash = Hash::hash(data);
        assert!(verify_file(&file_path, &hash).await.unwrap());

        let wrong_hash = Hash::hash(b"different content");
        assert!(!verify_file(&file_path, &wrong_hash).await.unwrap());
    }

    #[test]
    fn test_hash_from_hex_errors() {
        // Too short
        let result = Hash::from_hex("1234");
        assert!(result.is_err());

        // Too long
        let result = Hash::from_hex(&"a".repeat(65));
        assert!(result.is_err());

        // Invalid hex
        let result = Hash::from_hex("xyz123");
        assert!(result.is_err());
    }
}
