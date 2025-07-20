# Hash Algorithm Migration Guide

## Overview

sps2 has been updated to use a dual-hash system:
- **BLAKE3**: Used for download verification and package integrity checks
- **xxHash 128-bit**: Used for local file verification and content-addressed storage

This change improves performance for local operations while maintaining security for downloads.

## Migration Strategy

### Backward Compatibility

The system maintains backward compatibility by:
1. Automatically detecting hash algorithm based on hex string length:
   - 64 characters (32 bytes) = BLAKE3
   - 32 characters (16 bytes) = xxHash 128-bit
2. Existing BLAKE3 hashes continue to work for verification
3. New local operations default to xxHash 128-bit

### Performance Benefits

xxHash 128-bit provides:
- **3-5x faster** hashing for local file operations
- **Lower CPU usage** during package installation and verification
- **Reduced battery drain** on mobile devices
- **Faster startup times** for package verification

### Security Considerations

- **Download verification** still uses BLAKE3 for cryptographic security
- **Local verification** uses xxHash 128-bit for integrity checking
- xxHash 128-bit provides sufficient collision resistance for local file integrity
- The dual approach balances security and performance appropriately

## Implementation Details

### Hash Algorithm Selection

```rust
use sps2_hash::{Hash, HashAlgorithm};

// For download verification (secure)
let download_hash = Hash::blake3_hash_file(&downloaded_file).await?;

// For local verification (fast)
let local_hash = Hash::hash_file(&local_file).await?; // Uses xxHash by default

// Explicit algorithm selection
let blake3_hash = Hash::hash_file_with_algorithm(&file, HashAlgorithm::Blake3).await?;
let xxhash_hash = Hash::hash_file_with_algorithm(&file, HashAlgorithm::XxHash128).await?;
```

### Migration Timeline

1. **Phase 1** (Current): Dual-hash system introduced
   - New installations use xxHash for local operations
   - Existing BLAKE3 hashes continue to work
   - Download verification remains BLAKE3

2. **Phase 2** (Future): Gradual migration
   - Background rehashing of existing packages to xxHash
   - Performance monitoring and optimization

3. **Phase 3** (Long-term): Full migration
   - All local operations use xxHash
   - BLAKE3 reserved for downloads and security-critical operations

## Testing

The migration has been tested with:
- Existing package installations
- New package installations
- Mixed hash environments
- Performance benchmarks
- Integrity verification

## Rollback Plan

If issues arise:
1. The system can fall back to BLAKE3 for all operations
2. Existing BLAKE3 hashes remain valid
3. No data loss occurs during rollback

## Monitoring

Monitor these metrics during migration:
- Package verification times
- CPU usage during operations
- Hash collision rates (should remain zero)
- User-reported issues

## FAQ

**Q: Will my existing packages still work?**
A: Yes, all existing BLAKE3 hashes continue to work normally.

**Q: Is xxHash secure enough for local verification?**
A: Yes, xxHash 128-bit provides sufficient collision resistance for integrity checking. BLAKE3 is still used for security-critical download verification.

**Q: How much faster is the new system?**
A: Local operations are typically 3-5x faster, with the most noticeable improvements during package installation and verification.

**Q: Can I force BLAKE3 for all operations?**
A: Yes, you can explicitly specify the algorithm in your code, though this will reduce performance benefits.