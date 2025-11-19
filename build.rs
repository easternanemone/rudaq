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
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["proto/daq.proto"], &["proto"])?;

    Ok(())
}
