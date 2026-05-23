/**
 * MemGuard MCP — Postinstall Binary Download
 *
 * Detects the current platform and downloads the matching prebuilt Rust binary
 * from the GitHub Releases page.  Extracts and places it alongside the npm
 * shim so the `memguard-mcp` command works immediately after install.
 */

const { createWriteStream, existsSync, mkdirSync, chmodSync, unlinkSync } = require("fs");
const { join, dirname } = require("path");
const { pipeline } = require("stream");
const { promisify } = require("util");
const { createGunzip } = require("zlib");
const { execSync } = require("child_process");
const os = require("os");

const streamPipeline = promisify(pipeline);

// ── Configuration ───────────────────────────────────────────────────────

const GITHUB_REPO = "liuhengyuan666/memguard-mcp";
const BINARY_BASE = "memguard-mcp";

const TARGET_MAP = {
  "win32-x64":   { rustTarget: "x86_64-pc-windows-msvc",  ext: ".exe", archiveExt: ".zip" },
  "darwin-arm64": { rustTarget: "aarch64-apple-darwin",    ext: "",      archiveExt: ".tar.gz" },
  "darwin-x64":   { rustTarget: "x86_64-apple-darwin",     ext: "",      archiveExt: ".tar.gz" },
  "linux-x64":    { rustTarget: "x86_64-unknown-linux-gnu", ext: "",      archiveExt: ".tar.gz" },
  "linux-arm64":  { rustTarget: "aarch64-unknown-linux-gnu", ext: "",     archiveExt: ".tar.gz" },
};

const SUPPORTED_PLATFORMS = Object.keys(TARGET_MAP);

// ── Helpers ─────────────────────────────────────────────────────────────

function getPlatformKey() {
  return `${os.platform()}-${os.arch()}`;
}

function log(msg) {
  console.log(`[memguard-mcp] ${msg}`);
}

function getVersion() {
  try {
    // package.json is in the same directory as this script.
    const pkg = require(join(__dirname, "package.json"));
    return pkg.version;
  } catch {
    return "0.1.0";
  }
}

// ── Download ────────────────────────────────────────────────────────────

async function downloadWithRetry(url, dest, retries = 3) {
  for (let i = 0; i < retries; i++) {
    try {
      const response = await fetch(url, {
        redirect: "follow",
        headers: { "User-Agent": "memguard-mcp-installer" },
      });
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
      }
      const fileStream = createWriteStream(dest);
      await streamPipeline(response.body, fileStream);
      return;
    } catch (err) {
      if (i === retries - 1) throw err;
      log(`Download attempt ${i + 1} failed: ${err.message}. Retrying...`);
      await new Promise((r) => setTimeout(r, 2000));
    }
  }
}

// ── Extract ─────────────────────────────────────────────────────────────

function extractZip(archivePath, destDir) {
  // Windows: use PowerShell Expand-Archive
  if (os.platform() === "win32") {
    execSync(
      `powershell -NoProfile -Command "Expand-Archive -Path '${archivePath}' -DestinationPath '${destDir}' -Force"`,
      { stdio: "pipe" }
    );
  } else {
    // macOS / Linux: use unzip
    execSync(`unzip -o "${archivePath}" -d "${destDir}"`, { stdio: "pipe" });
  }
}

function extractTarGz(archivePath, destDir) {
  const tar = require("tar");
  tar.extract({ file: archivePath, cwd: destDir, sync: true });
}

// ── Main ────────────────────────────────────────────────────────────────

async function main() {
  const platformKey = getPlatformKey();
  const target = TARGET_MAP[platformKey];
  const binaryName = BINARY_BASE + target.ext;
  const version = getVersion();

  // npm global install: binary goes next to the shim (in node_modules/.bin/).
  const shimDir = dirname(__filename);
  const binaryPath = join(shimDir, binaryName);

  // Already installed?
  if (existsSync(binaryPath)) {
    log(`Binary already installed: ${binaryPath}`);
    return;
  }

  if (!SUPPORTED_PLATFORMS.includes(platformKey)) {
    log(`Platform ${platformKey} is not yet supported.`);
    log(`Supported: ${SUPPORTED_PLATFORMS.join(", ")}`);
    return;
  }

  const archiveName = `${BINARY_BASE}-${platformKey}${target.archiveExt}`;
  const archiveUrl = `https://github.com/${GITHUB_REPO}/releases/download/v${version}/${archiveName}`;
  const archivePath = join(shimDir, archiveName);

  log(`Downloading ${archiveName} for ${platformKey}...`);
  log(`URL: ${archiveUrl}`);

  try {
    await downloadWithRetry(archiveUrl, archivePath);
    log(`Downloaded ${archiveName}`);

    // Extract
    if (target.archiveExt === ".zip") {
      extractZip(archivePath, shimDir);
    } else {
      extractTarGz(archivePath, shimDir);
    }
    log(`Extracted to ${shimDir}`);

    // Ensure binary exists after extraction
    if (!existsSync(binaryPath)) {
      // Some archives nest the binary — search one level deep
      const { readdirSync } = require("fs");
      const entries = readdirSync(shimDir, { withFileTypes: true });
      for (const entry of entries) {
        if (entry.isDirectory()) {
          const nested = join(shimDir, entry.name, binaryName);
          if (existsSync(nested)) {
            require("fs").renameSync(nested, binaryPath);
            break;
          }
        }
      }
    }

    if (!existsSync(binaryPath)) {
      throw new Error(`Binary ${binaryName} not found after extraction`);
    }

    // Make executable on Unix
    if (os.platform() !== "win32") {
      chmodSync(binaryPath, 0o755);
    }

    // Clean up archive
    try { unlinkSync(archivePath); } catch {}

    log(`memguard-mcp v${version} installed for ${platformKey}`);
  } catch (err) {
    log(`Failed to install binary: ${err.message}`);
    log(`You can download the binary manually from:`);
    log(`  https://github.com/${GITHUB_REPO}/releases/tag/v${version}`);
    // Clean up partial download
    try { unlinkSync(archivePath); } catch {}
  }
}

main().catch((err) => {
  log(`Installation error: ${err.message}`);
});
