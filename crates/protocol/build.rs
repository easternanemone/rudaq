//! Build script for protocol
//!
//! Generates gRPC/protobuf bindings during `cargo build`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let is_wasm = target_arch == "wasm32";

    tonic_build::configure()
        .build_server(!is_wasm)
        .build_client(true)
        .build_transport(!is_wasm)
        .type_attribute(".", "#[allow(missing_docs)]")
        .compile(
            &[
                "proto/daq.proto",
                "proto/health.proto",
                "proto/ni_daq.proto",
            ],
            &["proto"],
        )?;

    Ok(())
}
