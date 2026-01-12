# Host-Specific Environment Configuration

This directory contains environment configuration files for specific machines.

## Usage

### Manual Sourcing

```bash
# On the maitai hardware machine:
source config/hosts/maitai.env

# Then build/test as normal:
cargo build --features pvcam_sdk
cargo test --features hardware_tests
```

### Automatic Detection

The `scripts/env-check.sh` script will auto-detect common configurations.
For automatic loading on every shell session, source from your shell profile:

```bash
# In ~/.bashrc or ~/.zshrc on maitai:
if [[ -f ~/rust-daq/config/hosts/maitai.env ]]; then
    source ~/rust-daq/config/hosts/maitai.env
fi
```

### With direnv

Copy `.envrc.template` to `.envrc` and customize:

```bash
cp .envrc.template .envrc
# Edit .envrc to source the appropriate host file
direnv allow
```

## Available Host Configurations

| File | Machine | Description |
|------|---------|-------------|
| `maitai.env` | maitai@100.117.5.12 | Hardware machine with PVCAM, serial devices |

## Creating a New Host Configuration

1. Copy an existing file as a template:
   ```bash
   cp config/hosts/maitai.env config/hosts/newhostname.env
   ```

2. Edit the new file with machine-specific values

3. Add a row to this README

## Environment Variables Reference

### PVCAM (Camera SDK)

| Variable | Required | Description |
|----------|----------|-------------|
| `PVCAM_SDK_DIR` | Build | Path to PVCAM SDK (contains `include/`) |
| `PVCAM_LIB_DIR` | Build | Path to PVCAM libraries (contains `libpvcam.so`) |
| `PVCAM_VERSION` | Runtime | Version string (prevents Error 151) |
| `PVCAM_UMD_PATH` | Runtime | User-mode driver path for USB cameras |
| `LIBRARY_PATH` | Build | Must include PVCAM lib directory |
| `LD_LIBRARY_PATH` | Runtime | Must include PVCAM lib and UMD directories |

### Serial Devices

| Variable | Description |
|----------|-------------|
| `ESP300_PORT` | ESP300 motion controller |
| `ELLIPTEC_PORT` | ELL14 rotator bus |
| `NEWPORT_1830C_PORT` | Newport 1830-C power meter |
| `MAITAI_LASER_PORT` | MaiTai laser controller |

### Testing

| Variable | Description |
|----------|-------------|
| `PVCAM_SMOKE_TEST` | Set to `1` to enable hardware smoke tests |
| `PVCAM_CAMERA_NAME` | Camera name for tests (default: `PrimeBSI`) |
