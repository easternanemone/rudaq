#!/usr/bin/env bash
# Verification script for dover-motion-sys builds
# Tests both with and without SDK feature on Linux

set -e

echo "========================================="
echo "Dover Motion FFI Bindings Build Verification"
echo "========================================="
echo ""

# Detect OS
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    PLATFORM="Linux"
elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
    PLATFORM="Windows"
else
    echo "Warning: Unknown platform $OSTYPE"
    PLATFORM="Unknown"
fi

echo "Platform: $PLATFORM"
echo ""

# Test 1: Build without SDK (dummy bindings)
echo "Test 1: Building WITHOUT dover-sdk feature (dummy bindings)"
echo "-----------------------------------------------------------"
cargo clean -p dover-motion-sys
if cargo build -p dover-motion-sys; then
    echo "✓ Build without SDK succeeded"
else
    echo "✗ Build without SDK failed"
    exit 1
fi
echo ""

# Test 2: Test without SDK
echo "Test 2: Testing WITHOUT dover-sdk feature"
echo "------------------------------------------"
if cargo test -p dover-motion-sys; then
    echo "✓ Tests without SDK passed"
else
    echo "✗ Tests without SDK failed"
    exit 1
fi
echo ""

# Test 3: Clippy
echo "Test 3: Clippy linting"
echo "----------------------"
if cargo clippy -p dover-motion-sys -- -D warnings; then
    echo "✓ Clippy passed"
else
    echo "✗ Clippy failed"
    exit 1
fi
echo ""

# Test 4: Format check
echo "Test 4: Format check"
echo "--------------------"
if cargo fmt -p dover-motion-sys -- --check; then
    echo "✓ Format check passed"
else
    echo "✗ Format check failed"
    exit 1
fi
echo ""

# Test 5: Build WITH SDK (if available)
echo "Test 5: Building WITH dover-sdk feature (real bindings)"
echo "--------------------------------------------------------"

SDK_AVAILABLE=false
if [[ "$PLATFORM" == "Linux" ]]; then
    if [ -f "/usr/local/include/dover-motion/IAxisDevice.h" ]; then
        SDK_AVAILABLE=true
    fi
elif [[ "$PLATFORM" == "Windows" ]]; then
    if [ -f "/c/Program Files/Dover Motion/MotionSynergyAPI/include/IAxisDevice.h" ]; then
        SDK_AVAILABLE=true
    fi
fi

if [ "$SDK_AVAILABLE" = true ]; then
    echo "Dover Motion SDK detected - building with dover-sdk feature"
    cargo clean -p dover-motion-sys
    if cargo build -p dover-motion-sys --features dover-sdk; then
        echo "✓ Build with SDK succeeded"
    else
        echo "✗ Build with SDK failed"
        exit 1
    fi

    # Test with SDK
    echo ""
    echo "Test 6: Testing WITH dover-sdk feature"
    echo "---------------------------------------"
    if cargo test -p dover-motion-sys --features dover-sdk -- --test-threads=1; then
        echo "✓ Tests with SDK passed"
    else
        echo "✗ Tests with SDK failed"
        exit 1
    fi
else
    echo "Dover Motion SDK not found - skipping SDK build test"
    echo "To test with SDK:"
    if [[ "$PLATFORM" == "Linux" ]]; then
        echo "  - Install SDK headers to /usr/local/include/dover-motion/"
        echo "  - Install libMotionSynergyCore.so to /usr/local/lib/"
    elif [[ "$PLATFORM" == "Windows" ]]; then
        echo "  - Install Dover Motion SDK to C:\\Program Files\\Dover Motion\\MotionSynergyAPI"
    fi
fi

echo ""
echo "========================================="
echo "All tests passed! ✓"
echo "========================================="
