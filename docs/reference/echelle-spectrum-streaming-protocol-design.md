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

## Implementation Phases

1. Proto message definition + metadata fields
2. Server serialization path for `Measurement::Spectrum`
3. Client deserialization and subscription handling
4. UI plotting support for streamed spectra
5. Benchmark and optimize if payload size becomes limiting

## Validation Checklist for Protocol Implementation

- x/y lengths match
- units preserved end-to-end
- metadata preserved for per-order vs merged spectra
- large arrays do not stall UI/client event loops
- backward compatibility with older clients is maintained
