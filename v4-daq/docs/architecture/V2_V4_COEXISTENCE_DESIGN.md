# V2/V4 Coexistence Architecture Design

## 1. Executive Summary

This document describes the strategy for running V2 and V4 actors simultaneously during the gradual migration from V2 to V4. Phase 1D has completed V4 implementations for SCPI, ESP300, and PVCAM actors. V2 actors still exist and must continue operating until they are fully replaced.

The coexistence strategy enables:
- **Parallel operation** of V2 and V4 instruments in the same application
- **Gradual migration** of instruments from V2 to V4 actor models
- **Zero breaking changes** to V2 users during transition period
- **Clear migration path** for each instrument type with identified blockers

Status: **DESIGN PHASE** - Ready for implementation in Phase 1E+

---

## 2. Architecture Overview

### 2.1 Dual-Actor Model

```
┌─────────────────────────────────────────────────────────────────┐
│                        Application                              │
│                    (DaqManagerActor or GUI)                     │
└────────────────────────┬────────────────────────────────────────┘
                         │
        ┌────────────────┴────────────────┐
        │                                  │
        ▼                                  ▼
   ┌─────────────┐               ┌──────────────────┐
   │   V2 Stack  │               │    V4 Stack      │
   │             │               │                  │
   │ DaqManager  │               │ InstrumentMgr    │
   │ (tokio ch)  │               │ (Kameo Actor)    │
   │             │               │                  │
   │  V2 Instrs  │               │  V4 Instrs       │
   │  (tokio)    │               │  (Kameo Actors)  │
   │             │               │                  │
   │ DataDist    │               │ DataPublisher    │
   │ (broadcast) │               │ (pub/sub)        │
   └──────┬──────┘               └────────┬─────────┘
          │                               │
          │    Shared Resources           │
          │  (Serial ports, VISA)         │
          │                               │
          └───────────┬───────────────────┘
                      ▼
          ┌──────────────────────┐
          │  Hardware Adapters   │
          │  (Serial, VISA, etc) │
          │                      │
          │  Resource Pools      │
          │  (Mutexes/Channels)  │
          └──────────────────────┘
```

### 2.2 Runtime Topology

Two independent runtime loops coexist with **shared hardware resources**:

```
Main Application Thread
│
├── V2 Subsystem
│   └── DaqManagerActor (tokio task)
│       ├── V2 Instrument Tasks (tokio)
│       │   ├── Newport1830C (serial)
│       │   └── Elliptec (serial)
│       │
│       ├── Storage Writer
│       │
│       └── DataDistributor (broadcast channel)
│
├── V4 Subsystem
│   └── Kameo Runtime (independent)
│       ├── InstrumentManager (Kameo)
│       │   ├── SCPI Instruments (Kameo)
│       │   ├── ESP300 (Kameo, serial)
│       │   └── PVCAM (Kameo)
│       │
│       ├── DataPublisher (Kameo)
│       │
│       └── HDF5Storage (Kameo)
│
└── Shared Hardware Resources
    ├── Serial Port Pool (Arc<Mutex>)
    ├── VISA Session Manager
    └── Device Enumeration Cache
```

---

## 3. Core Design Principles

### 3.1 Independence & Isolation

V2 and V4 are operationally independent:
- **Separate actor systems**: V2 uses tokio tasks, V4 uses Kameo actors
- **Separate supervisors**: Each has its own lifecycle management
- **Separate data paths**: DataDistributor (V2) vs DataPublisher (V4)
- **No direct communication**: Between V2 and V4 actors

### 3.2 Resource Sharing Strategy

Shared hardware resources use **Arc<Mutex<>>** for safe concurrent access:

**Serial Ports:**
```rust
// Shared across V2 and V4
pub struct SerialPortPool {
    ports: Arc<Mutex<HashMap<String, Box<dyn SerialPort>>>>,
}
```

**VISA Sessions:**
```rust
// Single VISA session (VISA is NOT thread-safe)
pub struct VisaSessionManager {
    session: Arc<Mutex<Option<visa_rs::Session>>>,
}
```

**Device Cache:**
```rust
// Enumeration results cached and shared
pub struct DeviceCache {
    instruments: Arc<Mutex<Vec<InstrumentInfo>>>,
}
```

### 3.3 Message Passing (Not Actor Communication)

V2↔V4 communication flows through **shared data structures**, not actor messages:

```rust
// Pseudo-code: Bridge for measurement exchange

// V4 writes measurements to shared channel
let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

// V2 receives from shared channel (optional)
tokio::spawn(async move {
    while let Some(batch) = rx.recv().await {
        v2_distributor.broadcast(batch);
    }
});
```

**No request/response** between V2 and V4 actors.
Only **asynchronous data flow** through channels.

---

## 4. Detailed Coexistence Strategy

### 4.1 Actor Spawning

Both V2 and V4 need independent spawning mechanisms in the configuration.

**Configuration Structure:**
```toml
[application]
name = "rust-daq"
log_level = "info"

[v2]
enabled = true

[v4]
enabled = true

[[v2.instruments]]
id = "elliptec1"
type = "Elliptec"
serial_port = "/dev/ttyUSB0"

[[v4.instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
resource = "TCPIP0::192.168.1.100::INSTR"

[[v4.instruments]]
id = "pvcam_camera"
type = "PVCAMInstrument"
camera_name = "pvcam0"
```

**Spawning Logic in Application:**

```rust
pub struct DualRuntimeManager {
    v2_actor: Option<ActorHandle>,
    v4_manager: Option<ActorRef<InstrumentManager>>,
    config: AppConfig,
}

impl DualRuntimeManager {
    pub async fn start(&mut self) -> Result<()> {
        // Start V2 subsystem (tokio-based)
        if self.config.v2.enabled {
            let v2_handle = self.spawn_v2_subsystem().await?;
            self.v2_actor = Some(v2_handle);
        }

        // Start V4 subsystem (Kameo-based)
        if self.config.v4.enabled {
            let v4_mgr = self.spawn_v4_subsystem().await?;
            self.v4_manager = Some(v4_mgr);
        }

        Ok(())
    }
}
```

### 4.2 Supervisor Hierarchy

**V2 Supervisor:**
```
DaqManagerActor
├── Newport1830C (JoinHandle)
├── Elliptec (JoinHandle)
├── StorageWriter (JoinHandle)
└── DataDistributor (broadcast channel)
```

**V4 Supervisor:**
```
InstrumentManager (Kameo)
├── SCPI Instruments (Kameo)
├── ESP300 (Kameo)
├── PVCAM (Kameo)
├── DataPublisher (Kameo)
└── HDF5Storage (Kameo)
```

**Key Differences:**

| Aspect | V2 | V4 |
|--------|----|----|
| **Supervisor** | DaqManagerActor (tokio) | InstrumentManager (Kameo) |
| **Spawn** | `tokio::spawn()` | `kameo::spawn()` |
| **Lifecycle** | JoinHandle + channels | ActorRef supervision |
| **Monitoring** | Manual task tracking | Kameo's automatic supervision |
| **Restart Policy** | Manual | Kameo's configurable policies |

### 4.3 Message Passing & Data Flow

**V2 Data Path:**
```
V2 Instrument → DataDistributor (broadcast) → Subscribers
                    ↓
              (ringbuf channels)
                    ↓
              Storage Writer + GUI
```

**V4 Data Path:**
```
V4 Instrument (Kameo) → InstrumentManager → DataPublisher (pub/sub)
                              ↓
                        (Arrow batches)
                              ↓
              HDF5Storage + Analysis + GUI
```

**Bridge (Optional for V2→V4):**
```rust
// If GUI needs to display both V2 and V4 data:

pub struct DataBridge {
    v2_rx: mpsc::UnboundedReceiver<Arc<Measurement>>,
    v4_rx: mpsc::UnboundedReceiver<RecordBatch>,
    gui_tx: mpsc::UnboundedSender<UnifiedDataPoint>,
}

impl DataBridge {
    pub async fn run(&mut self) {
        loop {
            select! {
                Some(v2_meas) = self.v2_rx.recv() => {
                    let unified = self.convert_v2_to_unified(&v2_meas);
                    let _ = self.gui_tx.send(unified);
                }
                Some(v4_batch) = self.v4_rx.recv() => {
                    let unified = self.convert_v4_to_unified(&v4_batch);
                    let _ = self.gui_tx.send(unified);
                }
            }
        }
    }
}
```

### 4.4 Resource Sharing

**Serial Ports (Critical):**

Problem: Two actors cannot simultaneously hold open a serial port.

Solution: **Shared Pool with Arc<Mutex<>>**

```rust
pub struct SharedSerialPort {
    port: Arc<Mutex<Box<dyn SerialPort>>>,
    // Track owner for debugging
    owner: Arc<parking_lot::RwLock<Option<String>>>,
}

impl SharedSerialPort {
    pub async fn acquire(&self, owner_id: &str) -> Result<SerialGuard<'_>> {
        let mut guard = self.port.lock().await;
        // Check not already owned
        let mut owner = self.owner.write();
        if owner.is_some() {
            return Err(anyhow!("Serial port already in use"));
        }
        *owner = Some(owner_id.to_string());
        Ok(SerialGuard { port: guard, owner: self.owner.clone() })
    }
}

pub struct SerialGuard {
    port: MutexGuard<Box<dyn SerialPort>>,
    owner: Arc<parking_lot::RwLock<Option<String>>>,
}

impl Drop for SerialGuard {
    fn drop(&mut self) {
        let mut owner = self.owner.write();
        *owner = None;
    }
}
```

**VISA Sessions:**

Problem: VISA SDK is inherently single-session (not thread-safe).

Solution: **Single Session Manager with Queueing**

```rust
pub struct VisaSessionManager {
    session: Arc<Mutex<Option<visa_rs::Session>>>,
    queue: Arc<tokio::sync::Mutex<VecDeque<VisaCommand>>>,
    worker_task: JoinHandle<()>,
}

impl VisaSessionManager {
    pub async fn enqueue(&self, cmd: VisaCommand) -> Result<VisaResponse> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.queue.lock().await.push_back(QueuedCommand { cmd, response_tx: tx });
        rx.await?
    }
}
```

This ensures VISA operations are serialized even when called from both V2 and V4.

### 4.5 Shutdown & Lifecycle

**Graceful Shutdown Protocol:**

```rust
impl DualRuntimeManager {
    pub async fn shutdown(&mut self) -> Result<()> {
        // Phase 1: Stop accepting new work
        tracing::info!("Shutting down dual runtime");

        // Phase 2: Stop V4 (Kameo handles supervised shutdown)
        if let Some(ref mgr) = self.v4_manager {
            mgr.cast(Shutdown)?; // Fire-and-forget
            // Kameo automatically stops child actors
        }

        // Phase 3: Stop V2 (manual coordination)
        if let Some(ref mut handle) = self.v2_actor {
            // Send shutdown signal
            self.v2_cmd_tx.send(DaqCommand::Shutdown).await?;

            // Wait with timeout
            tokio::time::timeout(
                Duration::from_secs(5),
                handle.join()
            ).await??;
        }

        // Phase 4: Release shared resources
        self.release_hardware_resources().await?;

        Ok(())
    }
}
```

**Shutdown Order:**
1. V4 actors (Kameo supervision handles orderly shutdown)
2. V2 actors (manual message-based shutdown)
3. Shared resources (serial ports, VISA session)

---

## 5. Data Flow & Integration Points

### 5.1 Hardware Resource Access Pattern

```rust
// Both V2 and V4 follow this pattern:

pub struct InstrumentWithSharedSerial {
    id: String,
    shared_port: Arc<SharedSerialPort>,
}

impl InstrumentWithSharedSerial {
    async fn query(&self, cmd: &str) -> Result<String> {
        let guard = self.shared_port.acquire(&self.id).await?;
        let response = guard.port.query(cmd).await?;
        // Guard dropped here, serial port released
        Ok(response)
    }
}
```

Key properties:
- **Non-blocking acquisitions** through async locks
- **Automatic release** via RAII (Drop trait)
- **Ownership tracking** for debugging
- **Timeout protection** to prevent deadlocks

### 5.2 Measurement Flow for Unified GUI

If the GUI needs to display measurements from both V2 and V4:

```rust
pub enum MeasurementSource {
    V2(Arc<Measurement>),
    V4(RecordBatch),
}

pub struct UnifiedMeasurement {
    instrument_id: String,
    timestamp: SystemTime,
    data: MeasurementSource,
}

// In GUI event loop:
loop {
    select! {
        Some(v2_meas) = v2_rx.recv() => {
            let unified = UnifiedMeasurement {
                instrument_id: v2_meas.instrument_id.clone(),
                timestamp: v2_meas.timestamp,
                data: MeasurementSource::V2(v2_meas),
            };
            gui_state.update(unified)?;
        }
        Some(v4_batch) = v4_rx.recv() => {
            for row in v4_batch.iter_rows() {
                let unified = UnifiedMeasurement {
                    instrument_id: row.get("instrument_id").to_string(),
                    timestamp: row.get("timestamp"),
                    data: MeasurementSource::V4(row),
                };
                gui_state.update(unified)?;
            }
        }
    }
}
```

---

## 6. Migration Path

### 6.1 Instrument-by-Instrument Migration

Each instrument follows this lifecycle:

```
PHASE X: Instrument in V2
    ├── Active in V2 subsystem
    ├── Configuration in V2 format
    └── Data via DataDistributor

PHASE X+1: Dual-Actor (Development)
    ├── V4 implementation developed
    ├── Integration tests in V4
    ├── Runs in parallel with V2
    └── Shadowing (optional)

PHASE X+2: Gradual Cutover
    ├── Configuration migration tool created
    ├── V2 disabled in config
    ├── V4 enabled in config
    ├── All measurements via V4
    └── V2 instance kept as fallback

PHASE X+3: Complete Migration
    ├── V2 code deleted
    ├── V2 configuration schema removed
    └── Full V4 implementation
```

### 6.2 Instrument Priority Order

Based on complexity and V4 readiness:

**Priority 1 (Phase 1D - COMPLETED):**
- SCPI Instruments (generic)
- ESP300 (motion controller)
- PVCAM (camera)

**Priority 2 (Phase 1E - Ready):**
- Newport 1830C (power meter) - V4 implementation exists
- MaiTai (tunable laser) - V4 implementation exists

**Priority 3 (Phase 1F - Design):**
- Elliptec (motion controller) - Needs V4 implementation
- Additional VISA instruments

**Priority 4 (Phase 2+):**
- Custom instruments
- Legacy hardware

### 6.3 Per-Instrument Checklist

For each instrument migration:

```
[ ] V4 actor implementation
[ ] Hardware adapter (serial/VISA) working
[ ] Integration tests pass
[ ] Configuration schema defined
[ ] Measurement format compatible
[ ] Error handling equivalent to V2
[ ] Shutdown behavior tested
[ ] Documentation updated
[ ] Migration script created
[ ] Shadow mode testing (optional)
[ ] Parallel operation testing (V2 + V4)
[ ] V2 code deletion scheduled
```

---

## 7. Configuration Management

### 7.1 Dual Configuration Files

**config.v2.toml** (Existing)
```toml
[instruments]
[[instruments.list]]
id = "newport1"
type = "Newport1830C"
port = "/dev/ttyUSB0"
```

**config.v4.toml** (New)
```toml
[application]
name = "rust-daq"

[actors]
default_mailbox_capacity = 100
spawn_timeout_ms = 5000

[instruments]
[[instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
config.resource = "TCPIP0::192.168.1.100::INSTR"
```

**Unified config (Optional):**
```toml
[v2]
enabled = true

[v4]
enabled = true

# Mixed instrument definitions...
```

### 7.2 Feature Flags

Control subsystem activation:

```rust
#[cfg(feature = "v2")]
pub mod v2_subsystem;

#[cfg(feature = "v4")]
pub mod v4_subsystem;

#[cfg(all(feature = "v2", feature = "v4"))]
pub struct DualRuntimeManager {
    v2: v2_subsystem::Subsystem,
    v4: v4_subsystem::Subsystem,
}
```

---

## 8. Risks & Mitigation

### 8.1 Resource Conflicts

**Risk:** Two actors try to use same serial port simultaneously.

**Mitigation:**
- Arc<Mutex<>> enforces exclusive access
- Owner tracking for debugging
- Timeout detection for deadlock prevention
- Unit tests for contention scenarios

### 8.2 VISA Session Limitations

**Risk:** VISA SDK single-session limitation blocks parallel usage.

**Mitigation:**
- Serialize all VISA commands through VisaSessionManager
- Message queuing with response channels
- Per-command timeout
- Fallback to mock mode if session unavailable

### 8.3 Data Format Incompatibility

**Risk:** V2 and V4 produce different measurement formats, GUI confusion.

**Mitigation:**
- Define unified measurement format early
- Conversion layers (V2→Unified, V4→Unified)
- GUI displays unified format regardless of source
- End-to-end tests with both data sources

### 8.4 Shutdown Ordering Issues

**Risk:** One subsystem shuts down before releasing shared resources.

**Mitigation:**
- Explicit shutdown sequence in DualRuntimeManager
- Timeouts on all shutdown operations
- Forced cleanup of locks (parking_lot)
- Cleanup verification tests

### 8.5 Configuration Complexity

**Risk:** Users confused by dual configuration system.

**Mitigation:**
- Migration tools that auto-convert V2 → V4 config
- Configuration validation with clear error messages
- Documentation with examples for both modes
- Deprecation warnings when V2 config used

---

## 9. Implementation Roadmap

### Phase 1E: Core Coexistence Infrastructure
- [ ] Create DualRuntimeManager
- [ ] Implement SharedSerialPort wrapper
- [ ] Implement VisaSessionManager
- [ ] Add feature flags for v2/v4
- [ ] Basic integration tests

### Phase 1F: Per-Instrument Migration
- [ ] Newport1830C → V4 (design review)
- [ ] Elliptec → V4 (implementation)
- [ ] Shadow mode for testing
- [ ] Configuration migration tools

### Phase 2: Production Coexistence
- [ ] Long-running stability tests
- [ ] Performance profiling (resource contention)
- [ ] Documentation and user guides
- [ ] Training/migration support

### Phase 3: V2 Deprecation
- [ ] Announce V2 deprecation timeline
- [ ] Provide automatic migration tools
- [ ] Support period (e.g., 2-3 releases)
- [ ] Scheduled V2 code deletion

---

## 10. Code Structure

### New Modules

```
src/
├── dual_runtime/
│   ├── mod.rs (DualRuntimeManager)
│   ├── shared_serial.rs (SharedSerialPort)
│   ├── shared_visa.rs (VisaSessionManager)
│   ├── device_cache.rs (DeviceCache)
│   └── measurement_bridge.rs (MeasurementSource bridge)
│
├── config/
│   ├── v2.rs (existing - no changes)
│   ├── v4.rs (existing in v4-daq crate)
│   └── unified.rs (new - optional unified config)
│
└── tests/
    ├── coexistence_tests.rs
    ├── serial_contention_tests.rs
    ├── visa_queueing_tests.rs
    └── shutdown_tests.rs
```

### Key Types (Pseudo-code)

```rust
// Shared hardware resources
pub struct SharedSerialPort {
    port: Arc<Mutex<Box<dyn SerialPort>>>,
    owner: Arc<RwLock<Option<String>>>,
}

pub struct VisaSessionManager {
    session: Arc<Mutex<Option<visa_rs::Session>>>,
    queue: Arc<Mutex<VecDeque<VisaCommand>>>,
}

pub struct DeviceCache {
    instruments: Arc<Mutex<Vec<InstrumentInfo>>>,
    last_refresh: Arc<Mutex<SystemTime>>,
}

// Manager
pub struct DualRuntimeManager {
    v2_actor: Option<DaqManagerActor>,
    v4_manager: Option<ActorRef<InstrumentManager>>,
    shared_serial: Arc<SharedSerialPort>,
    shared_visa: Arc<VisaSessionManager>,
    device_cache: Arc<DeviceCache>,
}

// Bridge (optional)
pub struct MeasurementBridge {
    v2_rx: mpsc::UnboundedReceiver<Arc<Measurement>>,
    v4_rx: mpsc::UnboundedReceiver<RecordBatch>,
    gui_tx: mpsc::UnboundedSender<UnifiedMeasurement>,
}
```

---

## 11. Testing Strategy

### 11.1 Unit Tests

- SharedSerialPort RAII semantics
- VisaSessionManager queueing
- DeviceCache refresh logic
- Measurement format conversions

### 11.2 Integration Tests

- Both subsystems starting simultaneously
- Serial port access from V2 and V4
- VISA command serialization under load
- Data flow through bridge
- Graceful shutdown sequence

### 11.3 Stress Tests

- Rapid acquire/release of serial ports
- High-frequency VISA commands from both subsystems
- Memory/CPU usage under dual-subsystem load
- Long-running stability (hours/days)

### 11.4 Scenario Tests

```rust
#[tokio::test]
async fn test_v2_v4_serial_contention() {
    // Spawn V2 instrument using /dev/ttyUSB0
    // Spawn V4 instrument using /dev/ttyUSB0
    // Verify exclusive access enforced
    // Verify no deadlocks under load
}

#[tokio::test]
async fn test_visa_serialization() {
    // Enqueue VISA commands from V2 thread
    // Enqueue VISA commands from V4 actor
    // Verify all commands execute in order
    // Verify no command loss
}

#[tokio::test]
async fn test_measurement_bridge() {
    // Emit V2 measurement
    // Emit V4 measurement
    // Verify both arrive at GUI
    // Verify correct timestamp ordering
}

#[tokio::test]
async fn test_shutdown_sequence() {
    // Start V2 and V4
    // Send shutdown to DualRuntimeManager
    // Verify V4 shuts down first (Kameo)
    // Verify V2 shuts down cleanly
    // Verify all resources released
}
```

---

## 12. Conclusion

The V2/V4 coexistence architecture enables:

1. **Parallel operation** of both subsystems with safe resource sharing
2. **Gradual migration** path with clear per-instrument checkpoints
3. **Zero breaking changes** for V2 users during transition
4. **Production stability** with comprehensive testing strategy
5. **Clear documentation** for users and developers

The key innovation is **Arc<Mutex<>> resource sharing** combined with **message-based inter-subsystem communication**, allowing independent actor models to safely coexist without direct coupling.

Implementation should begin with Phase 1E coexistence infrastructure, followed by per-instrument migrations using the provided checklist and configuration tools.

---

## Appendix A: Glossary

- **Actor**: An isolated, message-driven concurrent entity (V4 uses Kameo)
- **DataDistributor**: V2's broadcast channel for measurements
- **DataPublisher**: V4's pub/sub actor for Arrow batches
- **Kameo**: Fault-tolerant actor framework (V4)
- **RAII**: Resource Acquisition Is Initialization (Rust pattern)
- **Supervision**: Automatic restart/monitoring of actors
- **VISA**: Virtual Instrument Software Architecture (instrument control standard)

## Appendix B: Related Documents

- `ARCHITECTURE.md` - Overall V4 architecture overview
- `V4Config` documentation - Configuration system details
- `InstrumentManager` design - V4 instrument supervision
- Phase 1D completion report - SCPI/ESP300/PVCAM implementations
