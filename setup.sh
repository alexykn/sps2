#!/bin/bash
# spsv2 Package Manager Setup Script

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
PM_ROOT="/opt/pm"
SQLX_DIR="$PM_ROOT/.sqlx"
DB_PATH="$PM_ROOT/state.sqlite"
LIVE_DIR="$PM_ROOT/live"
STORE_DIR="$PM_ROOT/store"
STATES_DIR="$PM_ROOT/states"
LOGS_DIR="$PM_ROOT/logs"

echo "=== spsv2 Package Manager Setup ==="
echo

# Check if running as root
if [[ $EUID -ne 0 ]]; then
   echo -e "${RED}This script must be run as root${NC}"
   exit 1
fi

# Check architecture
ARCH=$(uname -m)
if [[ "$ARCH" != "arm64" && "$ARCH" != "aarch64" ]]; then
    echo -e "${YELLOW}Warning: spsv2 is designed for ARM64 macOS. Current architecture: $ARCH${NC}"
fi

# Create directory structure
echo "Creating directory structure..."
mkdir -p "$PM_ROOT"
mkdir -p "$SQLX_DIR"
mkdir -p "$LIVE_DIR/bin"
mkdir -p "$LIVE_DIR/lib"
mkdir -p "$LIVE_DIR/share"
mkdir -p "$STORE_DIR"
mkdir -p "$STATES_DIR"
mkdir -p "$LOGS_DIR"

# Set permissions
chmod 755 "$PM_ROOT"
chmod 755 "$LIVE_DIR"
chmod 755 "$STORE_DIR"
chmod 755 "$STATES_DIR"
chmod 755 "$LOGS_DIR"

# Initialize SQLite database
echo "Initializing database..."
if [[ ! -f "$DB_PATH" ]]; then
    sqlite3 "$DB_PATH" < crates/state/migrations/0001_initial_schema.sql
    sqlite3 "$DB_PATH" < crates/state/migrations/0002_add_build_deps.sql
    echo -e "${GREEN}Database initialized${NC}"
else
    echo -e "${YELLOW}Database already exists, skipping initialization${NC}"
fi

# Prepare SQLx offline queries
echo "Preparing SQLx offline queries..."
export DATABASE_URL="sqlite://$DB_PATH"

# Create .env file for development
cat > .env << EOF
DATABASE_URL=sqlite://$DB_PATH
SQLX_OFFLINE=true
SQLX_OFFLINE_DIR=$SQLX_DIR
EOF

# Run sqlx prepare
if command -v cargo &> /dev/null && command -v sqlx &> /dev/null; then
    echo "Running cargo sqlx prepare..."
    cd crates/state
    cargo sqlx prepare --database-url "sqlite://$DB_PATH"
    
    # Copy prepared queries to system location
    if [[ -d ".sqlx" ]]; then
        cp -r .sqlx/* "$SQLX_DIR/"
        echo -e "${GREEN}SQLx queries prepared${NC}"
    else
        echo -e "${YELLOW}Warning: .sqlx directory not created${NC}"
    fi
    cd ../..
else
    echo -e "${YELLOW}Warning: cargo or sqlx CLI not found. Install with: cargo install sqlx-cli${NC}"
fi

# Create initial state
echo "Creating initial state..."
INITIAL_STATE_ID=$(uuidgen)
sqlite3 "$DB_PATH" <<EOF
INSERT INTO states (id, parent_id, operation, created_at, success)
VALUES ('$INITIAL_STATE_ID', NULL, 'initial', $(date +%s), 1);

INSERT INTO active_state (id, state_id, updated_at)
VALUES (1, '$INITIAL_STATE_ID', $(date +%s));
EOF

echo -e "${GREEN}Initial state created: $INITIAL_STATE_ID${NC}"

# Setup PATH
echo
echo "Add the following to your shell configuration (.zshrc or .bash_profile):"
echo -e "${YELLOW}export PATH=\"$LIVE_DIR/bin:\$PATH\"${NC}"

# Create sps2 symlink if binary exists
if [[ -f "target/release/sps2" ]]; then
    echo "Installing sps2 binary..."
    cp target/release/sps2 "$LIVE_DIR/bin/"
    chmod 755 "$LIVE_DIR/bin/sps2"
    echo -e "${GREEN}sps2 installed to $LIVE_DIR/bin/${NC}"
elif [[ -f "target/debug/sps2" ]]; then
    echo "Installing sps2 binary (debug build)..."
    cp target/debug/sps2 "$LIVE_DIR/bin/"
    chmod 755 "$LIVE_DIR/bin/sps2"
    echo -e "${YELLOW}sps2 (debug) installed to $LIVE_DIR/bin/${NC}"
else
    echo -e "${YELLOW}sps2 binary not found. Run 'cargo build --release' first${NC}"
fi

echo
echo -e "${GREEN}Setup complete!${NC}"
echo
echo "Next steps:"
echo "1. Add $LIVE_DIR/bin to your PATH"
echo "2. Run 'sps2 reposync' to sync package index"
echo "3. Run 'sps2 list' to see available packages"