Strategy: Modern Data Processing and Mathematics

For a high-throughput DAQ system, data must be manipulated, filtered, and aggregated with minimal overhead. Relying on pure-Rust, battle-tested libraries ensures memory safety, removes C-binding bottlenecks, and guarantees cross-platform reproducibility.

1. Polars (High-Performance DataFrames)

The Need: rust-daq streams high volumes of telemetry and sensor data via Apache Arrow.
The Solution: polars is a lightning-fast DataFrame library built natively on Apache Arrow.

Benefits: \* Zero-Copy: Because Polars and your current DAQ pipeline both use Arrow memory layouts, transferring a batch of sensor readings from the DAQ memory buffer into a Polars DataFrame requires zero memory copying.

Lazy Evaluation: You can build a pipeline that aggregates, downsamples, or filters data (e.g., binning 1MHz ADC data to 1kHz for UI plotting) using Polars' multithreaded query optimizer.

Implementation Strategy: Integrate Polars into your DataSink or ProcessingPipeline traits. When a chunk of Arrow data completes, pass the ArrowArray pointer to Polars, run rolling averages or filtering, and push the processed DataFrame to the ui-slint frontend or zarrs storage.

2. rustfft & realfft (Frequency Analysis)

The Need: Real-time signal analysis (e.g., Fourier transforms) on ADC streams from the driver-comedi module.
The Solution: rustfft and realfft.

Benefits: Pure Rust, highly optimized FFT implementations. realfft is specifically optimized for real-valued arrays, which perfectly matches the output of hardware ADCs. It uses half the memory and computes much faster than standard complex-to-complex FFTs.

Implementation Strategy: Create an FftProcessor node in your data graph. When comedi pushes a chunk of 10,000 voltage readings, use realfft to compute the frequency spectrum and stream the result to a live plot in the UI.

3. nalgebra vs. ndarray (Linear Algebra & Tensors)

It is highly recommended to use both, but strictly separate their concerns:

ndarray (The Tensor Processor): Use this for N-dimensional data of arbitrary size known only at runtime.

Use Case: Processing 2D/3D camera frames from driver-pvcam or driver-andor-sdk3. It offers excellent Python-like slicing and dicing.

nalgebra (The Spatial/Kinematics Engine): Use this for strictly sized matrices known at compile time (e.g., 3x3 matrices, 3D vectors).

Use Case: Hardware motion control in driver-dover-motion and driver-esp300. It enforces dimensional correctness at compile-time, preventing mathematical panics when calculating stage kinematics, rotations, and multi-axis synchronizations.
