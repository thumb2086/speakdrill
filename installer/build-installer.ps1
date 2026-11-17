#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Build speakdrill MSI installer

.DESCRIPTION
    Builds the speakdrill MSI installer using WiX Toolset.
    Requires: WiX Toolset v3.11+, Rust toolchain, cargo

.PARAMETER Version
    Product version (default: from Cargo.toml)
#>

param(
    [string]$Version = "0.2.42",
    [string]$Configuration = "Release"
)

$ErrorActionPreference = "Stop"

Write-Host "Building speakdrill $Version installer..." -ForegroundColor Green

# 1. Build Rust project
Write-Host "Building Rust project..." -ForegroundColor Cyan
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 2. Check WiX availability
if (-not (Get-Command candle.exe -ErrorAction SilentlyContinue)) {
    Write-Error "WiX Toolset not found. Install from https://wixtoolset.org/"
    exit 1
}
if (-not (Get-Command light.exe -ErrorAction SilentlyContinue)) {
    Write-Error "WiX Toolset light.exe not found."
    exit 1
}

# 3. Compile WiX source
Write-Host "Compiling WiX source..." -ForegroundColor Cyan
candle.exe -dProductVersion=$Version -dInstallDir=speakdrill installer/speakdrill.wxs -out installer/speakdrill.wixobj
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 4. Link to create MSI
Write-Host "Linking MSI..." -ForegroundColor Cyan
light.exe -ext WixUIExtension -cultures:en-us installer/speakdrill.wixobj -out "speakdrill-$Version-win32-x64.msi"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "`n==========================================" -ForegroundColor Green
Write-Host "Installer built successfully!" -ForegroundColor Green
Write-Host "Output: speakdrill-$Version-win32-x64.msi" -ForegroundColor Green
Write-Host "==========================================" -ForegroundColor Green