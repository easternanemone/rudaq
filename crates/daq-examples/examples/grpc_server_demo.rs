//! Demonstration of the DAQ gRPC server
//!
//! This example shows how to:
//! 1. Start the gRPC server
//! 2. Upload scripts via the control API
//! 3. Execute scripts remotely
//! 4. Monitor script execution status
//!
//! Run with: cargo run --example grpc_server_demo

use rust_daq::grpc::start_server;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let addr: SocketAddr = "127.0.0.1:50051".parse()?;

    println!("Starting DAQ gRPC Server Demo");
    println!("==============================");
    println!();
    println!("Server will listen on: {}", addr);
    println!();
    println!("You can interact with the server using:");
    println!("  - grpcurl for command-line testing");
    println!("  - Client applications using the generated protobuf definitions");
    println!();
    println!("Available RPCs:");
    println!("  - UploadScript: Upload and validate a Rhai script");
    println!("  - StartScript: Start execution of an uploaded script");
    println!("  - StopScript: Stop a running script (TODO)");
    println!("  - GetScriptStatus: Query execution status");
    println!("  - StreamStatus: Streaming system status updates (10Hz)");
    println!("  - StreamMeasurements: Streaming measurement data");
    println!();
    println!("Press Ctrl+C to stop the server");
    println!();

    start_server(addr).await?;

    Ok(())
}
