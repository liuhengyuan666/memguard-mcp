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
    Write-Host "  {"
    Write-Host '    "mcpServers": {'
    Write-Host '      "memguard": {'
    Write-Host "        \"command\": `"$binaryPath`","
    Write-Host "        \"args\": [\"<your-project-root>\"]"
    Write-Host "      }"
    Write-Host "    }"
    Write-Host "  }"
}

Write-Host ""
Write-Host "Build complete. Binary at: $binaryPath" -ForegroundColor Green
