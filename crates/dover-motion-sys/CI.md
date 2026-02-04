# CI/CD Configuration for dover-motion-sys

## Build Matrix

This crate must be tested on BOTH Windows and Linux platforms to ensure cross-platform compatibility.

### Without Hardware (Default)

Runs on standard GitHub Actions runners without Dover Motion SDK installed.

```yaml
jobs:
  test-no-hardware:
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build without SDK
        run: cargo build -p dover-motion-sys
      - name: Test without SDK
        run: cargo test -p dover-motion-sys
      - name: Check clippy
        run: cargo clippy -p dover-motion-sys -- -D warnings
      - name: Check formatting
        run: cargo fmt -p dover-motion-sys -- --check
```

**Expected result**: Builds successfully using dummy bindings. All tests pass.

### With Hardware (dover-sdk Feature)

Requires self-hosted runners with Dover Motion SDK installed.

#### Windows Runner

```yaml
jobs:
  test-windows-hardware:
    runs-on: [self-hosted, windows, dover-motion]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Verify SDK installation
        run: |
          if (-not (Test-Path "$env:DOVER_SDK_DIR\include\IAxisDevice.h")) {
            Write-Error "Dover Motion SDK not found at $env:DOVER_SDK_DIR"
            exit 1
          }

      - name: Build with SDK
        run: cargo build -p dover-motion-sys --features dover-sdk

      - name: Copy DLL to test directory
        run: copy "$env:DOVER_LIB_DIR\MotionSynergyCore.dll" .\target\debug\

      - name: Test with SDK (single-threaded)
        run: cargo test -p dover-motion-sys --features dover-sdk -- --test-threads=1
```

**Expected result**: Builds against real SDK headers. Tests verify binding generation.

#### Linux Runner

```yaml
jobs:
  test-linux-hardware:
    runs-on: [self-hosted, linux, dover-motion]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Verify SDK installation
        run: |
          if [ ! -f "/usr/local/include/dover-motion/IAxisDevice.h" ]; then
            echo "Error: Dover Motion SDK not found"
            exit 1
          fi

      - name: Build with SDK
        run: cargo build -p dover-motion-sys --features dover-sdk

      - name: Test with SDK (single-threaded)
        run: |
          export LD_LIBRARY_PATH=/usr/local/lib:$LD_LIBRARY_PATH
          cargo test -p dover-motion-sys --features dover-sdk -- --test-threads=1
```

**Expected result**: Builds against real SDK headers. Tests verify binding generation.

## Required Environment Variables

### Windows

- `DOVER_SDK_DIR` (optional): SDK installation directory
  - Default: `C:\Program Files\Dover Motion\MotionSynergyAPI`
- `DOVER_INCLUDE_DIR` (optional): Header files directory
  - Default: `$DOVER_SDK_DIR\include`
- `DOVER_LIB_DIR` (optional): Library files directory
  - Default: `$DOVER_SDK_DIR\lib`

### Linux

- `DOVER_SDK_DIR` (optional): SDK installation directory
  - Default: `/usr/local`
- `DOVER_INCLUDE_DIR` (optional): Header files directory
  - Default: `/usr/local/include/dover-motion`
- `DOVER_LIB_DIR` (optional): Library files directory
  - Default: `/usr/local/lib`
- `LD_LIBRARY_PATH`: Must include `$DOVER_LIB_DIR` for runtime

## Self-Hosted Runner Setup

### Windows Runner

1. Install Dover Motion MotionSynergyAPI SDK
2. Install Microsoft Visual C++ Redistributable
3. Add runner labels: `windows`, `dover-motion`
4. Set environment variables:
   ```powershell
   [System.Environment]::SetEnvironmentVariable(
       "DOVER_SDK_DIR",
       "C:\Program Files\Dover Motion\MotionSynergyAPI",
       [System.EnvironmentVariableTarget]::Machine
   )
   ```

### Linux Runner

1. Install Dover Motion SDK:
   ```bash
   sudo dpkg -i dover-motion-sdk_*.deb
   # Or manual installation
   sudo cp libMotionSynergyCore.so /usr/local/lib/
   sudo cp -r include/* /usr/local/include/dover-motion/
   sudo ldconfig
   ```

2. Install build dependencies:
   ```bash
   sudo apt-get install clang libclang-dev
   ```

3. Add runner labels: `linux`, `dover-motion`

4. Set environment variables in runner config:
   ```bash
   export DOVER_SDK_DIR=/usr/local
   export LD_LIBRARY_PATH=/usr/local/lib:$LD_LIBRARY_PATH
   ```

## Testing Strategy

1. **All PRs**: Build and test WITHOUT hardware on Windows + Linux
   - Ensures dummy bindings compile
   - Ensures cross-platform build script works

2. **Before merge**: Test WITH hardware on Windows + Linux (manual trigger)
   - Ensures real SDK bindings generate correctly
   - Verifies platform-specific linking

3. **Release builds**: Full test on both platforms with hardware
   - Ensures production binaries work correctly

## Troubleshooting

### Windows: DLL not found

```
error: linking with `link.exe` failed
```

**Solution**: Copy `MotionSynergyCore.dll` to executable directory or add to PATH.

### Linux: Library not found

```
error while loading shared libraries: libMotionSynergyCore.so
```

**Solution**: Add library directory to `LD_LIBRARY_PATH`:
```bash
export LD_LIBRARY_PATH=/usr/local/lib:$LD_LIBRARY_PATH
```

### Bindgen errors

```
error: unable to generate bindings
```

**Causes**:
- SDK headers not found
- Clang not installed (Linux)
- Wrong C++ standard version

**Solution**: Verify `DOVER_INCLUDE_DIR` points to valid headers.
