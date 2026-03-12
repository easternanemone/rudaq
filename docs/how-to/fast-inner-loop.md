# Fast Inner-Loop Workflows

> **Goal**: Minimize the time between code change and feedback. Use these exact commands for the fastest credible results.

## 1. The "I just want to know it compiles" Loop
Fastest check for the entire workspace excluding UI.
```bash
cargo check --workspace --exclude ui
```

## 2. Crate-Scoped Logic (Unit Tests)
Fastest way to run unit tests for the specific module you are editing.
```bash
# Example: testing common crate
cargo nextest run -p common

# Example: testing storage logic
cargo nextest run -p storage
```

## 3. Integration Smoke Test (Local Mock)
Fastest way to verify gRPC and hardware integration without real hardware.
```bash
cargo nextest run --workspace --exclude ui --exclude comedi-sys --exclude driver-comedi --profile ci
```

## 4. UI WASM Compile Smoke
Check that UI changes don't break the WASM build (run this if editing `crates/ui` or `crates/protocol`).
```bash
cargo check -p ui --lib --target wasm32-unknown-unknown --no-default-features --features web
```

## 5. Hardware-Only Verification (maitai)
*Note: Only works on maitai hardware.*
```bash
source scripts/env-check.sh && cargo nextest run --profile hardware --features hardware_tests
```

## 6. Fast Scripting Loop
Verify Rhai scripts against mock hardware.
```bash
# Run a specific script
cargo run -p bin -- run examples/demo_scan.rhai --hardware-config config/demo.toml
```

## 7. Automated Helper
Run the consolidated fast-check script:
```bash
./scripts/fast-check.sh
```
EOF
