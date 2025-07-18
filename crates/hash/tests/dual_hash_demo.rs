//! Demonstration of the dual-hash system in sps2
//! 
//! This example shows how BLAKE3 is used for downloads while xxHash is used for local verification.

use sps2_hash::{Hash, HashAlgorithm};
use std::time::Instant;
use tokio::fs;
use tempfile::TempDir;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔄 sps2 Dual-Hash System Demo");
    println!("=============================\n");

    // Create test data
    let temp_dir = TempDir::new()?;
    let test_file = temp_dir.path().join("demo.txt");
    let test_data = b"This is a demonstration of sps2's dual-hash system. \
                     BLAKE3 provides security for downloads, while xxHash provides \
                     performance for local operations.";
    
    fs::write(&test_file, test_data).await?;
    println!("📁 Created test file: {} bytes", test_data.len());

    // Demonstrate different hash algorithms
    println!("\n🔐 Hash Algorithm Comparison:");
    println!("-----------------------------");

    // BLAKE3 (for downloads)
    let start = Instant::now();
    let blake3_hash = Hash::blake3_hash_file(&test_file).await?;
    let blake3_time = start.elapsed();
    
    println!("BLAKE3 (downloads):  {}", blake3_hash.to_hex());
    println!("  - Length: {} bytes ({} hex chars)", blake3_hash.as_bytes().len(), blake3_hash.to_hex().len());
    println!("  - Time: {:?}", blake3_time);
    println!("  - Use case: Download verification, security-critical operations");

    // xxHash 128-bit (for local verification)
    let start = Instant::now();
    let xxhash = Hash::hash_file(&test_file).await?; // Default is xxHash
    let xxhash_time = start.elapsed();
    
    println!("\nxxHash128 (local):   {}", xxhash.to_hex());
    println!("  - Length: {} bytes ({} hex chars)", xxhash.as_bytes().len(), xxhash.to_hex().len());
    println!("  - Time: {:?}", xxhash_time);
    println!("  - Use case: Local file verification, content-addressed storage");

    // Performance comparison
    if blake3_time > xxhash_time {
        let speedup = blake3_time.as_nanos() as f64 / xxhash_time.as_nanos() as f64;
        println!("\n⚡ Performance: xxHash is {:.1}x faster than BLAKE3", speedup);
    }

    // Demonstrate backward compatibility
    println!("\n🔄 Backward Compatibility:");
    println!("-------------------------");
    
    // Parse existing BLAKE3 hash
    let blake3_hex = blake3_hash.to_hex();
    let parsed_blake3 = Hash::from_hex(&blake3_hex)?;
    println!("✅ BLAKE3 hash parsing: {} -> {}", 
             if parsed_blake3.is_blake3() { "BLAKE3" } else { "Unknown" },
             if parsed_blake3 == blake3_hash { "Match" } else { "Mismatch" });

    // Parse xxHash
    let xxhash_hex = xxhash.to_hex();
    let parsed_xxhash = Hash::from_hex(&xxhash_hex)?;
    println!("✅ xxHash parsing:   {} -> {}", 
             if parsed_xxhash.is_xxhash128() { "xxHash128" } else { "Unknown" },
             if parsed_xxhash == xxhash { "Match" } else { "Mismatch" });

    // Demonstrate use cases
    println!("\n📦 Use Case Examples:");
    println!("--------------------");
    
    println!("Download verification (BLAKE3):");
    println!("  curl https://example.com/package.tar.gz");
    println!("  Expected: {}", blake3_hash.to_hex());
    println!("  ✅ Cryptographically secure");
    
    println!("\nLocal file verification (xxHash):");
    println!("  /opt/pm/live/bin/myapp");
    println!("  Expected: {}", xxhash.to_hex());
    println!("  ⚡ Fast integrity checking");

    // Algorithm selection examples
    println!("\n🎯 Algorithm Selection:");
    println!("----------------------");
    
    // Explicit algorithm selection
    let explicit_blake3 = Hash::hash_file_with_algorithm(&test_file, HashAlgorithm::Blake3).await?;
    let explicit_xxhash = Hash::hash_file_with_algorithm(&test_file, HashAlgorithm::XxHash128).await?;
    
    println!("Explicit BLAKE3:  {} ({})", 
             explicit_blake3.to_hex(), 
             if explicit_blake3 == blake3_hash { "✅ Match" } else { "❌ Mismatch" });
    println!("Explicit xxHash:  {} ({})", 
             explicit_xxhash.to_hex(),
             if explicit_xxhash == xxhash { "✅ Match" } else { "❌ Mismatch" });

    println!("\n🎉 Demo completed successfully!");
    println!("The dual-hash system provides both security and performance.");

    Ok(())
}