# Streaming Policy: Preview vs Full Resolution

This document defines intended use of `StreamQuality` for camera frame streaming.

## Quality modes

`StreamQuality` is applied server-side before frame transport:

- `Full` (`STREAM_QUALITY_FULL`): no downsampling, full sensor/ROI resolution
- `Preview` (`STREAM_QUALITY_PREVIEW`): 2x2 spatial averaging, ~4x fewer pixels
- `Fast` (`STREAM_QUALITY_FAST`): 4x4 spatial averaging, ~16x fewer pixels

Implementation details:

- gRPC observer downsampling selection:
  - `crates/server/src/grpc/hardware_service/streaming.rs`
- Downsampling kernels:
  - `crates/protocol/src/downsample.rs` (`downsample_2x2`, `downsample_4x4`)
- LZ4 compression happens after downsampling in forwarding pipeline:
  - `crates/server/src/grpc/hardware_service/mod.rs`

### Bandwidth implications

For fixed bit depth and frame rate, transport bytes scale approximately with pixel count:

- `Full`: baseline (1.0x)
- `Preview`: ~0.25x of Full
- `Fast`: ~0.0625x of Full

Actual network usage is typically lower due to LZ4 compression, but the relative ordering above remains the same.

## Recommended defaults by use case

| Use case | Recommended quality | Typical `max_fps` | Rationale |
|---|---:|---:|---|
| Live preview / interactive focusing | `Fast` | 15-60 | Lowest transport/CPU cost, best UI responsiveness |
| Experiment monitoring dashboards | `Preview` | 10-30 | Better detail than Fast with large bandwidth savings |
| Remote/thin client (VPN/tablet/Wi‑Fi) | `Fast` | 10-30 | Most robust under constrained links |
| Recording/archival stream in UI | `Preview` or `Fast` | 10-30 | Keep GUI stream lightweight; preserve science data via storage pipeline, not GUI stream |
| Scientific pixel-level analysis from stream | `Full` | As needed | Use only when exact spatial detail is required |

Policy summary:

- Default to `Fast` for generic UI image viewing.
- Use `Preview` for experiment designer/live-plot contexts where moderate detail helps.
- Reserve `Full` for explicit analysis/harness/testing workflows.

## `max_fps` interaction with quality

`max_fps` and `quality` are orthogonal controls:

- `quality` reduces bytes per frame (spatial resolution)
- `max_fps` reduces frames per second (temporal rate)

Effective throughput is approximately:

`throughput ∝ (pixels per frame after quality) × (effective fps after max_fps/backpressure)`

Guidance:

- Prefer reducing `quality` first when bandwidth/CPU constrained.
- Then cap `max_fps` to what the UI can render/consume.
- For stable remote operation, pair `Fast` with capped FPS.

## Backpressure and frame dropping

Frame streaming is intentionally lossy under load to preserve responsiveness:

1. Observer path uses non-blocking `try_send()`; full channel drops frames.
   - `GrpcStreamObserver::on_frame()` in `crates/server/src/grpc/hardware_service/streaming.rs`
2. Forwarding task drops when rate-limit (`max_fps`) is exceeded.
3. Forwarding task also drops when gRPC queue is near saturation.
4. Compression queue uses `try_send()` and drops if compressor is backlogged.

This is expected behavior for live visualization; consumers should not assume every acquired frame is delivered.

## Tap registry decimation (`nth_frame`)

Storage/ring-buffer taps support independent temporal decimation via `nth_frame`:

- `nth_frame = 1`: deliver every frame
- `nth_frame = N`: deliver every Nth frame

Relevant code:

- `crates/storage/src/tap_registry.rs`
- `crates/storage/src/ring_buffer.rs`
- gRPC tap status surface (`TapStatus.nth_frame`):
  - `crates/protocol/proto/storage.proto`
  - `crates/server/src/grpc/storage_service.rs`

Use `nth_frame` in addition to stream quality/rate controls when downstream consumers need lower ingest rates.

## Where to change defaults in code

Current defaults are already aligned with the policy:

- Image viewer default stream quality: `Fast`
  - `crates/ui/src/panels/image_viewer/mod.rs` (`stream_quality: StreamQuality::Fast`)
- Experiment designer live visualization: `Preview` at 30 FPS
  - `crates/ui/src/panels/experiment_designer.rs` (`stream_frames(..., 30, StreamQuality::Preview)`)
- UI quality picker options and labels:
  - `crates/ui/src/panels/image_viewer/mod.rs`
  - `crates/ui/src/panels/image_viewer/types.rs`
- Client API surface:
  - `crates/client/src/client.rs` (`stream_frames(device_id, max_fps, quality)`)

Intentionally `Full` users found during verification:

- `crates/bin/src/bin/hardware_smoke.rs` (diagnostic/smoke path)
- `crates/bin/src/bin/pvcam_grpc_harness.rs` (harness validation path)

No UI default path currently hardcodes `Full`.
