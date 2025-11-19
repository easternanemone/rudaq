# V2/V4 Coexistence Implementation Roadmap

## Overview

This document breaks down the implementation of V2/V4 coexistence into concrete, actionable tasks organized by phase. Each task has dependency information and estimated effort.

---

## Phase 1E: Core Coexistence Infrastructure

### Task 1E.1: DualRuntimeManager Foundation
**Effort:** 3-4 days
**Dependencies:** None
**Blocks:** All subsequent tasks

Create the central manager that coordinates both V2 and V4 subsystems.

**Deliverables:**
- `src/dual_runtime/mod.rs` - DualRuntimeManager struct
- `src/dual_runtime/lifecycle.rs` - Start/stop logic
- Feature flag integration
- Basic startup tests

**Key Implementation Details:**
```rust
pub struct DualRuntimeManager {
    config: DualConfig,
    v2_subsystem: Option<V2Subsystem>,
    v4_subsystem: Option<V4Subsystem>,
    shared_resources: Arc<SharedResources>,
}

impl DualRuntimeManager {
    pub async fn start(&mut self) -> Result<()> {
        if self.config.v2_enabled {
            self.start_v2().await?;
        }
        if self.config.v4_enabled {
            self.start_v4().await?;
        }
        Ok(())
    }
}
```

**Acceptance Criteria:**
- Both V2 and V4 start without conflicts
- Configuration correctly enables/disables each
- Graceful shutdown in reverse order
- No resource leaks detected in tests

---

### Task 1E.2: SharedSerialPort Implementation
**Effort:** 2-3 days
**Dependencies:** 1E.1
**Blocks:** Any serial instrument migration

Create the shared serial port wrapper with exclusive access enforcement.

**Deliverables:**
- `src/dual_runtime/shared_serial.rs`
- SerialPortPool for managing multiple ports
- SerialGuard RAII implementation
- Owner tracking and debugging support

**Key Implementation Details:**
```rust
pub struct SerialPortPool {
    ports: Arc<Mutex<HashMap<String, Arc<SerialPortEntry>>>>,
}

pub struct SerialPortEntry {
    port: Arc<Mutex<Box<dyn SerialPort>>>,
    owner: Arc<RwLock<Option<String>>>,
    stats: Arc<Mutex<AccessStats>>,
}

pub struct SerialGuard {
    entry: Arc<SerialPortEntry>,
    duration: Instant,
}

impl SerialGuard {
    async fn acquire(entry: Arc<SerialPortEntry>, owner: &str) -> Result<Self> {
        let mut owner_lock = entry.owner.write();
        if owner_lock.is_some() {
            return Err(anyhow!("Serial port in use by {:?}", owner_lock));
        }
        *owner_lock = Some(owner.to_string());
        Ok(SerialGuard { entry, duration: Instant::now() })
    }
}

impl Drop for SerialGuard {
    fn drop(&mut self) {
        let mut owner = self.entry.owner.write();
        *owner = None;

        // Record access stats
        let duration = self.duration.elapsed();
        self.entry.stats.lock().unwrap().record_access(duration);
    }
}
```

**Acceptance Criteria:**
- Multiple actors cannot simultaneously access same port
- Port is automatically released when guard drops
- Owner tracking works for debugging
- Timeout protection prevents indefinite waits
- Unit tests verify contention scenarios

---

### Task 1E.3: VisaSessionManager Implementation
**Effort:** 3-4 days
**Dependencies:** 1E.1
**Blocks:** VISA instrument migration

Create the VISA session manager with command queueing.

**Deliverables:**
- `src/dual_runtime/shared_visa.rs`
- VisaSessionManager with queuing
- Command serialization
- Response routing via oneshot channels

**Key Implementation Details:**
```rust
pub struct VisaCommand {
    id: Uuid,
    command: String,
    timeout: Duration,
    response_tx: oneshot::Sender<VisaResponse>,
}

pub struct VisaSessionManager {
    session: Arc<Mutex<Option<Session>>>,
    queue: Arc<Mutex<VecDeque<VisaCommand>>>,
    worker_task: JoinHandle<()>,
    stats: Arc<Mutex<SessionStats>>,
}

impl VisaSessionManager {
    pub async fn enqueue_command(
        &self,
        command: &str,
        timeout: Duration,
    ) -> Result<String> {
        let (tx, rx) = oneshot::channel();
        let cmd = VisaCommand {
            id: Uuid::new_v4(),
            command: command.to_string(),
            timeout,
            response_tx: tx,
        };

        self.queue.lock().unwrap().push_back(cmd);

        tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| anyhow!("VISA command timeout"))?
            .map_err(|e| anyhow!("VISA response error: {}", e))
    }

    async fn worker_loop(&self) {
        loop {
            let cmd = self.queue.lock().unwrap().pop_front();
            if let Some(cmd) = cmd {
                let response = self.execute_command(&cmd).await;
                let _ = cmd.response_tx.send(response);
            } else {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }
}
```

**Acceptance Criteria:**
- Commands from both V2 and V4 queue and execute in order
- Response routing via oneshot channels works correctly
- Timeouts prevent indefinite waits
- Session handle properly initialized/cleaned up
- Load tests show stable performance under concurrent load

---

### Task 1E.4: Shared Resources Container
**Effort:** 1-2 days
**Dependencies:** 1E.2, 1E.3
**Blocks:** Resource injection

Create the container holding all shared resources.

**Deliverables:**
- `src/dual_runtime/shared_resources.rs`
- SharedResources struct
- Access methods with timeout protection
- Resource initialization/cleanup

**Key Implementation Details:**
```rust
pub struct SharedResources {
    serial_pool: Arc<SerialPortPool>,
    visa_manager: Arc<VisaSessionManager>,
    device_cache: Arc<DeviceCache>,
    config: Arc<DualConfig>,
}

impl SharedResources {
    pub async fn acquire_serial(&self, port_path: &str, owner: &str) -> Result<SerialGuard> {
        self.serial_pool.acquire(port_path, owner).await
    }

    pub async fn execute_visa(&self, cmd: &str) -> Result<String> {
        self.visa_manager.enqueue_command(cmd, Duration::from_secs(2)).await
    }

    pub fn get_device_cache(&self) -> Arc<DeviceCache> {
        self.device_cache.clone()
    }
}
```

**Acceptance Criteria:**
- All shared resources accessible through single container
- Resource initialization order correct
- Cleanup on application shutdown
- Thread-safe access from all subsystems

---

### Task 1E.5: Configuration System Unification
**Effort:** 2-3 days
**Dependencies:** 1E.1
**Blocks:** Configuration-driven startup

Create unified configuration that can enable/disable V2/V4.

**Deliverables:**
- `src/config/unified.rs` (or update existing)
- Configuration schema with v2/v4 sections
- Validation logic
- Configuration file examples

**Key Implementation Details:**
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct DualConfig {
    pub application: ApplicationConfig,

    #[serde(default)]
    pub v2: V2SubsystemConfig,

    #[serde(default)]
    pub v4: V4SubsystemConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct V2SubsystemConfig {
    pub enabled: bool,
    pub instruments: Vec<V2InstrumentDef>,
    pub storage: Option<V2StorageConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct V4SubsystemConfig {
    pub enabled: bool,
    pub instruments: Vec<V4InstrumentDef>,
    pub storage: Option<V4StorageConfig>,
}

impl DualConfig {
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if !self.v2.enabled && !self.v4.enabled {
            return Err(anyhow!("At least one subsystem (v2 or v4) must be enabled"));
        }
        // Validate no duplicate instrument IDs across subsystems
        Ok(())
    }
}
```

**Example Configuration:**
```toml
[application]
name = "rust-daq"
log_level = "info"

[v2]
enabled = true

[[v2.instruments]]
id = "elliptec1"
type = "Elliptec"
serial_port = "/dev/ttyUSB0"

[v4]
enabled = true

[[v4.instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
resource = "TCPIP0::192.168.1.100::INSTR"

[[v4.instruments]]
id = "pvcam_camera"
type = "PVCAMInstrument"
camera_name = "pvcam0"
```

**Acceptance Criteria:**
- Unified configuration loads correctly
- Both empty and mixed configurations work
- Validation prevents invalid configs
- Examples provided for all common scenarios

---

### Task 1E.6: Measurement Bridge (Optional)
**Effort:** 2-3 days
**Dependencies:** 1E.1
**Blocks:** Unified GUI display

Create bridge for routing measurements from both subsystems to GUI.

**Deliverables:**
- `src/dual_runtime/measurement_bridge.rs`
- UnifiedMeasurement enum
- Bridge actor/task routing measurements
- Conversion utilities

**Key Implementation Details:**
```rust
pub enum UnifiedMeasurement {
    V2(Arc<Measurement>),
    V4(RecordBatch),
}

pub struct MeasurementBridge {
    v2_rx: mpsc::UnboundedReceiver<Arc<Measurement>>,
    v4_rx: mpsc::UnboundedReceiver<RecordBatch>,
    gui_tx: mpsc::UnboundedSender<UnifiedMeasurement>,
    stats: Arc<Mutex<BridgeStats>>,
}

impl MeasurementBridge {
    pub async fn run(&mut self) {
        loop {
            select! {
                Some(v2_meas) = self.v2_rx.recv() => {
                    let _ = self.gui_tx.send(UnifiedMeasurement::V2(v2_meas));
                    self.stats.lock().unwrap().v2_count += 1;
                }
                Some(v4_batch) = self.v4_rx.recv() => {
                    let _ = self.gui_tx.send(UnifiedMeasurement::V4(v4_batch));
                    self.stats.lock().unwrap().v4_count += 1;
                }
            }
        }
    }
}
```

**Acceptance Criteria:**
- Measurements from both sources flow to GUI
- Timestamp ordering preserved
- No measurement loss
- Bridge doesn't block either subsystem

---

### Task 1E.7: Integration Tests
**Effort:** 2 days
**Dependencies:** All 1E tasks
**Blocks:** Phase 1F

Create comprehensive integration tests for coexistence infrastructure.

**Deliverables:**
- `tests/coexistence_integration_tests.rs`
- Serial port contention tests
- VISA queueing tests
- Dual startup/shutdown tests
- Measurement flow tests

**Tests:**
```rust
#[tokio::test]
async fn test_both_subsystems_start() {
    let mut manager = DualRuntimeManager::new(test_config()).await?;
    manager.start().await?;

    assert!(manager.v2_started());
    assert!(manager.v4_started());

    manager.shutdown().await?;
}

#[tokio::test]
async fn test_serial_exclusive_access() {
    let shared = Arc::new(SharedResources::new());

    // V2 acquires port
    let guard1 = shared.acquire_serial("/dev/ttyUSB0", "v2_actor").await?;

    // V4 cannot acquire
    let result = shared.acquire_serial("/dev/ttyUSB0", "v4_actor").await;
    assert!(result.is_err());

    // After V2 releases
    drop(guard1);

    // V4 can acquire
    let guard2 = shared.acquire_serial("/dev/ttyUSB0", "v4_actor").await?;
    assert!(guard2.owner() == Some("v4_actor"));
}

#[tokio::test]
async fn test_visa_command_serialization() {
    let manager = VisaSessionManager::new()?;

    // Spawn multiple threads sending VISA commands
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let mgr = manager.clone();
            tokio::spawn(async move {
                for j in 0..10 {
                    let resp = mgr.enqueue_command(&format!("CMD_{}{}", i, j),
                        Duration::from_secs(1)).await?;
                    assert!(!resp.is_empty());
                }
                Ok::<_, anyhow::Error>(())
            })
        })
        .collect();

    // All commands execute without error
    for h in handles {
        h.await??;
    }

    // All commands accounted for
    assert_eq!(manager.stats().total_commands, 50);
}
```

**Acceptance Criteria:**
- All tests pass
- No deadlocks detected
- No resource leaks
- Coverage > 85% for coexistence code

---

## Phase 1F: Per-Instrument Migration

### Task 1F.1: Newport1830C V4 Adapter Design Review
**Effort:** 1-2 days
**Dependencies:** Phase 1E complete
**Blocks:** 1F.2

Review existing V4 Newport implementation and plan any needed adjustments.

**Deliverables:**
- Design review document
- List of any needed changes
- Testing strategy document

**Acceptance Criteria:**
- Design review completed and approved
- Clear path forward identified

---

### Task 1F.2: Newport1830C Parallel Operation Testing
**Effort:** 2-3 days
**Dependencies:** 1F.1
**Blocks:** Cutover

Test Newport1830C V2 and V4 running simultaneously.

**Deliverables:**
- Integration test with both running
- Serial port contention testing
- Measurement comparison validation
- Performance baseline

**Acceptance Criteria:**
- Both can initialize and acquire measurements
- No serial port conflicts
- Measurements from both are valid
- Performance acceptable

---

### Task 1F.3: Elliptec V4 Implementation
**Effort:** 4-5 days
**Dependencies:** Phase 1E complete
**Blocks:** Elliptec cutover

Implement Elliptec motion controller as V4 actor.

**Deliverables:**
- `v4-daq/src/actors/elliptec.rs`
- ElliptecActor implementing MotionController trait
- Hardware tests
- Integration with InstrumentManager

**Acceptance Criteria:**
- All motion commands working
- Hardware tests pass
- Measurement format compatible with existing

---

### Task 1F.4: Configuration Migration Tools
**Effort:** 2-3 days
**Dependencies:** 1E.5
**Blocks:** User migration

Create tools to help users migrate configs from V2 to V4.

**Deliverables:**
- `tools/migrate_v2_to_v4.py` or Rust tool
- Configuration conversion logic
- Validation of converted configs
- User documentation

**Example:**
```bash
$ cargo run --bin migrate-config -- config.v2.toml > config.v4.toml
Converting V2 configuration...
  Found 3 instruments
  - Elliptec1: V2 -> V4 (serial adapter, fully compatible)
  - Newport1: V2 -> V4 (power meter, fully compatible)
  - MockPower: V2-only (skipped, create manually)
Conversion complete: config.v4.toml
```

**Acceptance Criteria:**
- Automatic conversion handles common cases
- Manual intervention guided for V2-only instruments
- Converted configs validated
- User guide provided

---

### Task 1F.5: Shadow Mode Testing Framework
**Effort:** 2-3 days
**Dependencies:** 1E.6
**Blocks:** Optional - improves confidence

Implement shadow mode where V4 runs in parallel but results not used, enabling validation.

**Deliverables:**
- ShadowMode configuration option
- Measurement comparison logic
- Difference reporting and statistics
- Dashboard for shadow monitoring

**Acceptance Criteria:**
- Shadow mode can be enabled for any instrument
- Measurements compared automatically
- Differences logged and reported
- Performance impact < 5%

---

## Phase 2: Production Stability

### Task 2.1: Stress Testing Suite
**Effort:** 3-4 days
**Dependencies:** Phase 1F complete
**Blocks:** Phase 3

Create comprehensive stress tests for production readiness.

**Deliverables:**
- Long-running tests (4-8 hours)
- Contention stress tests
- Memory leak detection
- Performance regression detection

---

### Task 2.2: Documentation & Migration Guide
**Effort:** 2-3 days
**Dependencies:** Phase 1F complete
**Blocks:** User rollout

Create comprehensive user documentation.

**Deliverables:**
- Migration guide for each instrument
- Configuration examples
- Troubleshooting guide
- FAQ for V2->V4 questions

---

### Task 2.3: User Communication & Training
**Effort:** 1-2 days
**Dependencies:** 2.2
**Blocks:** Phase 3

Communicate migration strategy to users.

**Deliverables:**
- Migration announcement
- Timeline and phases
- Support contact information
- Training materials

---

## Phase 3: V2 Deprecation & Cleanup

### Task 3.1: Gradual V2 Code Removal
**Effort:** 3-4 days per batch
**Dependencies:** Instruments fully migrated
**Blocks:** None - cleanup

Remove V2 code for completed instruments in batches.

**Schedule:**
- Batch 1: Newport1830C, Elliptec (after 1 month)
- Batch 2: Other common instruments (after 3 months)
- Batch 3: Legacy/custom instruments (after 6 months)

---

### Task 3.2: V2 Feature Flag Removal
**Effort:** 1-2 days
**Dependencies:** All V2 instruments removed
**Blocks:** Final cleanup

Remove `feature = "v2"` completely.

---

## Dependency Graph

```
1E.1 (DualRuntimeManager)
├── 1E.2 (SharedSerialPort)
├── 1E.3 (VisaSessionManager)
├── 1E.4 (SharedResources)
├── 1E.5 (Configuration)
└── 1E.6 (MeasurementBridge - optional)

1E.1-6 → 1E.7 (Integration Tests)

1E.7 → 1F.1 (Newport Review)
1E.7 → 1F.2 (Newport Testing)
1E.7 → 1F.3 (Elliptec V4)
1E.5 → 1F.4 (Config Tools)
1E.6 → 1F.5 (Shadow Mode)

1F.1-5 → 2.1 (Stress Tests)
1F.1-5 → 2.2 (Documentation)
2.2 → 2.3 (User Communication)

(All instruments migrated) → 3.1 (V2 Code Removal)
3.1 → 3.2 (Feature Flag Removal)
```

---

## Effort Summary

| Phase | Duration | Tasks | Status |
|-------|----------|-------|--------|
| 1E | 2-3 weeks | 7 core tasks | Ready to start |
| 1F | 3-4 weeks | 5 migration tasks | Depends on 1E |
| 2 | 2-3 weeks | 3 stability tasks | Depends on 1F |
| 3 | 1-2 weeks (spread over months) | 2 cleanup tasks | Depends on user migration |

**Total: 8-12 weeks to Phase 2 readiness, 4-6 months to complete V2 removal**

---

## Success Criteria

By end of Phase 1F:
- [x] V2 and V4 run simultaneously without conflicts
- [x] All Phase 1D instruments (SCPI, ESP300, PVCAM) proven in V4
- [x] Additional instruments (Newport, Elliptec, others) migrated to V4
- [x] Configuration tools enable smooth user migration
- [x] Zero breaking changes to V2 API during migration

By end of Phase 2:
- [x] Production stability confirmed through stress testing
- [x] Users educated on migration path
- [x] Migration tools fully documented

By end of Phase 3:
- [x] V2 code completely removed
- [x] Single clean codebase
- [x] Feature flag cleanup complete
