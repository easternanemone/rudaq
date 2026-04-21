# crate: `server`

<!--
last-ingested: 2026-04-19
sources:
  - crates/server/
  - docs/reference/grpc-api.md
  - docs/explanation/architecture.md §Frame Streaming Pipeline
see-also:
  - ./client.md
  - ./protocol.md
  - ../concepts/ring-buffer.md
-->

**Role:** gRPC server (`tonic`). Exposes hardware control, script
execution, and data streaming to clients.

**Key modules:**

- `grpc/hardware_service/` — `HardwareServiceImpl`, dedicated LZ4 compression thread, streaming observers:
  - `mod.rs` — service impl + streaming glue.
  - `helpers.rs` — validation + proto conversions.
  - `streaming.rs` — `GrpcStreamObserver`, `ObserverFramePacket`, `StreamLimiter`.
- `grpc/storage_service.rs` — data-sink registration.
- `alerting.rs` — Slack / Discord webhook notifications on device fault, restart exhaustion, RunEngine abort. Rate-limited per device. Feature section in `config/config.v4.toml`.
- `health/heartbeat_log.rs` — JSONL per-minute vitals to `/tmp/rust_daq_heartbeat.jsonl` (CPU %, RSS, disk free, device health, RunEngine state, queue depth).

**Auth / CORS:** token-based auth; CORS configured for WASM client.

**Historical note:** `ScanServiceImpl` has been removed and replaced by
`RunEngineService`. Older docs listing it as "deprecated" describe a
migration that is already complete — grep returns 0 matches for
`ScanServiceImpl` in the current tree.

**Dependents:** `bin` (daemon entry), `integration-tests`.
