# Integration Tests

This directory contains integration tests and test fixtures for the sps2 package manager.

## Structure

```
tests/
├── integration.rs          # Main integration test suite
├── test_runner.sh          # Comprehensive test runner script
├── README.md              # This file
└── fixtures/              # Test data and fixtures
    ├── config/            # Sample configuration files
    │   └── test-config.toml
    ├── manifests/         # Sample package manifests
    │   ├── hello-world-1.0.0.toml
    │   └── complex-app-2.1.3.toml
    ├── recipes/           # Sample build recipes
    │   ├── hello-world.rhai
    │   └── complex-build.rhai
    ├── sboms/             # Sample Software Bills of Materials
    │   ├── hello-world-spdx.json
    │   └── complex-app-cyclonedx.json
    ├── index/             # Sample package index data
    │   └── packages.json
    └── vulnerabilities/   # Sample vulnerability data
        └── sample-vulns.json
```

## Running Tests

### Quick Test Run
```bash
cargo test --test integration
```

### Comprehensive Test Suite
```bash
./tests/test_runner.sh
```

The test runner script performs:
- Code formatting checks (`cargo fmt`)
- Linting with Clippy (`cargo clippy`)
- Unit tests for all crates
- Integration tests
- Release build verification
- Documentation tests
- Security audit (if `cargo-audit` is installed)
- Unused dependency check (if `cargo-machete` is installed)

### Individual Crate Tests
```bash
# Run integration tests for a specific crate
cd crates/audit
cargo test --test integration
```

## Test Fixtures

### Package Manifests
- `hello-world-1.0.0.toml`: Simple package with no dependencies
- `complex-app-2.1.3.toml`: Complex package with multiple dependencies

### Build Recipes
- `hello-world.rhai`: Simple C compilation recipe
- `complex-build.rhai`: Complex CMake-based build with dependencies

### SBOM Files
- `hello-world-spdx.json`: SPDX 2.3 format SBOM
- `complex-app-cyclonedx.json`: CycloneDX 1.6 format SBOM

### Package Index
- `packages.json`: Sample package repository index with multiple packages and versions

### Vulnerability Data
- `sample-vulns.json`: Sample CVE data for testing the audit system

### Configuration
- `test-config.toml`: Test configuration with smaller timeouts and resource limits

## Test Coverage

The integration tests cover:

1. **System Initialization**: Basic setup and component initialization
2. **Manifest Parsing**: TOML manifest parsing and validation
3. **Version Handling**: Version parsing and constraint satisfaction
4. **Dependency Resolution**: Graph construction and topological sorting
5. **Content Hashing**: BLAKE3 hashing functionality
6. **Event System**: Event publishing and receiving
7. **SBOM Parsing**: Both SPDX and CycloneDX format parsing
8. **Index Parsing**: Package repository index handling
9. **Configuration**: Configuration file loading and validation
10. **Vulnerability Data**: CVE data parsing and structure validation
11. **Error Handling**: Invalid input handling and error propagation
12. **Concurrent Operations**: Multi-threaded operation testing
13. **Performance**: Large data handling and stress testing

## Continuous Integration

The test suite is automatically run on:
- Every push to `main` and `develop` branches
- Every pull request to `main`

The CI pipeline includes:
- macOS ARM64 testing (primary target)
- Code formatting validation
- Clippy linting
- Unit and integration tests
- Security audit
- Code coverage reporting
- Documentation generation

## Writing New Tests

When adding new functionality:

1. Add unit tests in the relevant crate's `tests/` directory
2. Add integration tests to `tests/integration.rs` if the feature involves multiple crates
3. Add test fixtures to `tests/fixtures/` if you need sample data
4. Update this README if you add new test categories

### Test Naming Conventions

- Use descriptive test names: `test_dependency_resolution_with_conflicts`
- Group related tests with common prefixes: `test_sbom_*`, `test_audit_*`
- Use `integration` for tests that cross crate boundaries
- Use descriptive assertions with helpful error messages

### Test Environment

Tests use:
- Temporary directories for file operations
- In-memory SQLite databases where possible
- Mock HTTP servers for network tests
- Smaller resource limits for faster execution
- Disabled signature verification for test data

### Performance Testing

Large-scale tests should:
- Use reasonable data sizes (not excessive)
- Have configurable timeouts
- Test both success and failure scenarios
- Verify memory usage doesn't grow unbounded
- Test concurrent operations safely
