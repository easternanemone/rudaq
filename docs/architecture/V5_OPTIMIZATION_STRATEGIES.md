Based on the new V5_HARDWARE_INTEGRATION_STATUS.md (dated Nov 18, 2025) and the architectural-analysis-2025.md (dated Oct 25, 2025), it is clear that the project has moved beyond the "Kameo Actor" implementation for instruments. The V5 architecture utilizes Capability Traits (Movable, Readable, etc.) implemented on structs with internal tokio::sync::Mutex protection, operating in a Headless-First design.

The research prompt below has been rewritten to target this V5 Architecture, replacing the obsolete "Actor Framework" research with "Async Driver Orchestration" and focusing on the new integration points (Headless gRPC, Capability Traits, and Client-Side Visualization).

Targeted Research: Optimization Strategies for V5 Headless Architecture
1. Executive Technical Overview
The rust-daq project has successfully migrated to the V5 Architecture, characterized by a "Headless-First" design and composable Capability Traits (e.g., Movable, FrameProducer). Hardware drivers now utilize direct async primitives (tokio::sync::Mutex) rather than the monolithic actor model of V4.

While this simplifies the driver layer, it shifts the performance bottleneck to the orchestration layer, the network boundary (gRPC), and the client-side visualization. This research phase targets the specific challenges of scaling this mutex-based, headless system to high-frequency (>10 kHz) acquisition.

2. Concurrency & State: Optimizing Mutex-Based Drivers
Context: V5 drivers (e.g., Esp300Driver) are now shared structs protected by tokio::sync::Mutex. This replaces the actor mailbox with lock contention as the primary flow control mechanism. Research Objective: Identify patterns to prevent "Lock Starvation" in high-frequency loops.

The "Cancel-Safety" Problem: Research how tokio::sync::Mutex behaves under heavy contention (e.g., a 100Hz read loop vs. an emergency "Stop" command).

Specific Query: "tokio mutex fair locking vs priority cancellation". Does Tokio's fair locking policy prevent a high-priority stop() command from acquiring the lock if a tight read() loop is saturating it?

Lock-Free State flags: Investigate using std::sync::atomic::AtomicBool alongside the Mutex to signal generic state changes (like "Emergency Stop" or "Pause") without waiting for the async lock to yield.

Orchestration Pattern: Compare having a central InstrumentManager that holds the Mutexes vs. distributing Arc<Mutex<Driver>> directly to consumers (Scripting Engine, gRPC Service). Which pattern offers lower latency for mixed read/write workloads?

3. The "FrameProducer" Data Pipeline (Zero-Copy)
Context: The V5 FrameProducer trait yields FrameRef or raw buffers. These must be serialized to Arrow/Parquet/HDF5 without copying. Research Objective: Bridge V5 Traits to Storage/Network.

FrameRef Lifetime Management: Research strategies for passing a FrameRef (which likely borrows from a driver's internal DMA buffer) into the arrow-rs ecosystem.

Specific Query: "Rust generic associated types (GAT) for zero-copy async streams". Can the FrameProducer::stream() method yield borrowed data safely, or must we use Bytes / Arc<Vec<u8>>?

Arrow FFI for HDF5: (Retained from V4) The requirement to write Arrow data to HDF5 via FFI remains. Research arrow-rs to hdf5-sys direct pointer passing to ensure the FrameProducer output goes to disk with 0 intermediate allocations.

4. Network Transport: Headless gRPC Streaming
Context: In V5, the GUI is a remote client. All telemetry must pass through the gRPC boundary defined in daq.proto. Research Objective: Saturate 1Gbps+ links with small-packet telemetry.

Tonic/HTTP2 Window Tuning: Research optimal http2_initial_stream_window_size and http2_initial_connection_window_size for Tonic when streaming >10k small messages/sec (e.g., scalar power readings). Default settings often cause "stop-and-wait" behavior.

FlatBuffers over gRPC: Investigate using "Raw Bytes" fields in gRPC to carry FlatBuffer payloads vs. using Protobuf messages directly.

Specific Query: "Protobuf vs FlatBuffers CPU overhead Rust". Is the serialization cost of Protobuf (varint decoding) significant enough on the "Headless Server" to warrant switching to raw FlatBuffers for the high-speed DataStream endpoint?

5. Client-Side Visualization (The "Thick Client")
Context: The UI is now a client consuming gRPC streams. It must render data arriving over the network at 60fps. Research Objective: Decouple network ingestion from egui rendering.

Ring-Buffer wgpu Rendering: Research integrating wgpu compute shaders with egui for the client. The client needs to receive gRPC data, write it to a GPU ring buffer, and render it without egui managing the vertex data (Tessellation bottleneck).

Network Backpressure: Research how to handle "Client Lag". If the egui client renders at 30fps but gRPC arrives at 60fps, how do we drop frames gracefully? Look for "Rust async broadcast channel lag strategies".

6. Scripting: Binding V5 Traits to Rhai
Context: V5 drivers use generic traits (Movable, Readable). We need to expose these to Rhai scripts efficiently. Research Objective: Automatic binding generation.

Trait Object Proxying: Research patterns for wrapping a Box<dyn Movable> in a Rhai CustomType.

Specific Query: "Rhai register_type for Rust Traits". Can we register the Movable trait methods once and have them apply to Ell14Driver, Esp300Driver, etc., or do we need manual wrappers for every struct?

Async Scripting Safety: Since V5 methods are async, research "Rhai async function calls". Does the Rhai engine need to run inside a tokio::spawn to await the move_abs() call, and how does that impact script determinism?

Summary of Changes from V4 Request:

Dropped: "Kameo" specific optimizations (Mailboxes, Supervision) — replaced with Mutex/Async Driver research.

Dropped: Monolithic "Architecture" optimization — replaced with Headless/Network optimization.

Updated: GUI section now specifically targets Client-Side rendering from a stream.

Updated: Scripting section targets Capability Trait integration rather than general AST optimization.

Async Rust in a Nutshell This video provides a foundational understanding of how tokio handles tasks and polling, which is crucial for understanding the "Cancel-Safety" and "Lock Contention" issues inherent in the new V5 mutex-based driver design.
