//! Build script for rust_daq
//!
//! Generates gRPC/protobuf bindings during `cargo build`. FlatBuffers generation is
//! currently disabled (Phase 2 network layer).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile FlatBuffers schema (existing)
    // NOTE: Disabled for Phase 2 - Network layer not yet implemented
    // flatc_rust::run(flatc_rust::Args {
    //     inputs: &[std::path::Path::new("schemas/daq.fbs")],
    //     out_dir: std::path::Path::new("src/network/generated/"),
    //     ..Default::default()
    // })
    // .expect("flatc");

    // Compile Protocol Buffers schema (Phase 3: gRPC server)
    // NOTE: type_attribute adds #[allow(missing_docs)] to all generated types
    // since protobuf-generated code cannot have doc comments at source
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let is_wasm = target_arch == "wasm32";

    tonic_build::configure()
        .build_server(!is_wasm)
        // Client codegen is needed, but transport logic (connect()) fails on WASM
        .build_client(true)
        .build_transport(!is_wasm)
        .type_attribute(".", "#[allow(missing_docs)]")
        .compile(&["proto/daq.proto", "proto/health.proto"], &["proto"])?;

    Ok(())
}
