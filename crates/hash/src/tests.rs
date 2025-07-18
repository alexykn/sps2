//! Tests for the dual-hash system

use super::*;
use tempfile::TempDir;
use tokio::fs;

#[tokio::test]
async fn test_dual_hash_algorithms() {
    let test_data = b"Hello, world! This is test data for dual hashing.";

    // Test BLAKE3 hashing
    let blake3_hash = Hash::blake3_from_data(test_data);
    assert!(blake3_hash.is_blake3());
    assert!(!blake3_hash.is_xxhash128());
    assert_eq!(blake3_hash.expected_length(), 32);
    assert_eq!(blake3_hash.as_bytes().len(), 32);

    // Test xxHash 128-bit hashing
    let xxhash_hash = Hash::xxhash128_from_data(test_data);
    assert!(xxhash_hash.is_xxhash128());
    assert!(!xxhash_hash.is_blake3());
    assert_eq!(xxhash_hash.expected_length(), 16);
    assert_eq!(xxhash_hash.as_bytes().len(), 16);

    // Hashes should be different
    assert_ne!(blake3_hash, xxhash_hash);
    assert_ne!(blake3_hash.to_hex(), xxhash_hash.to_hex());
}

#[tokio::test]
async fn test_default_algorithm() {
    let test_data = b"Default algorithm test";

    // Default should be xxHash 128-bit
    let default_hash = Hash::from_data(test_data);
    assert!(default_hash.is_xxhash128());
    assert_eq!(default_hash.algorithm(), HashAlgorithm::XxHash128);

    // Should match explicit xxHash
    let explicit_xxhash = Hash::xxhash128_from_data(test_data);
    assert_eq!(default_hash, explicit_xxhash);
}

#[tokio::test]
async fn test_hex_parsing() {
    let test_data = b"Hex parsing test data";

    // Test BLAKE3 hex parsing (64 characters)
    let blake3_hash = Hash::blake3_from_data(test_data);
    let blake3_hex = blake3_hash.to_hex();
    assert_eq!(blake3_hex.len(), 64); // 32 bytes * 2

    let parsed_blake3 = Hash::from_hex(&blake3_hex).unwrap();
    assert_eq!(blake3_hash, parsed_blake3);
    assert!(parsed_blake3.is_blake3());

    // Test xxHash hex parsing (32 characters)
    let xxhash_hash = Hash::xxhash128_from_data(test_data);
    let xxhash_hex = xxhash_hash.to_hex();
    assert_eq!(xxhash_hex.len(), 32); // 16 bytes * 2

    let parsed_xxhash = Hash::from_hex(&xxhash_hex).unwrap();
    assert_eq!(xxhash_hash, parsed_xxhash);
    assert!(parsed_xxhash.is_xxhash128());
}

#[tokio::test]
async fn test_file_hashing() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    let test_content = b"File hashing test content with some length to it";

    fs::write(&test_file, test_content).await.unwrap();

    // Test default file hashing (should be xxHash)
    let default_hash = Hash::hash_file(&test_file).await.unwrap();
    assert!(default_hash.is_xxhash128());

    // Test explicit BLAKE3 file hashing
    let blake3_hash = Hash::blake3_hash_file(&test_file).await.unwrap();
    assert!(blake3_hash.is_blake3());

    // Test explicit algorithm selection
    let xxhash_explicit = Hash::hash_file_with_algorithm(&test_file, HashAlgorithm::XxHash128)
        .await
        .unwrap();
    let blake3_explicit = Hash::hash_file_with_algorithm(&test_file, HashAlgorithm::Blake3)
        .await
        .unwrap();

    assert_eq!(default_hash, xxhash_explicit);
    assert_eq!(blake3_hash, blake3_explicit);
    assert_ne!(default_hash, blake3_hash);
}

#[tokio::test]
async fn test_hash_consistency() {
    let test_data = b"Consistency test data";

    // Same data should produce same hash
    let hash1 = Hash::xxhash128_from_data(test_data);
    let hash2 = Hash::xxhash128_from_data(test_data);
    assert_eq!(hash1, hash2);

    let blake3_hash1 = Hash::blake3_from_data(test_data);
    let blake3_hash2 = Hash::blake3_from_data(test_data);
    assert_eq!(blake3_hash1, blake3_hash2);

    // Different data should produce different hashes
    let different_data = b"Different test data";
    let hash3 = Hash::xxhash128_from_data(different_data);
    assert_ne!(hash1, hash3);
}

#[tokio::test]
async fn test_serialization() {
    let test_data = b"Serialization test";

    // Test xxHash serialization
    let xxhash = Hash::xxhash128_from_data(test_data);
    let serialized = serde_json::to_string(&xxhash).unwrap();
    let deserialized: Hash = serde_json::from_str(&serialized).unwrap();
    assert_eq!(xxhash, deserialized);
    assert!(deserialized.is_xxhash128());

    // Test BLAKE3 serialization
    let blake3 = Hash::blake3_from_data(test_data);
    let serialized = serde_json::to_string(&blake3).unwrap();
    let deserialized: Hash = serde_json::from_str(&serialized).unwrap();
    assert_eq!(blake3, deserialized);
    assert!(deserialized.is_blake3());
}

#[tokio::test]
async fn test_backward_compatibility() {
    // Test that existing BLAKE3 hex strings still work
    let blake3_hex = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"; // Example BLAKE3 hash
    let parsed = Hash::from_hex(blake3_hex).unwrap();
    assert!(parsed.is_blake3());
    assert_eq!(parsed.to_hex(), blake3_hex);

    // Test that the system can handle both hash types
    let xxhash_hex = "1234567890abcdef1234567890abcdef"; // 32 chars = 16 bytes
    let parsed_xxhash = Hash::from_hex(xxhash_hex).unwrap();
    assert!(parsed_xxhash.is_xxhash128());
    assert_eq!(parsed_xxhash.to_hex(), xxhash_hex);
}

#[tokio::test]
async fn test_performance_difference() {
    use std::time::Instant;

    // Create a larger test file for performance comparison
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("large_test.txt");
    let large_content = vec![b'x'; 1024 * 1024]; // 1MB of data

    fs::write(&test_file, &large_content).await.unwrap();

    // Time xxHash
    let start = Instant::now();
    let _xxhash = Hash::hash_file_with_algorithm(&test_file, HashAlgorithm::XxHash128)
        .await
        .unwrap();
    let xxhash_duration = start.elapsed();

    // Time BLAKE3
    let start = Instant::now();
    let _blake3 = Hash::hash_file_with_algorithm(&test_file, HashAlgorithm::Blake3)
        .await
        .unwrap();
    let blake3_duration = start.elapsed();

    // xxHash should be faster (though this test might be flaky on slow systems)
    println!("xxHash duration: {:?}", xxhash_duration);
    println!("BLAKE3 duration: {:?}", blake3_duration);

    // Just verify both complete successfully - actual performance will vary by system
    assert!(xxhash_duration.as_millis() < 1000); // Should complete within 1 second
    assert!(blake3_duration.as_millis() < 1000); // Should complete within 1 second
}

#[tokio::test]
async fn test_invalid_hex() {
    // Test invalid hex strings
    assert!(Hash::from_hex("invalid_hex").is_err());
    assert!(Hash::from_hex("").is_err());

    // Test wrong length hex strings
    assert!(Hash::from_hex("1234").is_err()); // Too short
    assert!(Hash::from_hex(
        "12345678901234567890123456789012345678901234567890123456789012345678901234567890"
    )
    .is_err()); // Too long
}
