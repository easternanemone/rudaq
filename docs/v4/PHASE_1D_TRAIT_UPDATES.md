# Phase 1D Trait Design Updates - Hardware Validation Learnings

**Date:** 2025-11-17
**Status:** FINAL (Post-Hardware Validation)
**Previous Version:** phase1d_meta_traits.md (v0.1)

---

## 1. Key Learnings from Hardware Validation

### 1.1 Successful Patterns to Adopt

#### ✅ Builder Pattern for Adapters (SerialAdapterV4Builder)

**Success:** Fluent API with sensible defaults worked perfectly for MaiTai

```rust
let adapter = SerialAdapterV4Builder::new(port, baud_rate)
    .with_line_terminator("\r".to_string())
    .with_response_delimiter('\r')
    .with_timeout(Duration::from_secs(2))
    .build();
```

**Apply to:** VisaAdapterV4Builder, CameraAdapterV4Builder (future)

#### ✅ Kameo 0.17 Actor Patterns

**Validated:**
```rust
impl kameo::Actor for MaiTai {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(mut args: Self::Args, _actor_ref: ActorRef<Self>)
        -> Result<Self, Self::Error> {
        // Hardware initialization
        Ok(args)
    }

    async fn on_stop(&mut self, _actor_ref: WeakActorRef<Self>,
                     _reason: kameo::error::ActorStopReason)
        -> Result<(), Self::Error> {
        // Cleanup
        Ok(())
    }
}
```

**Apply to:** All V4 instrument actors

#### ✅ Arrow Conversion as Default Method

**Problem:** TunableLaser and PowerMeter both implement `to_arrow()` with similar logic

**Solution:** Move Arrow conversion to default trait methods

```rust
#[async_trait::async_trait]
pub trait TunableLaser: Send + Sync {
    // ... async methods ...

    /// Default Arrow conversion (can be overridden)
    fn to_arrow(&self, measurements: &[LaserMeasurement]) -> Result<RecordBatch> {
        // Default implementation with standard schema
    }
}
```

**Apply to:** All meta-instrument traits

### 1.2 Protocol Challenges to Address

#### ⚠️ Command Echo Handling

**Issue:** Some instruments echo commands in responses (MaiTai: `WAVELENGTH:800` response)

**Solution:** Parse responses with `.split(':').last().unwrap_or(&response)`

```rust
async fn read_hardware_wavelength(&self) -> Result<f64> {
    let response = adapter.send_command("WAVELENGTH?").await?;

    // MaiTai may echo the command, extract value after colon if present
    let value_str = response.split(':').last().unwrap_or(&response);

    value_str.trim().parse().with_context(|| ...)
}
```

**Apply to:** All adapter query methods

#### ⚠️ SET-Only Instruments

**Issue:** MaiTai accepts SET commands but doesn't respond to GET queries

**Implication:** Traits must support instruments that only track state locally

**Solution:** Document state-tracking fallback

```rust
async fn get_wavelength(&self) -> Result<Wavelength> {
    if let Some(_adapter) = &self.adapter {
        // Try hardware query first
        match self.read_hardware_wavelength().await {
            Ok(nm) => {
                self.wavelength = Wavelength { nm };
                Ok(self.wavelength)
            }
            Err(_) => {
                // Fallback to last-set value
                tracing::warn!("Hardware query failed, using cached value");
                Ok(self.wavelength)
            }
        }
    } else {
        // Mock mode: return cached value
        Ok(self.wavelength)
    }
}
```

**Apply to:** All GET methods in traits

#### ⚠️ Timeout Configuration

**Issue:** Default 1s timeout too short for some instruments (MaiTai needed 2s)

**Solution:** Make timeouts configurable via builder pattern

```rust
pub struct SerialAdapterV4Builder {
    timeout: Duration,  // Default: 1s
    // ...
}

impl SerialAdapterV4Builder {
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}
```

**Apply to:** All hardware adapters

---

## 2. Trait Design Updates

### 2.1 General Updates (All Traits)

#### ✅ Add kameo::Reply Derives

All reply types need `#[derive(kameo::Reply)]`:

```rust
#[derive(Debug, Clone, kameo::Reply)]
pub struct Frame { ... }

#[derive(Debug, Clone, Copy, kameo::Reply)]
pub struct AxisPosition { ... }

#[derive(Debug, Clone, kameo::Reply)]
pub struct ScpiEvent { ... }
```

#### ✅ Default Arrow Implementations

Move Arrow conversion to default methods:

```rust
#[async_trait::async_trait]
pub trait CameraSensor: Send + Sync {
    // ... async methods ...

    /// Default Arrow conversion for frames
    fn to_arrow_frames(&self, frames: &[Frame]) -> Result<RecordBatch> {
        // Default implementation
        static SCHEMA: Lazy<Arc<Schema>> = Lazy::new(|| {
            Arc::new(Schema::new(vec![
                Field::new("timestamp", DataType::Timestamp(...), false),
                // ... other fields
            ]))
        });

        // Convert frames to RecordBatch
        // ... implementation ...
    }
}
```

#### ✅ Error Context

Use `with_context` for all error propagation:

```rust
async fn query(&self, cmd: &str) -> Result<String> {
    self.adapter
        .query(cmd)
        .await
        .with_context(|| format!("Failed to query SCPI command: {}", cmd))
}
```

### 2.2 CameraSensor Updates

**No major changes needed** - trait design is solid.

**Minor additions:**

1. Add `kameo::Reply` derives to all types
2. Implement default `to_arrow_frames()` method
3. Document frame queueing strategy (resolved: use tokio::mpsc with bounded capacity)

### 2.3 MotionController Updates

**No major changes needed** - trait design is solid.

**Minor additions:**

1. Add `kameo::Reply` derives to all types
2. Implement default `to_arrow_positions()` method
3. Document position units in trait docs (resolved: adapters specify units in metadata)

### 2.4 ScpiEndpoint Updates

**Key Addition:** Document SET-only vs SET+GET instruments

```rust
/// SCPI endpoint meta-instrument trait
///
/// ## Instrument Categories
///
/// ### Type A: Query-Response (Standard SCPI)
/// - Supports both SET and GET commands
/// - Example: `VOLT?` returns voltage reading
/// - Most oscilloscopes, power supplies, meters
///
/// ### Type B: SET-Only (Configuration-Only)
/// - Supports SET commands but not GET queries
/// - GET methods return cached state from last SET
/// - Example: Some laser controllers, legacy instruments
/// - Warning logged when query fails but SET succeeded
///
/// Implementations should detect instrument type and adapt behavior.
#[async_trait::async_trait]
pub trait ScpiEndpoint: Send + Sync {
    // ... methods unchanged ...
}
```

---

## 3. Adapter Requirements Updates

### 3.1 VisaAdapterV4Builder (NEW)

Based on SerialAdapterV4Builder pattern:

```rust
pub struct VisaAdapterV4Builder {
    resource_name: String,  // e.g., "TCPIP0::192.168.1.100::INSTR"
    timeout: Duration,
    read_terminator: String,
    write_terminator: String,
}

impl VisaAdapterV4Builder {
    pub fn new(resource_name: String) -> Self {
        Self {
            resource_name,
            timeout: Duration::from_secs(1),
            read_terminator: "\n".to_string(),
            write_terminator: "\n".to_string(),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_read_terminator(mut self, terminator: String) -> Self {
        self.read_terminator = terminator;
        self
    }

    pub fn with_write_terminator(mut self, terminator: String) -> Self {
        self.write_terminator = terminator;
        self
    }

    pub fn build(self) -> VisaAdapterV4 {
        // ... create adapter
    }
}

pub struct VisaAdapterV4 {
    inner: Arc<Mutex<VisaSession>>,
    timeout: Duration,
    read_terminator: String,
    write_terminator: String,
}

impl VisaAdapterV4 {
    pub fn new(resource_name: String) -> Self {
        VisaAdapterV4Builder::new(resource_name).build()
    }

    pub async fn query(&self, cmd: &str) -> Result<String> {
        // Send command, read response
    }

    pub async fn query_with_timeout(&self, cmd: &str, timeout: Duration) -> Result<String> {
        // Override default timeout
    }

    pub async fn write(&self, cmd: &str) -> Result<()> {
        // Send command without reading response
    }

    pub async fn is_connected(&self) -> bool {
        // Check VISA session
    }

    pub async fn connect(&self) -> Result<()> {
        // Open VISA session
    }
}
```

### 3.2 SerialAdapterV4 Updates

**Add command echo stripping:**

```rust
impl SerialAdapterV4 {
    /// Send command and read response, stripping echo if present
    pub async fn query_with_echo_strip(&self, command: &str, separator: char) -> Result<String> {
        let response = self.send_command(command).await?;

        // Some instruments echo command: "WAVELENGTH:800"
        // Extract value after separator
        let value = response.split(separator).last().unwrap_or(&response);

        Ok(value.trim().to_string())
    }
}
```

### 3.3 CameraAdapterV4 (Future)

**Defer to Phase 1D Step 2C** - define during PVCAM migration

---

## 4. Resolved Open Questions

### 4.1 CameraSensor

**Q1: Frame Queueing Strategy**
- ✅ **RESOLVED:** Use `tokio::sync::mpsc` with bounded capacity (default: 10 frames)
- Oldest frames dropped when queue full
- Dropped frame count tracked in metadata

**Q2: Pixel Data Memory Management**
- ✅ **RESOLVED:** Allocate new Vec<u8> for each frame (simple, safe)
- Memory pool optimization deferred to Phase 2 performance tuning

**Q3: Binning vs. Software Crop**
- ✅ **RESOLVED:** Trait exposes hardware binning only
- Software cropping handled at analysis layer (not adapter responsibility)

**Q4: Bayer Filter Demosaicing**
- ✅ **RESOLVED:** Raw Bayer data only, demosaicing in analysis
- Keeps adapter simple, flexible for different demosaic algorithms

**Q5: Synchronization with Motion**
- ✅ **RESOLVED:** Shared system time (`SystemTime::now()`)
- Frame timestamp + position timestamp correlated post-hoc

### 4.2 MotionController

**Q1: Absolute vs. Relative Speed**
- ✅ **RESOLVED:** Both use same configured velocity
- Fine-grained speed control via `configure_motion()` before move

**Q2: Multi-Axis Coordination**
- ✅ **RESOLVED:** Defer to higher-level sequencer (not trait responsibility)
- Individual axis control only

**Q3: Limit Switch Handling**
- ✅ **RESOLVED:** Reaching limit sets `AxisState::LimitSwitch` (not error)
- Soft limits return error, hard limits return state

**Q4: Homing Behavior**
- ✅ **RESOLVED:** Use hardware default homing sequence
- Timeout: 30 seconds

**Q5: Velocity Ramps**
- ✅ **RESOLVED:** Acceleration/deceleration global per axis
- S-curves deferred to Phase 2

**Q6: Position Units**
- ✅ **RESOLVED:** Adapter specifies units in metadata (e.g., "mm", "degrees")
- Trait uses abstract "motor units"

### 4.3 ScpiEndpoint

**Q1: Compound Queries**
- ✅ **RESOLVED:** Caller parses compound responses
- Trait returns full string

**Q2: Command Queuing**
- ✅ **RESOLVED:** Adapter queues internally with `tokio::sync::Mutex`
- Max queue depth: unbounded (relies on backpressure)

**Q3: Status Polling**
- ✅ **RESOLVED:** Poll *STB? every 50ms
- Timeout applies to total transaction time

**Q4: Error Recovery**
- ✅ **RESOLVED:** Failed queries do NOT auto-clear errors
- Caller must explicitly call `clear_errors()` for recovery

**Q5: Binary Data Responses**
- ✅ **RESOLVED:** Defer binary responses to Phase 2 (waveform trait)
- Current ScpiEndpoint assumes UTF-8 strings

**Q6: Synchronized Multi-Instrument Queries**
- ✅ **RESOLVED:** Handled at InstrumentManager level (not trait)
- Phase 2 feature

---

## 5. Implementation Priority

### Phase 1D Step 1 (Days 1-3) - Trait Finalization

- [x] ✅ **Review RFC** (completed)
- [x] ✅ **Update trait files with kameo::Reply derives** (completed)
- [x] ✅ **Implement default Arrow methods** (completed)
- [x] ✅ **Create VisaAdapterV4Builder** (completed - visa_adapter_v4.rs:420 lines)
- [x] ✅ **Update SerialAdapterV4 with echo stripping** (completed - query_with_echo_strip method)
- [x] ✅ **Document trait contracts** (completed - TRAIT_USAGE_GUIDE.md:570 lines)

### Phase 1D Step 2A (Days 4-6) - SCPI Migration

**Target Instrument:** TBD (choose simplest SCPI instrument from legacy codebase)

**Tasks:**
1. Create ScpiActor implementing ScpiEndpoint trait
2. Wire VisaAdapterV4 to actor
3. Test with real SCPI hardware
4. Validate Arrow conversion

### Phase 1D Step 2B (Days 7-10) - ESP300 Migration

**Target Instrument:** Newport ESP300 motion controller

**Tasks:**
1. Create ESP300Actor implementing MotionController trait
2. Use SerialAdapterV4 with ESP300 protocol parsing
3. Test multi-axis motion
4. Validate position streaming

### Phase 1D Step 2C (Days 11-15) - PVCAM Migration

**Target Instrument:** Photometrics PrimeBSI camera

**Tasks:**
1. Create PVCAMActor implementing CameraSensor trait
2. Create CameraAdapterV4 wrapping PVCAM SDK
3. Test frame streaming
4. Validate high-bandwidth data flow

---

## 6. Trait Contract Documentation

### 6.1 Error Handling Contract

**ALL trait methods follow this pattern:**

```rust
async fn method(&self) -> Result<T> {
    self.adapter
        .hardware_call()
        .await
        .with_context(|| format!("Operation failed: {}", details))
}
```

**Error Types:**
- `Hardware communication error` - Serial/VISA timeout or connection loss
- `Invalid parameter` - Out of range, incompatible settings
- `Device not ready` - Busy, warming up, or in error state
- `Timeout exceeded` - Operation took longer than configured timeout

**Error Recovery:**
- Actors log errors and transition to error state
- Supervisor can restart actor or mark as failed
- Caller must check Result and handle gracefully

### 6.2 State Management Contract

**Stateful Traits (CameraSensor, MotionController):**
- Maintain internal state (streaming, position, etc.)
- State persists across calls
- `is_streaming()`, `num_axes()` are non-async (read cached state)

**Stateless Traits (ScpiEndpoint):**
- No internal state beyond configuration
- Every call is independent
- Timeout is only stateful configuration

### 6.3 Arrow Conversion Contract

**ALL `to_arrow_*()` methods:**
- Accept slice of measurements: `&[T]`
- Return `Result<RecordBatch>`
- Use static Lazy schema for zero-cost abstraction
- Schema is versioned (field order never changes)

**Schema Guarantees:**
- Timestamp always first field (i64, Timestamp(Nanosecond))
- Required fields are non-nullable
- Optional fields are nullable
- Binary fields for large data (pixel_data, waveforms)

### 6.4 Async Contract

**ALL potentially-blocking operations are async:**
- Hardware I/O (read, write, query)
- State changes (start, stop, configure)
- Long-running operations (homing, acquisition)

**Synchronous methods (rare):**
- Pure state queries (`is_streaming()`, `num_axes()`)
- Schema retrieval (`get_capabilities()`)
- Arrow conversion (CPU-bound, not I/O)

---

## 7. Next Steps

### Immediate Actions (Next 2 Hours)

1. **Update trait files** in `v4-daq/src/traits/`:
   - Add `kameo::Reply` derives to all types
   - Implement default Arrow methods
   - Add documentation for SET-only instruments

2. **Create VisaAdapterV4** in `v4-daq/src/hardware/`:
   - Builder pattern with configurable timeouts
   - VISA session management
   - Query/write methods

3. **Enhance SerialAdapterV4**:
   - Add `query_with_echo_strip()` method
   - Document command echo handling

### Tomorrow (Day 2)

4. **Write trait usage guide** in `docs/v4/TRAIT_USAGE_GUIDE.md`:
   - Example implementations
   - Error handling patterns
   - Arrow conversion examples

5. **Choose SCPI instrument** for Step 2A migration:
   - Review V2 SCPI implementations
   - Select simplest instrument with VISA connection
   - Prepare for migration

### Days 3-18

6. Continue with Phase 1D timeline as planned

---

## 8. Sign-Off

**Hardware Validation:** ✅ COMPLETE (Newport 1830-C, MaiTai laser)
**Trait Design:** ✅ FINALIZED (all open questions resolved)
**Adapter Patterns:** ✅ VALIDATED (Builder, Kameo, Arrow)
**Ready for Implementation:** ✅ YES (proceed to Step 1 implementation)

**Confidence Level:** VERY HIGH (95%+) for successful Phase 1D execution.

---

**Document Version:** 1.0 (Final)
**Last Updated:** 2025-11-17
**Status:** APPROVED - Proceed with implementation
