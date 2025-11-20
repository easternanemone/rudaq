# PVCAM SDK Documentation

Downloaded from maitai@100.117.5.12 on 2025-11-20

## Contents

- **doc/PVCAM User Manual/** - Complete HTML documentation for PVCAM SDK 3.10.0.3
  - Open `index.xhtml` in a browser to view
  - Includes API reference, examples, and best practices

- **include/** - C header files for PVCAM SDK
  - `pvcam.h` - Main PVCAM API header (201KB)
  - `master.h` - Type definitions and constants

## PVCAM Version

- Version: 3.10.0.3
- Platform: Linux
- Installation location on lab system: `/opt/pvcam/`

## Documentation Highlights

Key sections in the user manual:
- API Basics - Camera initialization and enumeration
- Acquisition Functions - Image capture and streaming
- Parameters - Camera configuration and capabilities
- Advanced Features - Metadata, post-processing, triggering
- Example Guides - Complete working examples with source code

## SDK Installation (on lab system)

The full SDK is installed at:
- Runtime: `/opt/pvcam/`
- Headers: `/opt/pvcam/sdk/include/`
- Libraries: `/opt/pvcam/sdk/lib/`
- Examples: `/opt/pvcam/sdk/examples/`

## Usage for Rust FFI

For binding generation with bindgen:
```bash
# Point to these headers when building pvcam-sys
PVCAM_INCLUDE_DIR=docs/pvcam-sdk/include
```

## Links

- Photometrics website: https://www.photometrics.com
- PVCAM support: https://www.photometrics.com/support
