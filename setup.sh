#!/bin/bash
# sps2 Package Manager Setup Script

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
PM_ROOT="/opt/pm"
DB_PATH="$PM_ROOT/state.sqlite"
LIVE_DIR="$PM_ROOT/live"
STORE_DIR="$PM_ROOT/store"
STATES_DIR="$PM_ROOT/states"
LOGS_DIR="$PM_ROOT/logs"
BUILD_DIR="$PM_ROOT/build"
KEYS_DIR="$PM_ROOT/keys"
VULNDB_DIR="$PM_ROOT/vulndb"

echo "=== sps2 Package Manager Setup ==="
echo

# Check if running as root
if [[ $EUID -ne 0 ]]; then
   echo -e "${RED}This script must be run as root${NC}"
   exit 1
fi

# Get the user who invoked sudo (or current user if not using sudo)
if [[ -n "${SUDO_USER}" ]]; then
    INSTALL_USER="${SUDO_USER}"
else
    INSTALL_USER="${USER}"
fi

echo "Installing for user: $INSTALL_USER"

# Function to set proper ownership
set_ownership() {
    local path="$1"
    chown -R "${INSTALL_USER}:admin" "$path"
}

# Check architecture
ARCH=$(uname -m)
if [[ "$ARCH" != "arm64" && "$ARCH" != "aarch64" ]]; then
    echo -e "${YELLOW}Warning: sps2 is designed for ARM64 macOS. Current architecture: $ARCH${NC}"
fi

# Create directory structure
echo "Creating directory structure..."
mkdir -p "$PM_ROOT"
mkdir -p "$LIVE_DIR"
mkdir -p "$STORE_DIR"
mkdir -p "$STATES_DIR"
mkdir -p "$LOGS_DIR"
mkdir -p "$BUILD_DIR"
mkdir -p "$KEYS_DIR"
mkdir -p "$VULNDB_DIR"

# Set permissions and ownership
chmod 755 "$PM_ROOT"
chmod 755 "$LIVE_DIR"
chmod 755 "$STORE_DIR"
chmod 755 "$STATES_DIR"
chmod 755 "$LOGS_DIR"
chmod 755 "$BUILD_DIR"
chmod 755 "$KEYS_DIR"
chmod 755 "$VULNDB_DIR"

# Set ownership to user:admin
set_ownership "$PM_ROOT"

# Database will be initialized automatically by sqlx migrations on first run
echo "Database will be initialized on first run..."

# Set DATABASE_URL for runtime queries
export DATABASE_URL="sqlite://$DB_PATH"

# Vulnerability database will be initialized by Rust app on first use
echo "Vulnerability database will be initialized on first use..."

# Setup PATH
echo
echo "Add the following to your shell configuration (.zshrc or .bash_profile):"
echo -e "${YELLOW}export PATH=\"$LIVE_DIR/bin:\$PATH\"${NC}"

# Create sps2 symlink if binary exists
if [[ -f "target/release/sps2" ]]; then
    echo "Installing sps2 binary..."
    cp target/release/sps2 "$LIVE_DIR/bin/"
    chmod 755 "$LIVE_DIR/bin/sps2"
    set_ownership "$LIVE_DIR/bin/sps2"
    echo -e "${GREEN}sps2 installed to $LIVE_DIR/bin/${NC}"
elif [[ -f "target/aarch64-apple-darwin/release/sps2" ]]; then
    echo "Installing sps2 binary (aarch64)..."
    cp target/aarch64-apple-darwin/release/sps2 "$LIVE_DIR/bin/"
    chmod 755 "$LIVE_DIR/bin/sps2"
    set_ownership "$LIVE_DIR/bin/sps2"
    echo -e "${YELLOW}sps2 (aarch64) installed to $LIVE_DIR/bin/${NC}"
else
    echo -e "${YELLOW}sps2 binary not found. Run 'cargo build --release' first${NC}"
fi

# Final ownership check to ensure everything is properly owned
echo "Ensuring proper ownership for all files..."
set_ownership "$PM_ROOT"

echo
echo -e "${GREEN}Setup complete!${NC}"
echo -e "All files in $PM_ROOT are owned by ${INSTALL_USER}:admin"
echo
echo "Next steps:"
echo "1. Add $LIVE_DIR/bin to your PATH"
echo "2. Run 'sps2 reposync' to sync package index"
echo "3. Run 'sps2 vulndb update' to update vulnerability database"
echo "4. Run 'sps2 list' to see available packages"
