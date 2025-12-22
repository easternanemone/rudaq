# ADR: gRPC Validation Layer

**Status:** Proposed
**Date:** 2025-12-22
**Context:** The current gRPC API lacks input validation, allowing malformed or dangerous data to penetrate deep into the system, causing unclear errors or potential safety issues.

## 1. Validation Layer Overview

We need a defense-in-depth approach to validation that catches errors as close to the network edge as possible.

### Location
Validation should occur **immediately upon request receipt**, before any business logic execution.
- **Syntactic Validation:** (Format, range, size) should be stateless and fast.
- **Semantic Validation:** (Device existence, state checks) requires system state access.

### Recommendation: Hybrid Approach
1.  **Proto-driven Stateless Validation:** Use Protocol Buffer annotations to define constraints (e.g., `min_len`, `range`).
2.  **Service-level Semantic Validation:** explicit checks within service handlers for logical consistency.

## 2. Validation Categories

### A. Size Limits (DoS Prevention)
*   **Strings:** `script_content`, `metadata` values.
    *   *Rule:* Max 1MB for scripts, 4KB for generic strings.
*   **Collections:** `repeated string channels`, `map<string, string>`.
    *   *Rule:* Max 100 items per list/map to prevent loop exhaustion.

### B. Range Checks (Safety)
*   **Motion:** `MoveRequest.value` must be within soft limits (though hardware enforces hard limits, API should reject obvious nonsense like `NaN` or `Inf`).
*   **Timeouts:** `timeout_ms` must be < 60,000 (1 minute) to prevent hanging connections.
*   **Rates:** `max_rate_hz` limited to system capability (e.g., 1000Hz).

### C. Format Validation (Correctness)
*   **Device IDs:** Alphanumeric + underscores only. Regex: `^[a-z0-9_]+$`.
*   **JSON Fields:** `DeviceCommandRequest.args` must be valid JSON.
*   **Enums:** Proto deserialization handles this, but "UNSPECIFIED" values should be explicitly rejected.

### D. Semantic Validation (Logic)
*   **Existence:** `device_id` must exist in the registry.
*   **Capability:** Target device must support the requested operation (e.g., `Move` called on a `Readable` only device).

## 3. Implementation Options

### Option A: Tonic Interceptor with Dynamic Inspection
Use a generic interceptor that reflects on messages.
*   *Pros:* Centralized, no code changes in handlers.
*   *Cons:* High performance cost (reflection), complex to implement in Rust/Prost (no native reflection support without hefty overhead), type erasure issues.

### Option B: Per-Service Validation Trait (Recommended Phase 1)
Define a `Validate` trait and implement it for Request structs.
```rust
pub trait ValidateRequest {
    fn validate(&self) -> Result<(), tonic::Status>;
}

// In service handler:
async fn move_absolute(&self, request: Request<MoveRequest>) -> Result<Response<MoveResponse>, Status> {
    let req = request.into_inner();
    req.validate()?; // Fail fast
    // ... logic
}
```
*   *Pros:* Explicit, simpler to debug, zero magic, high performance.
*   *Cons:* Boilerplate (manual implementation initially).

### Option C: Generated Validators (Recommended Phase 2)
Use `protoc-gen-validate` (PGV) or `buf-validate` to generate the `Validate` trait implementations automatically from `.proto` comments/annotations.
*   *Pros:* Single source of truth (the proto file), consistent rules across languages.
*   *Cons:* Requires build system integration (`prost-build` config).

### Final Recommendation
Start with **Option B (Trait)** for immediate control over critical endpoints (`HardwareService`). Migrate to **Option C** once the build pipeline is robust enough to support extra codegen steps.

## 4. Error Handling

### Status Codes
*   `INVALID_ARGUMENT (3)`: Malformed data, regex failure, range error.
*   `NOT_FOUND (5)`: `device_id` does not exist.
*   `FAILED_PRECONDITION (9)`: Device is busy, not staged, or lacks capability.

### Error Format
Return clear, field-scoped error messages.
*   *Standard:* "Field 'exposure_ms' must be positive, got -5.0"
*   *Debug:* Include specific limits in the message.

## 5. Validation Rules (Proto Annotations)

Future-proofing for Option C:

```protobuf
message MoveRequest {
  string device_id = 1 [(validate.rules).string.pattern = "^[a-z0-9_]+$"];
  double value = 2 [(validate.rules).double = {ignore_empty: false, no_inf: true, no_nan: true}];
  optional uint32 timeout_ms = 4 [(validate.rules).uint32 = {lte: 60000}];
}

message UploadRequest {
  string script_content = 1 [(validate.rules).string.max_bytes = 1048576]; // 1MB
}
```

## 6. Migration Strategy

### Phase 1: Critical Path (Manual Trait)
1.  Define `trait ValidateRequest` in `daq-server/src/grpc/validation.rs`.
2.  Implement for `HardwareService` requests (`MoveRequest`, `SetExposureRequest`).
3.  Add calls to `req.validate()?` in `hardware_service.rs`.

### Phase 2: Expansion & JSON
1.  Add `serde_json` validation for "args" fields in `DeviceCommandRequest`.
2.  Implement for `ScriptService` (Upload limits).

### Phase 3: Automation (Prost Validate)
1.  Integrate `protoc-gen-validate` into `build.rs`.
2.  Replace manual impls with generated ones.
3.  Add CI step to reject protos without validation annotations.
