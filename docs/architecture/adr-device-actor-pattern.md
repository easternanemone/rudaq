# ADR: Device Actor Pattern for DeviceRegistry

## Status
Proposed (planning)

## Context
Today `HardwareServiceImpl` shares a global `Arc<RwLock<DeviceRegistry>>` across all gRPC services (`crates/daq-server/src/grpc/hardware_service.rs`). This introduces:
- **Contention**: All operations (list, query, command) compete for a single lock.
- **Deadlock risk**: Some RPCs must hold a read lock while issuing hardware commands that can block (e.g., commands that span device I/O or long hardware operations).
- **Coupling**: Callers must be careful about dropping locks before `.await` (see explicit drops in `hardware_service.rs`).

The DeviceRegistry itself stores live device instances and capabilities (`crates/daq-hardware/src/registry.rs`). This is a good fit for an actor-style owner that serializes access without shared locks.

## Decision
Adopt an **actor pattern** for device ownership and command execution:
- A **DeviceRegistryActor** owns the device map (registry responsibilities).
- A **DeviceActor** owns a single device instance (exclusive ownership of hardware driver + capability traits).
- gRPC services communicate via channels (mpsc for requests, oneshot for replies).
- All commands to a device are serialized by the DeviceActor task.

This replaces `Arc<RwLock<DeviceRegistry>>` with message passing and explicit request/response semantics.

## Actor Pattern Overview
- **Exclusive ownership**: DeviceActor owns `RegisteredDevice` and capability objects directly; no external borrowing or locks.
- **Message-driven**: All access occurs via `DeviceMsg` requests sent over `mpsc::Sender`.
- **Request/response**: Each request carries a `oneshot::Sender` for replies.
- **Timeouts**: Actor enforces command deadlines and returns `DeviceError::Timeout` or `Status::deadline_exceeded` as needed.

## Proposed Architecture
```
 gRPC Services ── CommandRequest ──▶ DeviceActor (owns device) ──▶ Hardware
       ▲                               │
       └──── CommandResponse/Error ◀───┘

 gRPC Services ── RegistryRequest ──▶ DeviceRegistryActor (owns map)
       ▲                               │
       └──── RegistryResponse/Error ◀──┘
```

Key points:
- **Per-device serialization**: Each DeviceActor serializes commands for a single device.
- **Registry operations**: DeviceRegistryActor handles list/lookup/register/unregister and spawns DeviceActors.
- **No shared mutable state**: All mutations live inside actors.

## Message Types (Rust pseudocode)

```rust
// Shared reply helper
pub type Reply<T> = tokio::sync::oneshot::Sender<Result<T, DeviceError>>;

// Registry-level messages
pub enum RegistryMsg {
    RegisterDevice {
        config: DeviceConfig,
        reply: Reply<DeviceInfo>,
    },
    UnregisterDevice {
        id: DeviceId,
        reply: Reply<bool>,
    },
    ListDevices {
        reply: Reply<Vec<DeviceInfo>>,
    },
    GetDeviceHandle {
        id: DeviceId,
        reply: Reply<Option<DeviceHandle>>,
    },
}

pub struct DeviceHandle {
    pub tx: tokio::sync::mpsc::Sender<DeviceMsg>,
}

// Device-level messages
pub enum DeviceMsg {
    ExecuteCommand {
        request: CommandRequest,
        reply: Reply<CommandResponse>,
    },
    QueryStatus {
        reply: Reply<DeviceStatus>,
    },
    Stage {
        reply: Reply<()>,
    },
    Unstage {
        reply: Reply<()>,
    },
    Shutdown {
        reply: Reply<()>,
    },
}

// Command payloads (examples, not exhaustive)
pub enum CommandRequest {
    MoveAbsolute { position: f64 },
    MoveRelative { delta: f64 },
    ReadValue,
    Trigger,
    StartExposure { ms: u64 },
    StopExposure,
    SetParameter { name: String, value: ParamValue },
}

pub enum CommandResponse {
    Ack,
    ReadValue { value: f64 },
    FrameReady { frame_id: u64 },
}
```

## Benefits
- **No shared mutable state**: Removes global `Arc<RwLock<DeviceRegistry>>` and associated contention.
- **Natural serialization**: Commands are serialized per-device, avoiding races between RPCs.
- **Timeout control**: Actor can enforce timeouts and return consistent errors.
- **Better observability**: Centralized place for logging, metrics, and tracing around each device.
- **Safer async**: No `.await` while holding registry locks.

## Migration Strategy
1. **Phase 1 — Introduce DeviceActor**
   - Implement DeviceRegistryActor + DeviceActor tasks.
   - Keep existing gRPC APIs; route calls through actors.
2. **Phase 2 — Migrate services incrementally**
   - Move high-traffic RPCs (move/read/trigger/stream) first.
   - Keep old registry access for remaining endpoints temporarily.
3. **Phase 3 — Remove Arc<RwLock<DeviceRegistry>>**
   - Delete legacy lock usage once all RPCs use actors.
   - Clean up tests to use DeviceActor handles in fixtures.

## Trade-offs
- **Latency**: Each call adds a channel hop and task scheduling overhead.
- **Complexity**: Requires new message types and actor lifecycle management.
- **Debuggability**: Failures may become asynchronous and less direct without good tracing.

**Benefits vs costs**: For a hardware-heavy system with long-running commands, the reduction in lock contention and deadlock risk outweighs the small channel overhead.

## Open Questions
- Should some operations bypass DeviceActor (read-only metadata) for lower latency?
- Should DeviceActor be per-device (recommended) or a single registry actor (simpler but serializes all devices)?
- Should streaming results (frames, observables) use dedicated broadcast channels instead of request/response?

## Implementation Notes (Non-goals)
This ADR is a planning document only; no code changes are proposed here.
