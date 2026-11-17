@echo off
REM Build installer script for speakdrill
REM Requires: WiX Toolset v3.11+ installed

set PRODUCT_VERSION=0.2.42
set CONFIG=Release

echo Building speakdrill %PRODUCT_VERSION% installer...

REM 1. Build Rust project first
cargo build --release

REM 2. Check if WiX is available
where candle.exe >nul 2>nul
if %errorlevel% neq 0 (
    echo WiX Toolset not found. Please install WiX Toolset v3.11+ from https://wixtoolset.org/
    exit /b 1
)

where light.exe >nul 2>nul
if %errorlevel% neq 0 (
    echo WiX Toolset not found. Please install WiX Toolset v3.11+ from https://wixtoolset.org/
    exit /b 1
)

REM 3. Generate UUID for UpgradeCode if not set
REM First time: run `powershell -c "[guid]::NewGuid()"` and put in .wxs

REM 4. Compile WiX source
echo Compiling WiX source...
candle.exe -dProductVersion=%PRODUCT_VERSION% -dInstallDir=speakdrill installer\speakdrill.wxs -out installer\speakdrill.wixobj
if %errorlevel% neq 0 exit /b 1

REM 5. Link to create MSI
echo Linking MSI...
light.exe -ext WixUIExtension -cultures:en-us installer\speakdrill.wixobj -out speakdrill-%PRODUCT_VERSION%-win32-x64.msi
if %errorlevel% neq 0 exit /b 1

echo.
echo ==========================================
echo Installer built successfully!
echo Output: speakdrill-%PRODUCT_VERSION%-win32-x64.msi
echo ==========================================