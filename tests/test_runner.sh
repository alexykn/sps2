#!/bin/bash
# Test runner script for sps2

set -e

echo "🧪 Running sps2 integration tests..."

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Get the script directory (should be tests/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

print_status "Running cargo fmt check..."
if ! cargo fmt --all -- --check; then
    print_error "Code formatting issues found. Run 'cargo fmt' to fix."
    exit 1
fi

print_status "Running cargo clippy..."
if ! cargo clippy --all-targets --all-features -- -D warnings; then
    print_error "Clippy warnings found. Fix them before continuing."
    exit 1
fi

print_status "Running unit tests..."
if ! cargo test --all --lib; then
    print_error "Unit tests failed."
    exit 1
fi

print_status "Running integration tests..."
if ! cargo test --test integration; then
    print_error "Integration tests failed."
    exit 1
fi

print_status "Running crate-specific integration tests..."
for crate_dir in crates/*/; do
    crate_name=$(basename "$crate_dir")
    if [ -f "$crate_dir/tests/integration.rs" ]; then
        print_status "Testing $crate_name integration..."
        if ! (cd "$crate_dir" && cargo test --test integration); then
            print_error "Integration tests failed for $crate_name"
            exit 1
        fi
    fi
done

print_status "Running CLI integration tests..."
if [ -f "apps/sps2/tests/integration.rs" ]; then
    if ! (cd "apps/sps2" && cargo test --test integration); then
        print_error "CLI integration tests failed."
        exit 1
    fi
fi

print_status "Running build tests..."
if ! cargo build --all --release; then
    print_error "Release build failed."
    exit 1
fi

print_status "Checking for unused dependencies..."
if command -v cargo-machete &> /dev/null; then
    if ! cargo machete; then
        print_warning "Found unused dependencies. Consider removing them."
    fi
else
    print_warning "cargo-machete not installed. Skipping unused dependency check."
fi

print_status "Checking audit for security issues..."
if command -v cargo-audit &> /dev/null; then
    if ! cargo audit; then
        print_error "Security audit found issues."
        exit 1
    fi
else
    print_warning "cargo-audit not installed. Skipping security audit."
fi

print_status "Running documentation tests..."
if ! cargo test --doc; then
    print_error "Documentation tests failed."
    exit 1
fi

print_status "Building documentation..."
if ! cargo doc --all --no-deps; then
    print_error "Documentation build failed."
    exit 1
fi

print_status "All tests passed! ✅"

# Optional: Check binary size
if [ -f "target/release/sps2" ]; then
    binary_size=$(du -h "target/release/sps2" | cut -f1)
    print_status "Binary size: $binary_size"
fi

print_status "Test suite completed successfully!"
