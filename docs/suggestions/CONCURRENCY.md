Strategy: Concurrency and Low-Latency Pipelines

A DAQ system is essentially a massive data funnel. Hardware drivers produce data, processors mutate it, and sinks store or display it. Standard std::sync::mpsc channels are often not fast enough for high-speed camera or ADC feeds.

1. Flume / Crossbeam-Channel (Low-Latency Queues)

The Need: Passing pointers to multi-megabyte frames from the hardware acquisition threads to the processing/storage threads without dropping frames.
The Solution: flume (for async/sync mixing) or crossbeam-channel (pure sync).

Benefits: Unparalleled throughput. flume allows seamless bridging between synchronous hardware loops (which often cannot be async due to C-SDK constraints like PVCAM) and tokio async storage/network sinks.

Implementation Strategy: In your coordinator.rs, use flume::bounded channels with a carefully tuned capacity. The hardware thread pushes FrameData, and a pool of Tokio worker threads pops them for Zarr compression or UI updates.

2. Rayon (Data Parallelism)

The Need: CPU-heavy operations blocking the pipeline (e.g., calculating the mean over 10,000 pixels, or compressing data).
The Solution: rayon.

Benefits: Converts standard iterators to parallel iterators with just a few characters (changing .iter() to .par_iter()). It utilizes work-stealing to load-balance across all CPU cores perfectly.

Implementation Strategy: When applying a flat-field correction matrix to a mega-pixel image, or when searching a large buffer for threshold triggers, use Rayon to split the buffer across all available CPU cores.

3. Tokio (I/O & Network Orchestration)

The Need: Handling gRPC requests, writing to disk, and managing WebSocket/UI connections simultaneously.
The Solution: Maximize tokio (which you are already benchmarking in bench_tokio.rs).

Implementation Strategy: Ensure a strict separation of concerns. Do not run heavy math (like FFTs or Rayon parallel loops) directly inside Tokio async tasks, as this stalls the executor. Use tokio::task::spawn_blocking for CPU-bound work to keep your gRPC server (grpc_server.rs) highly responsive to hardware commands.
