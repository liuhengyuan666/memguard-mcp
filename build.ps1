# MemGuard v3 — Build & Install Script
# Usage: .\build.ps1 [-Install] [-ProjectRoot <path>]
param(
    [switch]$Install,
    [string]$ProjectRoot
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$binaryName = "memguard-mcp.exe"

Write-Host "=== MemGuard v3 MCP Server — Build ===" -ForegroundColor Cyan

# Step 1: Compile release binary
Write-Host "[1/3] Compiling release build..." -ForegroundColor Yellow
Push-Location $scriptDir
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally {
    Pop-Location
}

$binaryPath = Join-Path $scriptDir "target\release\$binaryName"
Write-Host "      Binary: $binaryPath" -ForegroundColor Green

# Step 2: Run tests (sanity check)
Write-Host "[2/3] Running tests..." -ForegroundColor Yellow
Push-Location $scriptDir
try {
    cargo test
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed" }
} finally {
    Pop-Location
}

# Step 3: Show install instructions or perform install
Write-Host "[3/3] Done." -ForegroundColor Green
Write-Host ""

if ($Install) {
    $targetDir = if ($ProjectRoot) {
        $ProjectRoot
    } else {
        $scriptDir
    }

    Write-Host "To use MemGuard MCP Server with OpenCode, add this to your opencode.json:" -ForegroundColor White
    Write-Host ""
    Write-Host "  Option A — Global install (RECOMMENDED): no args, auto-detects project via CWD" -ForegroundColor Cyan
    Write-Host "  {" -ForegroundColor Gray
    Write-Host '    "mcpServers": {' -ForegroundColor Gray
    Write-Host '      "memguard": {' -ForegroundColor Gray
    Write-Host "        \"command\": `"$binaryPath`"," -ForegroundColor Gray
    Write-Host "        \"type\": \"local\"," -ForegroundColor Gray
    Write-Host "        \"enabled\": true" -ForegroundColor Gray
    Write-Host "      }" -ForegroundColor Gray
    Write-Host "    }" -ForegroundColor Gray
    Write-Host "  }" -ForegroundColor Gray
    Write-Host ""
    Write-Host "  Option B — Environment variable (explicit project override):" -ForegroundColor Cyan
    Write-Host "  {" -ForegroundColor Gray
    Write-Host '    "mcpServers": {' -ForegroundColor Gray
    Write-Host '      "memguard": {' -ForegroundColor Gray
    Write-Host "        \"command\": `"$binaryPath`"," -ForegroundColor Gray
    Write-Host "        \"env\": { \"MEMGUARD_PROJECT_ROOT\": `"$targetDir`" }" -ForegroundColor Gray
    Write-Host "      }" -ForegroundColor Gray
    Write-Host "    }" -ForegroundColor Gray
    Write-Host "  }" -ForegroundColor Gray
    Write-Host ""
    Write-Host "  Option C — CLI argument (legacy single-project deploy; DEPRECATED for global installs):" -ForegroundColor Yellow
    Write-Host "  {" -ForegroundColor Gray
    Write-Host '    "mcpServers": {' -ForegroundColor Gray
    Write-Host '      "memguard": {' -ForegroundColor Gray
    Write-Host "        \"command\": `"$binaryPath`"," -ForegroundColor Gray
    Write-Host "        \"args\": [`"$targetDir`"]" -ForegroundColor Gray
    Write-Host "      }" -ForegroundColor Gray
    Write-Host "    }" -ForegroundColor Gray
    Write-Host "  }" -ForegroundColor Gray
} else {
    Write-Host "To configure OpenCode, add to opencode.json:" -ForegroundColor White
    Write-Host ""
    Write-Host "  Option A — Global install (RECOMMENDED):"
    Write-Host '    "mcpServers": {'
    Write-Host '      "memguard": {'
    Write-Host "        \"command\": \"<path-to-memguard-mcp.exe>\""
    Write-Host "      }"
    Write-Host "    }"
    Write-Host ""
    Write-Host "  Option B — Environment variable (explicit override):"
    Write-Host '    "mcpServers": {'
    Write-Host '      "memguard": {'
    Write-Host "        \"command\": \"<path-to-memguard-mcp.exe>\","
    Write-Host "        \"env\": { \"MEMGUARD_PROJECT_ROOT\": \"<your-project-root>\" }"
    Write-Host "      }"
    Write-Host "    }"
    Write-Host ""
    Write-Host "  Option C — CLI argument (legacy single-project deploy):"
    Write-Host '    "mcpServers": {'
    Write-Host '      "memguard": {'
    Write-Host "        \"command\": \"<path-to-memguard-mcp.exe>\","
    Write-Host "        \"args\": [\"<your-project-root>\"]"
    Write-Host "      }"
    Write-Host "    }"
}

Write-Host ""
Write-Host "Build complete. Binary at: $binaryPath" -ForegroundColor Green
