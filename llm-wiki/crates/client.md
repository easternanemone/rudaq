# crate: `client`

<!--
last-ingested: 2026-04-19
sources:
  - crates/client/Cargo.toml
  - crates/client/src/
see-also:
  - ./server.md
  - ./protocol.md
-->

**Role:** gRPC client library for the daemon. Typed API for remote
hardware control, streaming, device management.

**Binaries / consumers:**

- CLI client (`cargo run -p client --`).
- `ui` (via gRPC on native; via gRPC-web on WASM).
- `ui-slint` (experimental).

**Conventions:**

- Uses `protocol` for wire types.
- Returns `Result` with typed errors; do not `.unwrap()` network calls.
- Streaming APIs yield `impl Stream<Item = …>` — handle connection drops and backpressure.
