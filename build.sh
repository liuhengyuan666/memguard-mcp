#!/usr/bin/env bash
# MemGuard v3 — Build & Install Script (macOS / Linux)
# Usage: ./build.sh [--install] [--project-root <path>]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY_NAME="memguard-mcp"

# ── Parse arguments ────────────────────────────────────────────────────────
INSTALL=false
PROJECT_ROOT=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install)
      INSTALL=true
      shift
      ;;
    --project-root)
      if [[ -z "${2:-}" || "${2:0:1}" == "-" ]]; then
        echo "ERROR: --project-root requires a path argument" >&2
        exit 1
      fi
      PROJECT_ROOT="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1"
      echo "Usage: $0 [--install] [--project-root <path>]"
      exit 1
      ;;
  esac
done

# ── Build ──────────────────────────────────────────────────────────────────
echo "=== MemGuard v3 MCP Server — Build ==="

echo "[1/3] Compiling release build..."
cd "$SCRIPT_DIR"
if ! cargo build --release; then
  echo "ERROR: cargo build failed" >&2
  exit 1
fi

BINARY_PATH="$SCRIPT_DIR/target/release/$BINARY_NAME"
echo "      Binary: $BINARY_PATH"

echo "[2/3] Running tests..."
if ! cargo test; then
  echo "ERROR: cargo test failed" >&2
  exit 1
fi

echo "[3/3] Done."
echo ""

# ── Install instructions ──────────────────────────────────────────────────
if [ "$INSTALL" = true ]; then
  if [ -n "$PROJECT_ROOT" ]; then
    TARGET_DIR="$PROJECT_ROOT"
  else
    TARGET_DIR="$SCRIPT_DIR"
  fi

  echo "To use MemGuard MCP Server with OpenCode, add this to your opencode.json:"
  echo ""
  echo "  Option A — Global install (RECOMMENDED): no args, auto-detects project via CWD"
  echo "  {"
  echo '    "mcpServers": {'
  echo '      "memguard": {'
  echo "        \"command\": \"$BINARY_PATH\","
  echo "        \"type\": \"local\","
  echo "        \"enabled\": true"
  echo "      }"
  echo "    }"
  echo "  }"
  echo ""
  echo "  Option B — Environment variable (explicit project override):"
  echo "  {"
  echo '    "mcpServers": {'
  echo '      "memguard": {'
  echo "        \"command\": \"$BINARY_PATH\","
  echo "        \"env\": { \"MEMGUARD_PROJECT_ROOT\": \"$TARGET_DIR\" }"
  echo "      }"
  echo "    }"
  echo "  }"
  echo ""
  echo "  Option C — CLI argument (legacy single-project deploy; DEPRECATED for global installs):"
  echo "  {"
  echo '    "mcpServers": {'
  echo '      "memguard": {'
  echo "        \"command\": \"$BINARY_PATH\","
  echo "        \"args\": [\"$TARGET_DIR\"]"
  echo "      }"
  echo "    }"
  echo "  }"
else
  echo "To configure OpenCode, add to opencode.json:"
  echo ""
  echo "  Option A — Global install (RECOMMENDED):"
  echo '    "mcpServers": {'
  echo '      "memguard": {'
  echo "        \"command\": \"<path-to-memguard-mcp>\""
  echo "      }"
  echo "    }"
  echo ""
  echo "  Option B — Environment variable (explicit override):"
  echo '    "mcpServers": {'
  echo '      "memguard": {'
  echo "        \"command\": \"<path-to-memguard-mcp>\","
  echo "        \"env\": { \"MEMGUARD_PROJECT_ROOT\": \"<your-project-root>\" }"
  echo "      }"
  echo "    }"
  echo ""
  echo "  Option C — CLI argument (legacy single-project deploy):"
  echo '    "mcpServers": {'
  echo '      "memguard": {'
  echo "        \"command\": \"<path-to-memguard-mcp>\","
  echo "        \"args\": [\"<your-project-root>\"]"
  echo "      }"
  echo "    }"
fi

echo ""
echo "Build complete. Binary at: $BINARY_PATH"
