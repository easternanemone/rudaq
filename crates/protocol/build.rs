//! Build script for protocol
//!
//! Generates gRPC/protobuf bindings during `cargo build`.
//!
//! Automatically discovers and compiles all `.proto` files in the `proto/`
//! directory. This supports splitting proto definitions across multiple files
//! while allowing cross-file imports via the shared `proto/` include path.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = PathBuf::from("proto");

    // Discover all .proto files in the proto/ directory
    let proto_files: Vec<PathBuf> = std::fs::read_dir(&proto_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("proto") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    if proto_files.is_empty() {
        return Err("No .proto files found in proto/ directory".into());
    }

    // Re-run build script if any proto file changes or new ones are added
    println!("cargo:rerun-if-changed={}", proto_dir.display());
    for proto in &proto_files {
        println!("cargo:rerun-if-changed={}", proto.display());
    }

    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let is_wasm = target_arch == "wasm32";

    tonic_build::configure()
        .build_server(!is_wasm)
        .build_client(true)
        .build_transport(!is_wasm)
        .type_attribute(".", "#[allow(missing_docs)]")
        .compile_protos(&proto_files, &[proto_dir])?;

    Ok(())
}
