# Verification script for dover-motion-sys builds (Windows)
# Tests both with and without SDK feature

$ErrorActionPreference = "Stop"

Write-Host "=========================================" -ForegroundColor Cyan
Write-Host "Dover Motion FFI Bindings Build Verification (Windows)" -ForegroundColor Cyan
Write-Host "=========================================" -ForegroundColor Cyan
Write-Host ""

# Test 1: Build without SDK (dummy bindings)
Write-Host "Test 1: Building WITHOUT dover-sdk feature (dummy bindings)" -ForegroundColor Yellow
Write-Host "-----------------------------------------------------------" -ForegroundColor Yellow
cargo clean -p dover-motion-sys
cargo build -p dover-motion-sys
if ($LASTEXITCODE -ne 0) {
    Write-Host "✗ Build without SDK failed" -ForegroundColor Red
    exit 1
}
Write-Host "✓ Build without SDK succeeded" -ForegroundColor Green
Write-Host ""

# Test 2: Test without SDK
Write-Host "Test 2: Testing WITHOUT dover-sdk feature" -ForegroundColor Yellow
Write-Host "------------------------------------------" -ForegroundColor Yellow
cargo test -p dover-motion-sys
if ($LASTEXITCODE -ne 0) {
    Write-Host "✗ Tests without SDK failed" -ForegroundColor Red
    exit 1
}
Write-Host "✓ Tests without SDK passed" -ForegroundColor Green
Write-Host ""

# Test 3: Clippy
Write-Host "Test 3: Clippy linting" -ForegroundColor Yellow
Write-Host "----------------------" -ForegroundColor Yellow
cargo clippy -p dover-motion-sys -- -D warnings
if ($LASTEXITCODE -ne 0) {
    Write-Host "✗ Clippy failed" -ForegroundColor Red
    exit 1
}
Write-Host "✓ Clippy passed" -ForegroundColor Green
Write-Host ""

# Test 4: Format check
Write-Host "Test 4: Format check" -ForegroundColor Yellow
Write-Host "--------------------" -ForegroundColor Yellow
cargo fmt -p dover-motion-sys -- --check
if ($LASTEXITCODE -ne 0) {
    Write-Host "✗ Format check failed" -ForegroundColor Red
    exit 1
}
Write-Host "✓ Format check passed" -ForegroundColor Green
Write-Host ""

# Test 5: Build WITH SDK (if available)
Write-Host "Test 5: Building WITH dover-sdk feature (real bindings)" -ForegroundColor Yellow
Write-Host "--------------------------------------------------------" -ForegroundColor Yellow

$sdkPath = "C:\Program Files\Dover Motion\MotionSynergyAPI"
$headerPath = Join-Path $sdkPath "include\IAxisDevice.h"

if (Test-Path $headerPath) {
    Write-Host "Dover Motion SDK detected at: $sdkPath" -ForegroundColor Green
    Write-Host "Building with dover-sdk feature..." -ForegroundColor Yellow

    cargo clean -p dover-motion-sys
    cargo build -p dover-motion-sys --features dover-sdk
    if ($LASTEXITCODE -ne 0) {
        Write-Host "✗ Build with SDK failed" -ForegroundColor Red
        exit 1
    }
    Write-Host "✓ Build with SDK succeeded" -ForegroundColor Green

    # Copy DLL for testing
    $dllPath = Join-Path $sdkPath "lib\MotionSynergyCore.dll"
    $targetPath = ".\target\debug\MotionSynergyCore.dll"

    if (Test-Path $dllPath) {
        Write-Host "Copying DLL to test directory..." -ForegroundColor Yellow
        Copy-Item $dllPath $targetPath -Force
        Write-Host "✓ DLL copied" -ForegroundColor Green
    } else {
        Write-Host "Warning: MotionSynergyCore.dll not found at $dllPath" -ForegroundColor Yellow
    }

    # Test with SDK
    Write-Host ""
    Write-Host "Test 6: Testing WITH dover-sdk feature" -ForegroundColor Yellow
    Write-Host "---------------------------------------" -ForegroundColor Yellow
    cargo test -p dover-motion-sys --features dover-sdk -- --test-threads=1
    if ($LASTEXITCODE -ne 0) {
        Write-Host "✗ Tests with SDK failed" -ForegroundColor Red
        exit 1
    }
    Write-Host "✓ Tests with SDK passed" -ForegroundColor Green

} else {
    Write-Host "Dover Motion SDK not found at: $sdkPath" -ForegroundColor Yellow
    Write-Host "Skipping SDK build test" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "To test with SDK:" -ForegroundColor Cyan
    Write-Host "  1. Install Dover Motion MotionSynergyAPI SDK" -ForegroundColor Cyan
    Write-Host "  2. Verify installation at: $sdkPath" -ForegroundColor Cyan
    Write-Host "  3. Ensure include/IAxisDevice.h exists" -ForegroundColor Cyan
    Write-Host "  4. Ensure lib/MotionSynergyCore.dll exists" -ForegroundColor Cyan
}

Write-Host ""
Write-Host "=========================================" -ForegroundColor Cyan
Write-Host "All tests passed! ✓" -ForegroundColor Green
Write-Host "=========================================" -ForegroundColor Cyan
