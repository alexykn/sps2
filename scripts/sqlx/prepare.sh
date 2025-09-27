#!/usr/bin/env bash
set -euo pipefail

# Generates sqlx-data.json for offline compilation using the v2 state schema.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MIGRATIONS_DIR="${REPO_ROOT}/crates/state/migrations"
TARGET_DIR="${REPO_ROOT}/target"
DB_FILE="${TARGET_DIR}/sqlx-dev.sqlite"

mkdir -p "${TARGET_DIR}"
rm -f "${DB_FILE}"
touch "${DB_FILE}"

export DATABASE_URL="sqlite://${DB_FILE}"
export SQLX_OFFLINE_DIR="${REPO_ROOT}/.sqlx"
mkdir -p "${SQLX_OFFLINE_DIR}"

# Apply migrations (if any) before preparing offline data.
if command -v sqlx >/dev/null 2>&1; then
  sqlx migrate run --source "${MIGRATIONS_DIR}" --database-url "${DATABASE_URL}" >/dev/null
else
  echo "sqlx CLI not found; skipping migration run" >&2
fi

cargo sqlx prepare --workspace -- --lib
