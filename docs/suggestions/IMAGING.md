Strategy: Pure Rust Vision and Image Processing

Historically, scientific imaging has relied heavily on OpenCV (C++). Wrapping C++ in Rust creates friction during builds, cross-compilation pain (especially for embedded targets), and safety risks.

The Solution: image and imageproc

The image crate provides foundational buffers, while imageproc provides advanced computer vision algorithms—both written entirely in safe Rust.

Benefits:

Memory Safety: Eliminates segfaults common with improperly freed C++ OpenCV cv::Mat objects.

Build Simplicity: Compiles effortlessly anywhere rustc runs. No cmake, no system dependencies.

Performance: Heavily optimized for modern CPUs using auto-vectorization (SIMD).

Implementation Strategy:

Frame Conversion: When driver-andor-sdk3 produces a raw 16-bit array, wrap it in an image::ImageBuffer<Luma<u16>, Vec<u16>>. This is a zero-cost abstraction.

On-the-fly Processing: Use imageproc for real-time Region of Interest (ROI) extraction, background subtraction, flat-field correction, or thresholding before the frame hits the UI or Zarr storage.

Visualization: Easily downsample or map 16-bit scientific data to 8-bit RGB color maps for the ui-slint rendering pipeline using image operations.
