# V2/V4 Coexistence: Risks & Blockers

## Document Purpose

This document identifies concrete risks, potential blockers, and mitigation strategies for the V2/V4 coexistence architecture. Each risk is categorized by severity and phase, with actionable mitigation plans.

---

## Critical Risks (Severity: CRITICAL)

### CR-1: Hardware Resource Deadlock

**Description:**
V2 and V4 acquire shared hardware resources (serial ports, VISA sessions) in different orders, causing deadlock.

**Scenario:**
- V2 acquires serial port A, then requests VISA session
- V4 acquires VISA session, then requests serial port A
- Both wait indefinitely

**Probability:** MEDIUM
**Impact:** CRITICAL (application hang, requires restart)
**Affected Phase:** 1E (during SharedSerialPort/VisaSessionManager testing)

**Mitigation Strategy:**
1. **Ordering Protocol**: Define strict resource acquisition order globally
   - Always acquire serial ports before VISA sessions
   - Document in SharedResources::acquire() calls
   - Add compile-time assertions where possible

2. **Timeout Protection**: All acquisitions must have timeout
   ```rust
   async fn acquire_with_timeout<T>(
       &self,
       duration: Duration,
       acquire_fn: impl Future<Output = Result<T>>,
   ) -> Result<T> {
       tokio::time::timeout(duration, acquire_fn)
           .await
           .map_err(|_| anyhow!("Resource acquisition timeout - possible deadlock"))?
   }
   ```

3. **Detection & Recovery**:
   - Timeout triggers emergency cleanup
   - Deadlock detector logs all held resources
   - Automatic resource release on timeout

4. **Testing**:
   - Chaos tests that randomize acquisition order
   - Extended stress tests checking for hangs
   - Deadlock-detection tools (loom, miri for safe code)

**Verification Checklist:**
- [ ] Acquisition ordering documented
- [ ] All acquisitions have timeouts (max 2s)
- [ ] Unit tests with randomized orderings pass
- [ ] 8-hour stress test shows no hangs
- [ ] Deadlock detector proves no cycles possible

---

### CR-2: VISA Single-Session Limitation

**Description:**
VISA SDK fundamentally allows only one active session at a time. If both V2 and V4 try to use VISA instruments in parallel, one will fail.

**Technical Detail:**
VISA-RS (Rust bindings) wraps NI-VISA C library which has global session state. Multiple simultaneous open/read/write calls will fail.

**Probability:** HIGH (inherent to VISA)
**Impact:** CRITICAL (cannot use multiple VISA instruments simultaneously)
**Affected Phase:** 1E (VisaSessionManager design), 1F (when adding VISA instruments)

**Mitigation Strategy:**

1. **VisaSessionManager Architecture**:
   - Single global VISA session (per process)
   - Command queueing with response routing
   - Serializes all VISA operations
   - Guarantees single operation in flight

2. **Implementation Safeguards**:
   ```rust
   pub struct VisaSessionManager {
       // Only one real session can exist
       session: Arc<Mutex<Option<Session>>>,

       // Command queue ensures FIFO ordering
       queue: Arc<Mutex<VecDeque<PendingCommand>>>,

       // Worker task processes one command at a time
       worker_task: JoinHandle<()>,
   }

   // Enforce single instance at type level
   pub static VISA_MANAGER: OnceLock<Arc<VisaSessionManager>> = OnceLock::new();
   ```

3. **Per-Instrument VISA Fallback**:
   - For multiple VISA instruments, must share a single session
   - Session setup overhead amortized across many measurements
   - Cache instrument identity to avoid repeated *IDN? queries

4. **User Guidance**:
   - Documentation clearly states "only one VISA instrument at a time"
   - Configuration validation prevents registering multiple VISA instruments
   - Error messages suggest alternatives (use serial protocol if available)

5. **Testing**:
   - Unit tests verify command ordering
   - Load tests with 10+ commands from both V2/V4
   - Verify response routing correctness

**Workaround for Multiple VISA Instruments:**
If users need multiple VISA instruments, they must choose one of:
1. Use serial protocol instead of VISA when available
2. Use different test/measurement sets (not simultaneous)
3. Run separate applications for different instruments
4. Upgrade to separate VISA-capable instruments on different computers

**Verification Checklist:**
- [ ] VisaSessionManager ensures single session
- [ ] Command queue tested under load (100+ commands/sec)
- [ ] Response routing verified correct
- [ ] Configuration validation prevents invalid setups
- [ ] Documentation updated with VISA limitations
- [ ] User migration guide mentions limitation

---

### CR-3: Data Corruption from Concurrent Storage

**Description:**
Both V2 and V4 storage writers write to same HDF5 file or same directory, causing data corruption.

**Probability:** MEDIUM (depends on user configuration)
**Impact:** CRITICAL (data loss, corrupted recordings)
**Affected Phase:** 1E (initial design), 1F (when both storing data)

**Mitigation Strategy:**

1. **Separate Output Directories**:
   - V2 writes to `$OUTPUT_DIR/v2/$SESSION_ID/`
   - V4 writes to `$OUTPUT_DIR/v4/$SESSION_ID/`
   - No overlap possible at filesystem level

2. **Configuration Validation**:
   ```rust
   fn validate_storage_paths(&self) -> Result<()> {
       if self.v2.enabled && self.v4.enabled {
           let v2_path = Path::new(&self.v2.storage.output_dir);
           let v4_path = Path::new(&self.v4.storage.output_dir);

           if v2_path.canonicalize()? == v4_path.canonicalize()? {
               return Err(anyhow!(
                   "V2 and V4 must use different storage directories"
               ));
           }
       }
       Ok(())
   }
   ```

3. **File Locking**:
   - Even with separate directories, add file-level locks
   - HDF5 supports concurrent readers, but not concurrent writers
   - Use parking_lot::RwLock on file paths

4. **Session Isolation**:
   - Each run has unique session ID (timestamp-based)
   - Prevents writes to same file from different runs

5. **Testing**:
   - Create scenarios where both V2 and V4 record
   - Verify files don't corrupt
   - Compare recorded data integrity

**Verification Checklist:**
- [ ] Default configs use separate output directories
- [ ] Configuration validation catches conflicts
- [ ] Tests verify no data corruption with dual recording
- [ ] Documentation recommends directory separation
- [ ] Migration guide includes storage directory planning

---

## High-Risk Issues (Severity: HIGH)

### HR-1: Complex Shutdown Sequence

**Description:**
Graceful shutdown becomes complex when coordinating V2 (manual) and V4 (supervised) subsystems.

**Failure Modes:**
- V4 shuts down before V2, releasing shared resources
- V2 continues using freed serial port, causing panic
- Timeout on one subsystem leaves other partially shutdown

**Probability:** MEDIUM
**Impact:** HIGH (graceful shutdown fails, forced termination)
**Affected Phase:** 1E (shutdown design), all phases

**Mitigation Strategy:**

1. **Explicit Shutdown Order**:
   ```rust
   impl DualRuntimeManager {
       pub async fn shutdown(&mut self, timeout: Duration) -> Result<()> {
           // Phase 1: Notify both subsystems (non-blocking)
           self.signal_v4_shutdown();
           self.signal_v2_shutdown();

           // Phase 2: Wait for V4 with timeout (Kameo handles supervised shutdown)
           match tokio::time::timeout(timeout, self.wait_v4_stopped()) {
               Ok(Ok(())) => info!("V4 shutdown complete"),
               Ok(Err(e)) => warn!("V4 shutdown error: {}", e),
               Err(_) => {
                   warn!("V4 shutdown timeout, forcing");
                   self.force_v4_shutdown();
               }
           }

           // Phase 3: Wait for V2 with timeout
           match tokio::time::timeout(timeout, self.wait_v2_stopped()) {
               Ok(Ok(())) => info!("V2 shutdown complete"),
               Ok(Err(e)) => warn!("V2 shutdown error: {}", e),
               Err(_) => {
                   warn!("V2 shutdown timeout, forcing");
                   self.force_v2_shutdown();
               }
           }

           // Phase 4: Release shared resources
           self.cleanup_shared_resources().await?;

           Ok(())
       }
   }
   ```

2. **Timeout Configuration**:
   - Per-subsystem timeouts: 5s each
   - Total shutdown timeout: 15s max
   - Fallback to forced shutdown if exceeded

3. **Resource Release Ordering**:
   - V4 actors stop acquiring new resources
   - V2 actors stop acquiring new resources
   - All shared resources released atomically

4. **Verification Mechanisms**:
   - Shutdown monitor checks all resources released
   - Debug logs record shutdown sequence for forensics
   - Test framework validates shutdown completeness

5. **Testing**:
   - Shutdown at each phase (startup, mid-operation, under load)
   - Verify no resources leaked
   - Verify no panics during shutdown
   - Test timeout scenarios

**Verification Checklist:**
- [ ] Shutdown sequence documented clearly
- [ ] Unit tests verify ordering
- [ ] Tests confirm timeouts work correctly
- [ ] Resource leak detection passes
- [ ] No panics under any shutdown scenario
- [ ] Stress tests include shutdown cycles

---

### HR-2: Serial Port Driver Instability

**Description:**
Serial port drivers (especially USB serial adapters) can disconnect unexpectedly. V2 and V4 may not handle this consistently.

**Probability:** MEDIUM (hardware-dependent)
**Impact:** HIGH (measurement loss, potential data corruption)
**Affected Phase:** 1F (when Newport, Elliptec active)

**Mitigation Strategy:**

1. **Shared Port State Tracking**:
   ```rust
   pub struct SerialPortEntry {
       port: Arc<Mutex<Box<dyn SerialPort>>>,
       connected: Arc<AtomicBool>,
       last_error: Arc<Mutex<Option<String>>>,
   }

   impl SerialPortEntry {
       pub fn on_error(&self, error: String) {
           self.connected.store(false, Ordering::Relaxed);
           *self.last_error.lock().unwrap() = Some(error);
       }
   }
   ```

2. **Detection Strategy**:
   - V2 and V4 detect errors independently
   - Both report to central error handler
   - Automatic port reopen attempted
   - User notified if recovery fails

3. **Recovery Procedure**:
   - Close port (V2 and V4 both release locks)
   - Wait 500ms for device driver
   - Reopen port with fresh configuration
   - Resume from checkpoint if applicable

4. **Testing**:
   - Mock driver that simulates disconnect
   - Verify both subsystems detect failure
   - Verify recovery procedure works
   - Measure data loss on disconnect/recovery

**Verification Checklist:**
- [ ] Disconnect detection working in both V2/V4
- [ ] Recovery procedure tested
- [ ] No data corruption on recover
- [ ] User is notified of disconnects
- [ ] Documentation covers disconnect scenarios

---

### HR-3: Memory/CPU Contention Under Load

**Description:**
Running two full actor systems (tokio tasks + Kameo actors) may cause resource contention and performance degradation.

**Probability:** MEDIUM (depends on measurement rate)
**Impact:** HIGH (slow response, missed measurements)
**Affected Phase:** 2 (stress testing)

**Mitigation Strategy:**

1. **Baseline Profiling**:
   - Measure single-system performance (V2-only, V4-only)
   - Measure dual-system performance (same workload)
   - Track CPU, memory, latency, throughput

2. **Resource Limits**:
   ```rust
   pub struct ResourceBudget {
       v2_cpu_percent: f32,  // e.g., 30%
       v4_cpu_percent: f32,  // e.g., 30%
       shared_cpu_percent: f32, // e.g., 40%

       v2_memory_mb: usize,  // e.g., 256MB
       v4_memory_mb: usize,  // e.g., 256MB
   }
   ```

3. **Tokio Runtime Tuning**:
   - Separate thread pools for V2 and V4
   - Control worker thread count
   - Pin high-priority tasks to specific cores

4. **Kameo Actor Tuning**:
   - Mailbox size limits
   - Processing batch sizes
   - Priority levels for messages

5. **Monitoring**:
   - Real-time CPU/memory monitoring
   - Alert if usage exceeds limits
   - Log performance metrics periodically

6. **Testing**:
   - Stress test with maximum measurement rate
   - Monitor system resources throughout
   - Verify no timeout missed deadlines
   - Compare before/after performance

**Verification Checklist:**
- [ ] Single-system baseline established
- [ ] Dual-system adds <20% overhead
- [ ] No measurement loss under normal load
- [ ] CPU stays below 80% under stress
- [ ] Memory stable over 8-hour runs
- [ ] Documentation includes performance specs

---

## Medium-Risk Issues (Severity: MEDIUM)

### MR-1: Configuration Complexity & User Error

**Description:**
Unified configuration with V2 and V4 options is complex, leading to user mistakes (wrong paths, conflicting IDs, etc.).

**Probability:** HIGH (expected in user adoption)
**Impact:** MEDIUM (user frustration, support burden)
**Affected Phase:** 1F (configuration tools), 2+ (user adoption)

**Mitigation Strategy:**

1. **Configuration Validation**:
   - Check for duplicate instrument IDs
   - Verify all serial ports exist
   - Validate VISA resource strings
   - Ensure directories writable

2. **Clear Error Messages**:
   ```
   Error: Duplicate instrument ID 'scpi_meter' in both v2 and v4
   Suggestion: Rename to 'scpi_meter_v4' in v4 section
   See: config.toml:23
   ```

3. **Configuration Generator**:
   - Interactive tool that builds configs
   - Guides through each instrument setup
   - Validates as user enters data
   - Produces working config

4. **Migration Tool**:
   - Automatic conversion from V2 → V4 config
   - Flags any incompatibilities
   - Produces validated output

5. **Documentation**:
   - Clear examples for all common scenarios
   - Troubleshooting guide
   - FAQ for common mistakes

**Verification Checklist:**
- [ ] Validation catches all common mistakes
- [ ] Error messages are helpful
- [ ] Configuration generator tested
- [ ] Migration tool handles 95% of cases
- [ ] Documentation covers all scenarios

---

### MR-2: Measurement Format Incompatibility

**Description:**
V2 uses Arc<Measurement> with variants (Scalar, Spectrum, Image). V4 uses Arrow RecordBatch. GUI confusion if formats don't match.

**Probability:** MEDIUM
**Impact:** MEDIUM (incorrect visualization, incorrect analysis)
**Affected Phase:** 1E (bridge design), 1F (first dual operation)

**Mitigation Strategy:**

1. **Unified Measurement Format**:
   - Define intermediate format that both produce
   - V2→Unified conversion in adapter
   - V4→Unified conversion in DataPublisher
   - GUI consumes only unified format

2. **Schema Definition**:
   ```rust
   pub enum UnifiedDataVariant {
       Scalar(f64),
       Array(Vec<f64>), // spectrum
       Image(Vec<Vec<f64>>),
   }

   pub struct UnifiedMeasurement {
       instrument_id: String,
       timestamp: SystemTime,
       data: UnifiedDataVariant,
       source: MeasurementSource, // V2 or V4
   }
   ```

3. **Testing**:
   - Generate identical measurements from V2 and V4
   - Verify they convert to identical unified format
   - GUI display tests with both sources

4. **Documentation**:
   - Document unified format
   - Document conversion rules
   - Provide examples

**Verification Checklist:**
- [ ] Unified format defined
- [ ] Conversions tested end-to-end
- [ ] GUI correctly handles both sources
- [ ] Documentation complete

---

### MR-3: Version Compatibility Issues

**Description:**
V2 and V4 may use incompatible versions of dependencies (tokio, serde, arrow, etc.), causing version conflicts.

**Probability:** MEDIUM
**Impact:** MEDIUM (build failures, runtime incompatibilities)
**Affected Phase:** 1E (dependency resolution)

**Mitigation Strategy:**

1. **Dependency Analysis**:
   - Audit all V2 and V4 dependencies
   - Identify overlaps
   - Check compatibility (semver)

2. **Version Alignment**:
   - Use workspace dependencies where possible
   - Pin major versions consistently
   - Allow minor/patch version flexibility

3. **Workspace Configuration**:
   ```toml
   [workspace]
   members = ["v2-core", "v4-daq"]

   [workspace.dependencies]
   tokio = { version = "1.35", features = ["full"] }
   serde = { version = "1.0" }
   arrow = { version = "57" }
   ```

4. **CI Testing**:
   - Build with all feature combinations
   - Test with MSRV (Minimum Supported Rust Version)
   - Dependency audit in CI

**Verification Checklist:**
- [ ] All shared dependencies compatible
- [ ] Workspace dependencies used consistently
- [ ] CI tests all feature combinations
- [ ] No dependency conflicts reported

---

## Low-Risk Issues (Severity: LOW)

### LR-1: Documentation Maintenance

**Description:**
Dual architecture requires more documentation to avoid user confusion.

**Mitigation:** Create comprehensive documentation as part of Phase 1F and 2.

---

### LR-2: Testing Framework Complexity

**Description:**
More test scenarios needed to cover V2/V4 interactions.

**Mitigation:** Invest in test infrastructure early (Phase 1E.7).

---

## Blockers (Issues That Prevent Progress)

### BLOCKER-1: VISA Integration Unresolved

**Status:** RESOLVED (designed with VisaSessionManager)
**Blocking:** SCPI and other VISA instruments in V4
**Dependency:** VisaSessionManager implementation (Task 1E.3)

---

### BLOCKER-2: Shared Serial Port Access Not Verified

**Status:** REQUIRES TESTING
**Blocking:** Serial instruments in dual operation
**Dependency:** SharedSerialPort integration tests (Task 1E.7)

**Action Items:**
- [ ] Build test harness with two actors on same serial port
- [ ] Verify exclusive access enforced
- [ ] Verify no panics or deadlocks
- [ ] Measure latency impact

---

### BLOCKER-3: V4 Kameo Actor Lifecycle Integration

**Status:** RESEARCH NEEDED
**Blocking:** Phase 1E (DualRuntimeManager)
**Question:** How to cleanly integrate Kameo actor supervision with tokio-based V2 subsystem?

**Required Research:**
- Kameo's ActorRef and WeakActorRef semantics
- Shutdown coordination with external systems
- Error propagation from actor panics
- Message delivery guarantees

**Ownership:** Assigned to architecture review

---

## Contingency Plans

### If VisaSessionManager Fails
**Alternative:** Each VISA instrument runs in separate process, communicates via network
**Cost:** Higher latency, more complex deployment
**Timeline:** +2 weeks for implementation

### If Serial Port Contention Unresolvable
**Alternative:** Instruments forced to run on separate serial ports
**Cost:** Limits hardware configurations
**Timeline:** +1 week for configuration validation

### If Memory/CPU Contention Too High
**Alternative:** Run V2 and V4 in separate processes, communicate via network
**Cost:** Higher latency, network overhead, more complex IPC
**Timeline:** +3 weeks for RPC framework

### If Graceful Shutdown Fails
**Alternative:** Timeout-based forced shutdown
**Cost:** Potential resource leaks, data integrity risk
**Timeline:** N/A (fallback only)

---

## Risk Monitoring & Escalation

### Phase 1E Risk Gates
- [ ] No deadlocks in 8-hour runtime test
- [ ] All resource acquisitions have timeouts
- [ ] Shutdown sequence verified complete
- [ ] No measurement data loss detected

### Phase 1F Risk Gates
- [ ] Dual operation stress test passes (24 hours)
- [ ] Configuration validation catches all known errors
- [ ] Measurement format compatibility verified
- [ ] Performance overhead acceptable (<20%)

### Phase 2 Risk Gates
- [ ] 72-hour production stability test passes
- [ ] No data corruption detected
- [ ] User migration tools validated
- [ ] Documentation complete and tested

---

## Escalation Procedure

If any **CRITICAL** risk is triggered:
1. Immediate pause of Phase 1F tasks
2. Focus all resources on mitigation
3. Document root cause and fix
4. Add test case to prevent regression
5. Escalate to architecture review if fundamental design issue

If any **HIGH** risk is triggered:
1. Assess impact and urgency
2. Plan mitigation in next sprint
3. Document workarounds if needed
4. Add test case for coverage

---

## Appendix: Risk Scoring Methodology

**Probability Scale:**
- LOW: <20% chance of occurring
- MEDIUM: 20-50%
- HIGH: 50-80%
- CRITICAL: >80%

**Impact Scale:**
- LOW: User can work around easily
- MEDIUM: Significant disruption but not data-threatening
- HIGH: Major disruption or data-loss potential
- CRITICAL: System failure or permanent data loss

**Overall Risk = Probability × Impact**
