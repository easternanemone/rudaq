# Echelle Spectrum Streaming Protocol Design (gRPC)

This document captures the design for streaming full spectra/vector payloads
through rust-daq gRPC without scalarizing them.

This addresses the protocol design phase ahead of implementation.

## Problem

The current live-visualization path is scalar-oriented and does not carry full
`Measurement::Spectrum` arrays end-to-end for Image Viewer / live spectrum UX.

## Design Goals

- Preserve full x/y arrays (wavelength + flux)
- Carry units and metadata
- Support per-order and merged spectra
- Avoid lossy scalar summaries
- Remain compatible with future non-echelle spectra use cases

## Payload Options Considered

## Option A: `repeated double` arrays in protobuf (recommended for initial implementation)

- `repeated double x`
- `repeated double y`
- explicit units and metadata fields

Pros:

- straightforward protobuf/gRPC implementation
- debuggable
- no custom binary framing

Cons:

- larger payload size than packed binary

## Option B: packed binary payload (bytes + metadata)

Pros:

- better bandwidth efficiency

Cons:

- custom framing/endianness/versioning complexity
- harder debugging and tooling

## Option C: Arrow Flight / out-of-band vector transport

Pros:

- high-performance columnar transport
- good for larger analytics workflows

Cons:

- significant complexity increase
- larger architectural shift than needed for first spectrum streaming support

## Recommendation

Start with **Option A** (`repeated double`) and revisit optimization after
measuring real lab workloads.

## Transport Path Evaluation: Reuse `ModuleData` vs New Dedicated Spectrum RPC

This section records the evaluation requested for whether to reuse the existing
module-data live stream or introduce a dedicated spectrum streaming RPC.

## Option 1: Reuse `ModuleData` / `ModuleDataPoint`

Current shape (today):

- optimized for scalar time-series values (`map<string, double>`)
- naturally consumed by existing `LiveVisualizationPanel` line plots

Pros:

- no new subscription RPC
- fewer new moving parts in server/client auth/connection lifecycle

Cons:

- poor fit for vector payloads (`x[]`, `y[]`)
- risks breaking or bloating the scalar stream contract
- awkward metadata encoding for spectrum semantics (per-order vs merged, units, provenance)
- pushes UI/client complexity into ad-hoc decoding of pseudo-vector payloads

Conclusion:

- **Do not reuse `ModuleDataPoint` for full spectra**.
- Keep `ModuleData` for scalar telemetry and existing line-plot workflows.

## Option 2: Extend `StreamMeasurements` to carry array payloads end-to-end

Pros:

- conceptually aligned with `Measurement::Spectrum`
- may reduce duplicate server-side serialization code

Cons:

- current `StreamMeasurements` behavior may already be consumed by clients expecting scalarized summaries
- changing semantics in place creates compatibility risk
- protobuf message shape may not be suitable without a breaking change

Conclusion:

- preserve existing `StreamMeasurements` behavior for compatibility
- add a **new path** for true spectra rather than changing old semantics

## Option 3: New dedicated spectrum stream RPC (recommended)

Pros:

- clean contract for vector data (x/y arrays + units + metadata)
- explicit client opt-in
- safe backward compatibility with existing scalar streams
- easier capability negotiation and phased rollout

Cons:

- additional server/client/UI implementation work
- another RPC to maintain

Decision:

- **Recommended path:** introduce a new dedicated spectrum stream RPC and message type
- keep `ModuleData` and existing `StreamMeasurements` intact during migration

## Proposed Message Shape (Design)

Example conceptual message (`SpectrumDataPoint` placeholder name):

- `device_id`
- `stream_name`
- `name`
- `x_values` (`repeated double`)
- `y_values` (`repeated double`)
- `x_unit` (`string` optional)
- `y_unit` (`string` optional)
- `timestamp_ns`
- `metadata` (JSON string or typed fields)

## Echelle Metadata Fields (Required)

For echelle spectra (per-order or merged), include:

- `spectrum_kind` (`order` | `merged`)
- `relative_order_index` (optional for merged)
- `physical_order_number` (optional)
- `calibration_profile_id` (optional)
- `calibration_schema_version` (optional)
- `covered_samples`
- `total_samples`
- `saturated_samples`
- `mean_valid_fraction`
- `extraction_backend` (`rust_mvp` / sidecar backend name)

## Compatibility Strategy

- Add new message(s) and streaming path rather than overloading scalar-only types
- Preserve existing scalar live visualization behavior during rollout
- Gate new spectrum streaming path behind explicit client/server capability checks

## Backward Compatibility and Migration Plan (Existing `StreamMeasurements` Consumers)

This plan is intended to avoid breakage while enabling a new spectrum-capable
live path.

## Compatibility Rules

- Existing `StreamMeasurements` RPC semantics remain unchanged during the initial rollout.
- Existing `ModuleData`/scalar live visualization paths remain unchanged.
- Spectrum streaming is additive and opt-in.

## Capability Discovery (Recommended)

Server should advertise support for the new spectrum stream via one of:

- explicit gRPC capability RPC/field (preferred)
- versioned feature list in an existing capabilities response
- temporary feature flag configuration for internal rollout

Client behavior:

- if server supports spectrum streaming: use new RPC for spectrum panels
- otherwise: fall back to current scalar/preview behavior without erroring

## Rollout Phases

1. **Proto + server implementation**
- Add new `SpectrumDataPoint` (or equivalent) message and RPC.
- Keep old RPCs untouched.

2. **Client support (hidden/default-off)**
- Add deserialization + subscription plumbing.
- Feature-gate UI consumption path.

3. **UI integration**
- `LiveVisualizationPanel` (or dedicated spectrum panel mode) consumes new stream.
- Keep scalar line plots on existing path.

4. **Lab validation**
- Verify payload sizes, update cadence, UI responsiveness, and metadata correctness.

5. **Documentation + adoption guidance**
- Publish migration notes for downstream clients:
  - "existing scalar streams unchanged"
  - "new RPC required for full spectra"

## Deprecation Posture

- No immediate deprecation of `StreamMeasurements` or `ModuleData` for scalar use cases.
- Re-evaluate deprecation only after:
  - spectrum RPC is deployed and stable
  - downstream clients have migrated where needed
  - performance/operational behavior is characterized in lab conditions

## Implementation Phases

1. Proto message definition + metadata fields
2. Server serialization path for `Measurement::Spectrum`
3. Client deserialization and subscription handling
4. UI plotting support for streamed spectra
5. Benchmark and optimize if payload size becomes limiting

## Implementation Status (Current)

Implemented:

- additive `ControlService.StreamSpectra` RPC and spectrum payload messages
- server-side `Measurement::Spectrum` serialization path (full x/y arrays, units, metadata JSON)
- client `stream_spectra(...)` wrapper using dedicated no-timeout control streaming channel
- `LiveVisualizationPanel` spectrum plot mode with `SpectrumUpdate` channel support
- UI-side spectrum decimation and ring-buffering for high-rate updates

Remaining:

- wire a concrete gRPC subscription task in the UI workflow to feed `SpectrumUpdate`
- validate end-to-end live spectrum streaming UX against a real spectrum-producing source

## Validation Checklist for Protocol Implementation

- x/y lengths match
- units preserved end-to-end
- metadata preserved for per-order vs merged spectra
- large arrays do not stall UI/client event loops
- backward compatibility with older clients is maintained
