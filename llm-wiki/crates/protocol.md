# crate: `protocol`

<!--
last-ingested: 2026-04-19
sources:
  - crates/protocol/Cargo.toml
  - crates/protocol/src/
  - docs/reference/grpc-api.md
see-also:
  - ./server.md
  - ./client.md
  - ../concepts/ring-buffer.md
-->

**Role:** Protobuf wire format + domain↔proto conversion utilities +
frame compression helpers.

**Key exports:**

- Protobuf message types (`tonic` + `prost`).
- `compress_frame_into(frame, &mut buf)` / `decompress_frame_into(frame, &mut buf)` — buffer-reuse variants that write into pre-allocated `Vec<u8>` via `std::mem::swap`. **Use these** in hot paths; the allocating variants are wire-compatible fallbacks only.
- Domain↔proto converters for `DeviceInfo`, `FrameData`, document-stream types.

**Dependents:** `server`, `client`, `ui`, `experiment` (for doc-stream types).

**Conventions:**

- `.proto` sources compile via `build.rs`.
- Bumping a proto → bump version, write migration, coordinate server + client roll-out.
- Never break wire compatibility on a minor version.
