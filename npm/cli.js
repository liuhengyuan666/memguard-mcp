#!/usr/bin/env node

/**
 * MemGuard MCP — Entry Shim
 *
 * Locates the platform binary installed by postinstall and spawns it,
 * forwarding all args, stdin, stdout, stderr, and exit code.
 */

const { spawn } = require("child_process");
const { existsSync } = require("fs");
const path = require("path");
const os = require("os");

// ── Platform → binary name ──────────────────────────────────────────────

const PLATFORM_MAP = {
  "win32-x64":   "memguard-mcp.exe",
  "darwin-arm64": "memguard-mcp",
  "linux-x64":    "memguard-mcp",
  "linux-arm64":  "memguard-mcp",
};

function getPlatformKey() {
  return `${os.platform()}-${os.arch()}`;
}

function getBinaryName() {
  return PLATFORM_MAP[getPlatformKey()];
}

// ── Locate binary ───────────────────────────────────────────────────────

function findBinary() {
  const binaryName = getBinaryName();
  if (!binaryName) {
    console.error(
      `[memguard-mcp] Unsupported platform: ${getPlatformKey()}. ` +
      `Currently supported: win32-x64 (more platforms coming soon).`
    );
    process.exit(1);
  }

  // npm global install places binaries in the same directory as this shim.
  const shimDir = path.dirname(__filename);
  const binaryPath = path.join(shimDir, binaryName);

  if (existsSync(binaryPath)) {
    return binaryPath;
  }

  // Fallback: look in the package's own directory (local install).
  const pkgBinary = path.join(__dirname, binaryName);
  if (existsSync(pkgBinary)) {
    return pkgBinary;
  }

  console.error(
    `[memguard-mcp] Binary not found at ${binaryPath} or ${pkgBinary}. ` +
    `Try reinstalling: npm install -g @henry_lhy/memguard-mcp`
  );
  process.exit(1);
}

// ── Spawn ───────────────────────────────────────────────────────────────

const binaryPath = findBinary();
const child = spawn(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
  windowsHide: true,
});

child.on("error", (err) => {
  console.error(`[memguard-mcp] Failed to start binary: ${err.message}`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.exit(128 + (signal === "SIGINT" ? 2 : signal === "SIGTERM" ? 15 : 1));
  }
  process.exit(code ?? 1);
});
