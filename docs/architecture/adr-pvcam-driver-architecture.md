# ADR: PVCAM Driver Architecture

**Status:** Accepted
**Date:** 2025-01-09
**Author:** Architecture Review
**Related Issues:** bd-ek9n, bd-9ou9, bd-g9po, bd-nq82, bd-lwg7, bd-3gnv

---

## Context

The PVCAM driver (`crates/daq-driver-pvcam/`) provides Rust integration with Teledyne Photometrics cameras via the PVCAM SDK. During architectural review, the question arose whether the driver's ~9K lines of code represents over-engineering, or whether a simpler FFI binding approach would suffice.

This document records the analysis findings and justifies the architectural decisions.

---

## Decision

**The current multi-layer architecture is retained.** The complexity is justified by rust-daq's requirements for production scientific instrumentation.

---

## Architecture Overview

### Component Structure

```
┌──────────────────────────────────────────────────────────────────────┐
│                      PUBLIC API (lib.rs)                             │
│                      PvcamDriver - 1,237 LOC                         │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │ Capabilities: ExposureControl, Triggerable, FrameProducer,      │ │
│  │               Parameterized, MeasurementSource, Commandable     │ │
│  ├─────────────────────────────────────────────────────────────────┤ │
│  │ 48 Parameter<T> Fields: exposure, ROI, thermal, readout,        │ │
│  │                         shutter, streaming, processing, info    │ │
│  ├─────────────────────────────────────────────────────────────────┤ │
│  │ Error Recovery: has_error(), reinitialize(), reset_error_state()│ │
│  └─────────────────────────────────────────────────────────────────┘ │
└────────────────────────┬─────────────────────────────────────────────┘
                         │
         ┌───────────────┼───────────────┐
         │               │               │
    ┌────▼────────┐  ┌──▼─────────┐  ┌──▼─────────────┐
    │ Connection  │  │ Acquisition │  │    Features    │
    │  278 LOC    │  │  2,409 LOC  │  │   2,844 LOC    │
    │             │  │             │  │                │
    │ • SDK Init  │  │ • EOF CBs   │  │ • 50+ Params   │
    │ • Ref Count │  │ • Buffers   │  │ • Enums        │
    │ • Mock Mode │  │ • Loss Det  │  │ • Type Conv    │
    └─────────────┘  └─────────────┘  └────────────────┘
         │               │                   │
         └───────────────┼───────────────────┘
                         │
          ┌──────────────▼──────────────┐
          │   pvcam-sys FFI - 113 LOC   │
          │   Raw C SDK Bindings        │
          └─────────────────────────────┘
```

### Lines of Code Breakdown

| Component | LOC | Percentage | Primary Responsibility |
|-----------|-----|------------|------------------------|
| `pvcam-sys/src/lib.rs` | 113 | 1.6% | Bindgen-generated FFI |
| `components/connection.rs` | 278 | 4.0% | SDK lifecycle management |
| `components/acquisition.rs` | 2,409 | 35.0% | Streaming & frame handling |
| `components/features.rs` | 2,844 | 41.4% | Parameter access API |
| `lib.rs` | 1,237 | 18.0% | Public driver interface |
| **Total** | **6,881** | 100% | |

Additional: ~2,300 LOC in tests, examples, and benchmarks.

---

## Architectural Decisions & Justifications

### 1. Multi-Layer Component Architecture

**Decision:** Separate the driver into Connection, Acquisition, Features, and Driver layers.

**Justification:**
- **Single Responsibility:** Each component has a clear, testable purpose
- **Dependency Isolation:** Components depend only on layers below them
- **Mock Mode Support:** Each layer can be mocked independently for testing
- **Maintenance:** Changes to one layer don't cascade to others

**Alternative Considered:** Flat structure with all code in one file.
**Why Rejected:** Would create a 6K+ LOC monolith, difficult to test and maintain.

---

### 2. Reference-Counted SDK Lifecycle (bd-9ou9)

**Decision:** Use atomic reference counting for PVCAM SDK initialization.

**Location:** `components/connection.rs:15-45`

```rust
static SDK_REF_COUNT: AtomicU32 = AtomicU32::new(0);
static SDK_INIT_MUTEX: Mutex<()> = Mutex::new(());
```

**Justification:**
- PVCAM SDK is global state - only one `pl_pvcam_init()` / `pl_pvcam_uninit()` pair allowed per process
- Multiple `PvcamDriver` instances may be created (e.g., multi-camera setups)
- First driver initializes SDK; last driver uninitializes
- Atomic counter ensures thread-safe tracking

**Alternative Considered:** Single global driver instance.
**Why Rejected:** Prevents multi-camera scenarios; violates Rust ownership principles.

---

### 3. EOF Callback Architecture (bd-ek9n.2)

**Decision:** Use PVCAM EOF callbacks with condvar signaling instead of polling.

**Location:** `components/acquisition.rs:93-200`

```rust
pub struct CallbackContext {
    pub pending_frames: AtomicU32,      // Counter, not bool
    pub condvar: Condvar,               // Efficient waiting
    pub mutex: Mutex<bool>,             // Condvar pairing
    pub shutdown: AtomicBool,           // Graceful exit
    pub latest_frame_nr: AtomicI32,     // Frame tracking
}
```

**Justification:**
- **CPU Efficiency:** Callbacks use <1% CPU vs 15-30% for polling with sleep
- **Latency:** Microsecond-precision notification vs millisecond polling intervals
- **PVCAM Best Practice:** Official SDK documentation recommends callback-based acquisition
- **Event Coalescence:** `AtomicU32` counter prevents lost events when multiple callbacks fire during processing

**Alternative Considered:** Polling with `pl_exp_check_cont_status()` in a loop.
**Why Rejected:** Wastes CPU cycles; adds latency; doesn't scale to high frame rates.

---

### 4. Frame Loss Detection (bd-ek9n.3)

**Decision:** Track hardware frame numbers to detect dropped frames.

**Location:** `components/acquisition.rs:1899-1946`

**Justification:**
- **Data Integrity:** Silent frame drops corrupt scientific datasets
- **Hardware Source:** `FRAME_INFO.FrameNr` is generated by camera FPGA, not software
- **Metrics Exposure:** `lost_frames` and `discontinuity_events` counters enable monitoring
- **Detection Method:** Compare current frame number to previous; gaps indicate loss

**Metrics Provided:**
```rust
pub fn frame_loss_stats(&self) -> (u64, u64) {
    (
        self.lost_frames.load(Ordering::Relaxed),
        self.discontinuity_events.load(Ordering::Relaxed),
    )
}
```

**Alternative Considered:** Trust the SDK to deliver all frames.
**Why Rejected:** USB packet loss is real; undetected drops are worse than detected ones.

---

### 5. Dynamic Buffer Sizing (bd-ek9n.4)

**Decision:** Query `PARAM_FRAME_BUFFER_SIZE` to determine buffer allocation.

**Location:** `components/acquisition.rs:850-920`

**Justification:**
- **Memory Efficiency:** Fixed frame counts waste memory for small ROIs, overflow for large ones
- **SDK Guidance:** PVCAM provides min/max buffer size recommendations
- **ROI Awareness:** Buffer size scales with actual frame dimensions

**Alternative Considered:** Fixed 10-frame circular buffer.
**Why Rejected:** Doesn't adapt to ROI changes; may overflow or waste memory.

---

### 6. Parameter<T> Reactive System

**Decision:** Wrap all camera settings in rust-daq's `Parameter<T>` type with hardware callbacks.

**Location:** `lib.rs:221-453` (48 parameter definitions)

**Justification:**
- **gRPC Remote API:** Parameters are observable by remote clients
- **Validation Before Write:** `connect_to_hardware_write()` validates before touching hardware
- **State Consistency:** Prevents "split brain" where client thinks value is X but hardware is Y
- **Change Notification:** Subscribers receive updates automatically

**Parameter Groups:**
| Group | Count | Examples |
|-------|-------|----------|
| Acquisition | 8 | exposure_ms, trigger_mode, roi, binning |
| Thermal | 3 | temperature, temperature_setpoint, fan_speed |
| Readout | 9 | readout_port, speed_mode, gain_mode, adc_offset |
| Timing | 5 | readout_time_us, clearing_time_us, frame_time_us |
| Shutter | 4 | shutter_mode, shutter_status, open/close_delay |
| Streaming | 3 | smart_stream_enabled, smart_stream_mode, metadata |
| Processing | 4 | host_rotate, host_flip, summing_enabled/count |
| Info | 4 | serial_number, firmware_version, model_name, bit_depth |

**Alternative Considered:** Direct SDK calls without parameter abstraction.
**Why Rejected:** No gRPC observability; no validation; no change notification.

---

### 7. Error Recovery System (bd-g9po)

**Decision:** Implement explicit error detection and recovery methods.

**Location:** `lib.rs:964-1073`

**API:**
```rust
pub fn has_error(&self) -> bool
pub fn last_error(&self) -> Option<AcquisitionError>
pub async fn reset_error_state(&self) -> Result<()>
pub async fn reinitialize(&self) -> Result<()>
```

**Error Types:**
```rust
pub enum AcquisitionError {
    ReadoutFailed,      // USB disconnect, hardware error
    StatusCheckFailed,  // SDK internal error
    Timeout,            // No frames for extended period
}
```

**Justification:**
- **Production Reliability:** USB disconnects happen; applications need recovery paths
- **Graceful Degradation:** Transient errors can be cleared; persistent errors require reinit
- **Observability:** Applications can detect and report error conditions

**Alternative Considered:** Let errors propagate and require application restart.
**Why Rejected:** Unacceptable for long-running scientific acquisitions.

---

### 8. Safe Drop Order (bd-nq82, bd-lwg7)

**Decision:** Field declaration order ensures correct cleanup sequence.

**Location:** `lib.rs:46-54`

```rust
pub struct PvcamDriver {
    camera_name: String,

    // ORDER MATTERS: acquisition must drop BEFORE connection
    acquisition: Arc<PvcamAcquisition>,  // Drops first
    connection: Arc<Mutex<PvcamConnection>>,  // Drops second

    // ... parameters ...
}
```

**Justification:**
- **Thread Safety:** Poll thread must stop before SDK uninitialization
- **Rust Drop Order:** Fields drop in declaration order
- **Async Safety:** Cannot use `block_on()` in Drop from async context

**Best Practice:** Call `driver.shutdown().await` before dropping.

---

### 9. Async Integration via spawn_blocking

**Decision:** Wrap synchronous PVCAM calls in `tokio::task::spawn_blocking`.

**Location:** `lib.rs:126-150`

```rust
let connection = tokio::task::spawn_blocking({
    let name = camera_name.clone();
    move || -> Result<Arc<Mutex<PvcamConnection>>> {
        let mut conn = PvcamConnection::new();
        conn.initialize()?;
        conn.open(&name)?;
        Ok(Arc::new(Mutex::new(conn)))
    }
}).await??;
```

**Justification:**
- **PVCAM is Synchronous:** C SDK calls block the calling thread
- **Tokio Compatibility:** Blocking calls on async runtime starve other tasks
- **Solution:** `spawn_blocking` moves work to dedicated blocking thread pool

**Alternative Considered:** Use PVCAM calls directly in async functions.
**Why Rejected:** Would block tokio runtime; causes latency spikes and deadlocks.

---

### 10. Mock Mode for Testing

**Decision:** Feature-gate hardware calls; provide mock implementations.

**Location:** Throughout, controlled by `#[cfg(feature = "pvcam_hardware")]`

**Example:**
```rust
pub fn get_temperature(_conn: &PvcamConnection) -> Result<f64> {
    #[cfg(feature = "pvcam_hardware")]
    {
        // Real SDK call
    }
    #[cfg(not(feature = "pvcam_hardware"))]
    {
        Ok(_conn.mock_state.lock().unwrap().temperature_c)
    }
}
```

**Justification:**
- **CI/CD Testing:** Tests run without PVCAM hardware or SDK
- **Development:** Developers can work without camera connected
- **Isolation:** Tests verify logic without SDK bugs interfering

---

## Comparison: Minimal Binding vs Current Architecture

### Minimal PVCAM Binding (~2K LOC)

Would provide:
- Init/open/close wrappers
- Basic `start_stream()` / `stop_stream()`
- Single-frame acquisition
- Basic frame retrieval

Would lack:
- ❌ Parameter<T> integration → No gRPC remote control
- ❌ Feature detection → Hard-coded camera assumptions
- ❌ Frame loss detection → Silent data corruption
- ❌ Error recovery → Manual restart on failure
- ❌ Async safety → Blocks tokio runtime
- ❌ Multi-instance support → Single camera only

### Current Architecture (~9K LOC)

Provides all of the above, plus:
- ✅ 48 observable, validatable parameters
- ✅ Automatic hardware synchronization
- ✅ Frame loss metrics
- ✅ Error detection and recovery
- ✅ Async-safe operations
- ✅ Multi-camera support
- ✅ Mock mode for testing

---

## Potential Simplifications

If code reduction is desired, these optional features could be feature-gated:

| Feature | LOC | Impact |
|---------|-----|--------|
| Host frame processing | ~600 | Removes rotation/flip/summing |
| Smart streaming mode | ~200 | Removes hardware-timed sequences |
| Centroids detection | ~100 | Removes on-chip centroid feature |
| 85-frame auto-restart | ~100 | Removes Prime BSI workaround |

**Total potential reduction:** ~1,000 LOC (11% of driver)

---

## Consequences

### Positive
- Production-grade reliability for scientific instrumentation
- Full integration with rust-daq's gRPC remote API
- Testable without hardware via mock mode
- Handles real-world failure modes (USB disconnect, frame drops)
- Follows PVCAM SDK best practices

### Negative
- Higher initial complexity than minimal binding
- More code to maintain
- Longer compile times with full feature set

### Neutral
- Learning curve for new contributors
- Requires understanding of async Rust patterns

---

## References

- [PVCAM SDK Reference](../reference/PVCAM_SDK_REFERENCE.md)
- [PVCAM Setup Guide](../troubleshooting/PVCAM_SETUP.md)
- PVCAM SDK Official Documentation: https://docs.teledynevisionsolutions.com/pvcam-sdk/

---

## Revision History

| Date | Author | Description |
|------|--------|-------------|
| 2025-01-09 | Architecture Review | Initial analysis and documentation |
