# VISA Implementation Guide - Corrected Approach

**Purpose**: Guide V2/V4 coexistence implementation with proper VISA SDK understanding
**Status**: Final - Ready for Phase 1E/1F implementation
**Based On**: VISA_SDK_RESEARCH.md findings

---

## 1. Architecture Overview (CORRECTED)

### What We Learned

VISA allows multiple sessions. The "single-session limitation" was a misunderstanding.

```
Old (Incorrect) Design:
┌────────────────────────────────────┐
│  Application                       │
├────────────────────────────────────┤
│  VisaSessionManager (Global)       │
│  - Single VISA session             │
│  - Command queue                   │  <- BOTTLENECK
│  - Worker thread                   │
├────────────────────────────────────┤
│  V2 Instruments  │  V4 Instruments │
└────────────────────────────────────┘

New (Correct) Design:
┌────────────────────────────────────┐
│  V2 Subsystem    │  V4 Subsystem   │
├────────────────────────────────────┤
│  DefaultRM V2    │  DefaultRM V4    │  <- Independent
│  Sessions        │  Sessions        │
├────────────────────────────────────┤
│  Adapter + Mutex │  Adapter + Mutex │  <- Per-instrument serialization
│  (TCPIP Meter)   │  (ESP300)        │
└────────────────────────────────────┘
```

### Key Principle

**Serialize at the instrument level, not the VISA session level.**

---

## 2. Implementation Pattern

### 2.1 V4 Pattern (Current - Correct)

```rust
// src/hardware/visa_adapter_v4.rs
use std::sync::Arc;
use tokio::sync::Mutex;
use visa_rs::{DefaultRM, Instrument};

pub struct VisaAdapterV4 {
    inner: Arc<Mutex<Instrument>>,
    resource_name: String,
    timeout: Duration,
}

impl VisaAdapterV4 {
    pub async fn new(
        resource_name: String,
        timeout: Duration,
    ) -> Result<Self> {
        // V4 creates its own DefaultRM instance
        let rm = DefaultRM::new()?;

        // Open instrument
        let mut instr = rm.open(&resource_name)?;
        instr.set_timeout(timeout.as_millis() as u32)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(instr)),
            resource_name,
            timeout,
        })
    }

    // All SCPI commands go through the mutex
    pub async fn query(&self, cmd: &str) -> Result<String> {
        let mut instr = self.inner.lock().await;

        // Write with terminator
        let write_cmd = format!("{}\n", cmd);
        instr.write_all(write_cmd.as_bytes())?;

        // Read response
        let mut buf = [0u8; 4096];
        instr.read(&mut buf)?;

        Ok(String::from_utf8_lossy(&buf)
            .trim()
            .to_string())
    }

    pub async fn write(&self, cmd: &str) -> Result<()> {
        let mut instr = self.inner.lock().await;
        let write_cmd = format!("{}\n", cmd);
        instr.write_all(write_cmd.as_bytes())?;
        Ok(())
    }
}
```

**Benefits:**
- Simple, clear locking
- One adapter = one instrument = one lock
- Multiple instruments can run in parallel
- Per-SCPI-operation serialization

### 2.2 V2 Pattern (New Understanding)

```rust
// src/adapters/visa_adapter.rs
use std::sync::Arc;
use tokio::sync::Mutex;
use visa_rs::{DefaultRM, Instrument};

impl VisaAdapter {
    pub async fn connect_async(
        &mut self,
        resource: &str,
    ) -> Result<()> {
        // V2 creates its own DefaultRM instance
        // This is INDEPENDENT from V4's DefaultRM
        let rm = DefaultRM::new()?;

        let mut instr = rm.open(resource)?;
        instr.set_timeout(self.timeout.as_millis() as u32)?;

        // Store in mutex for async access
        self.instrument = Some(Arc::new(Mutex::new(instr)));
        self.connected = true;
        Ok(())
    }
}
```

**Key Point**: V2 and V4 each create their own `DefaultRM::new()` instance.

### 2.3 V2/V4 Coexistence Example

```rust
// Pseudo-code showing both systems accessing same instrument

#[tokio::main]
async fn main() -> Result<()> {
    // V2 Subsystem
    let rm_v2 = DefaultRM::new()?;
    let instr_v2 = Arc::new(Mutex::new(
        rm_v2.open("TCPIP0::192.168.1.100::INSTR")?
    ));

    // V4 Subsystem
    let adapter_v4 = VisaAdapterV4::new(
        "TCPIP0::192.168.1.100::INSTR".to_string(),
        Duration::from_secs(2),
    ).await?;
    // Internally creates separate DefaultRM instance

    // Both can now access the same instrument

    // V2 sends command
    let instr = instr_v2.lock().await;
    instr.write_all(b"*IDN?\n")?;
    // ... read response ...
    drop(instr);

    // V4 sends different command (no wait)
    let idn = adapter_v4.query("*IDN?").await?;
    println!("Instrument: {}", idn);

    // Both are independent sessions to the same resource
    // VISA allows this

    Ok(())
}
```

---

## 3. Why This Works

### 3.1 VISA Session Lifecycle

```
VISA SDK allows:
┌─────────────────────────────────────────┐
│ Resource: TCPIP0::192.168.1.100::INSTR  │
├─────────────────────────────────────────┤
│ Session 1 (V2)     │   Session 2 (V4)   │
│ - Open             │   - Open           │
│ - Query            │   - Write          │
│ - Read             │   - Query          │
│ - Close            │   - Close          │
└─────────────────────────────────────────┘

Both sessions are valid simultaneously.
Each maintains its own state.
No ordering constraints between sessions.
```

### 3.2 Command Ordering (SCPI Protocol)

**Per-Session Ordering:**
```
Session 1:  SEND "MEAS:VOLT:DC?"
            -> Waits for response
            -> Gets response (serialized within this session's lock)

Session 2:  (May run concurrently since different session)
            SEND "*IDN?"
            -> Gets response
```

**Per-SCPI-Instrument Ordering:**
```
The instrument itself must receive commands in order.
But VISA/TCP handles this - TCP guarantees ordering.
SCPI instruments queue incoming commands.

So even if V2 and V4 send at same time:
- TCP delivers in order
- Instrument queues both
- Responses come back in order
```

**Lock Serialization ensures:**
```
Thread 1 (V2):          Thread 2 (V4):
lock()                  (waits for lock)
write(query)
read(response)
unlock()                lock()
(done)                  write(query)
                        read(response)
                        unlock()
```

---

## 4. Testing Strategy

### 4.1 Unit Test: Dual Session Access

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]  // Requires VISA-capable instrument
    async fn test_dual_visa_sessions() -> Result<()> {
        // Configure to point to real TCPIP instrument
        const RESOURCE: &str = "TCPIP0::192.168.1.100::INSTR";

        // V2 opens session
        let rm_v2 = DefaultRM::new()?;
        let instr_v2 = Arc::new(Mutex::new(
            rm_v2.open(RESOURCE)?
        ));

        // V4 opens session (separate DefaultRM)
        let adapter_v4 = VisaAdapterV4::new(
            RESOURCE.to_string(),
            Duration::from_secs(2),
        ).await?;

        // Both should be able to query
        let idn_v2 = {
            let mut instr = instr_v2.lock().await;
            instr.write_all(b"*IDN?\n")?;
            let mut buf = [0u8; 256];
            instr.read(&mut buf)?;
            String::from_utf8_lossy(&buf).trim().to_string()
        };

        let idn_v4 = adapter_v4.query("*IDN?").await?;

        // Both should get same response
        assert_eq!(idn_v2, idn_v4);

        // Both should still be connected
        assert!(adapter_v4.is_connected().await);

        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_concurrent_queries_different_instruments() -> Result<()> {
        const METER: &str = "TCPIP0::192.168.1.100::INSTR";
        const PSU: &str = "TCPIP0::192.168.1.101::INSTR";

        // Create adapters for different instruments
        let meter = VisaAdapterV4::new(
            METER.to_string(),
            Duration::from_secs(2),
        ).await?;

        let psu = VisaAdapterV4::new(
            PSU.to_string(),
            Duration::from_secs(2),
        ).await?;

        // Run queries concurrently
        let (meter_idn, psu_idn) = tokio::join!(
            meter.query("*IDN?"),
            psu.query("*IDN?")
        );

        // Both should succeed
        assert!(meter_idn.is_ok());
        assert!(psu_idn.is_ok());

        println!("Meter: {}", meter_idn?);
        println!("PSU: {}", psu_idn?);

        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_command_ordering_within_session() -> Result<()> {
        const RESOURCE: &str = "TCPIP0::192.168.1.100::INSTR";

        let adapter = VisaAdapterV4::new(
            RESOURCE.to_string(),
            Duration::from_secs(2),
        ).await?;

        // Send sequence of commands
        adapter.write("*RST").await?;  // Reset
        adapter.write("*CLS").await?;  // Clear status

        let idn = adapter.query("*IDN?").await?;
        assert!(!idn.is_empty());

        let voltage = adapter.query("MEAS:VOLT:DC?").await?;
        assert!(!voltage.is_empty());

        Ok(())
    }
}
```

### 4.2 Integration Test: V2/V4 Concurrent Access

```rust
#[tokio::test]
#[ignore]  // Requires instrument
async fn test_v2_v4_concurrent_access() -> Result<()> {
    // V2 spawns task
    let v2_handle = tokio::spawn(async {
        // V2 measurement loop
        for i in 0..10 {
            let rm = DefaultRM::new()?;
            let instr = rm.open("TCPIP0::192.168.1.100::INSTR")?;
            // ... take measurements ...
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok::<(), anyhow::Error>(())
    });

    // V4 spawns task
    let v4_handle = tokio::spawn(async {
        // V4 measurement loop
        let adapter = VisaAdapterV4::new(
            "TCPIP0::192.168.1.100::INSTR".to_string(),
            Duration::from_secs(2),
        ).await?;

        for i in 0..10 {
            let _resp = adapter.query("*IDN?").await?;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Ok::<(), anyhow::Error>(())
    });

    // Both should complete without errors
    let (v2_result, v4_result) = tokio::join!(v2_handle, v4_handle);

    v2_result??;
    v4_result??;

    Ok(())
}
```

---

## 5. Performance Characteristics

### 5.1 Lock Contention Analysis

```
Scenario: Single TCPIP instrument accessed by V2 and V4

Per-instrument lock:
├─ Lock acquisition: < 1 μs (tokio::sync::Mutex is optimized)
├─ SCPI command execution: 5-50 ms (network latency dominates)
├─ Lock release: < 1 μs
│
Total: ~5-50 ms per command (network bound, not lock bound)

With per-adapter serialization:
- V2 sends query: takes lock, sends, waits for response (5-50 ms)
- V4 waits for lock (queued, < 1 μs to acquire)
- V4 sends query: holds lock (5-50 ms)
- Both execute sequentially but network latency dominates

Overhead from lock: ~0.1% (negligible)
```

### 5.2 Comparison: With vs Without Serialization

```
WITHOUT per-instrument lock (unsafe):
V2: write()  ─┐
V4: write()  ├─ Both send immediately
              └─ Responses may interleave
              └─ SCPI response mismatch!

WITH per-instrument lock (safe):
V2: lock() → write/read → unlock() ─┐  5-50 ms
V4: (waits) → lock() → write/read → unlock() ─┐  < 1 μs wait + 5-50 ms
```

**Conclusion**: Serialization is necessary and overhead is negligible.

---

## 6. Migration Path

### Phase 1E (Current)

**Current Status**: VisaAdapterV4 is correct, no changes needed.

**Action Items**:
1. Verify both V2 and V4 create independent DefaultRM
2. Add dual-session test
3. Update documentation (remove VisaSessionManager references)

### Phase 1F (VISA Instruments)

**When adding VISA instruments**:
1. Use VisaAdapterV4 pattern for all new instruments
2. Each V4 actor creates/owns its adapter instance
3. Adapters handle their own locking via Arc<Mutex<>>
4. No global VISA manager needed

### Phase 2 (Future)

**If GPIB instruments are needed**:
1. Evaluate GPIB bus arbitration (different from VISA)
2. May need GpibBusArbiter for GPIB-specific serialization
3. TCPIP/USB/Serial: no changes needed

---

## 7. Common Pitfalls and Solutions

### Pitfall 1: Sharing DefaultRM Between V2 and V4

❌ **Wrong:**
```rust
pub static VISA_RM: OnceLock<Arc<DefaultRM>> = OnceLock::new();

// V2 uses
let rm = VISA_RM.get_or_init(|| Arc::new(DefaultRM::new().unwrap()));

// V4 uses same RM
let rm = VISA_RM.get().unwrap();  // Same instance!

// If V2 closes its session -> breaks V4's sessions
```

✓ **Correct:**
```rust
// V2 subsystem
pub static VISA_RM_V2: OnceLock<DefaultRM> = OnceLock::new();

// V4 subsystem
pub static VISA_RM_V4: OnceLock<DefaultRM> = OnceLock::new();

// Each has independent lifecycle
```

### Pitfall 2: Not Locking Around VISA Operations

❌ **Wrong:**
```rust
pub struct Adapter {
    instr: Instrument,  // Shared without mutex
}

async fn query(&self, cmd: &str) -> Result<String> {
    // Thread 1 and Thread 2 both call this
    // Concurrent write/read on same socket!
    // => Corrupted SCPI responses
}
```

✓ **Correct:**
```rust
pub struct Adapter {
    instr: Arc<Mutex<Instrument>>,
}

async fn query(&self, cmd: &str) -> Result<String> {
    let mut instr = self.inner.lock().await;
    // Only one thread at a time
    // SCPI commands serialized
}
```

### Pitfall 3: Assuming Multiple Sessions to Same Resource Fail

❌ **Wrong:**
```rust
// Believe this will fail
let session1 = rm.open("TCPIP0::192.168.1.100::INSTR")?;  // OK
let session2 = rm.open("TCPIP0::192.168.1.100::INSTR")?;  // Fails?
```

✓ **Correct:**
```rust
// Both sessions are valid
let session1 = rm.open("TCPIP0::192.168.1.100::INSTR")?;  // OK
let session2 = rm.open("TCPIP0::192.168.1.100::INSTR")?;  // Also OK!

// Each is independent
// Both can be used concurrently with proper locking
```

---

## 8. Configuration and Deployment

### 8.1 Feature Flags

```toml
[features]
instrument_visa = ["dep:visa-rs"]
```

Keep existing feature flag. When not using VISA:
- Adapters return error
- V4 instruments still work with mock
- No VISA dependency required

### 8.2 Documentation for Users

**Best Practice Checklist:**

- [ ] Each VISA instrument gets its own adapter instance
- [ ] Adapter wraps Instrument in Arc<Mutex<>>
- [ ] All SCPI operations go through the mutex
- [ ] V2 and V4 each create their own DefaultRM
- [ ] Don't share DefaultRM between subsystems
- [ ] Don't attempt to create global VISA session manager
- [ ] Test with multiple V4 instruments accessing same physical device

---

## 9. Summary

### What Changed

**Before**: Misunderstanding that VISA only allows one session per resource
**After**: Correct understanding - multiple sessions allowed, just need per-operation locking

### What Stays the Same

- VisaAdapterV4 implementation is correct
- Per-instrument Arc<Mutex<>> pattern is correct
- No global coordination needed

### What's Different

- V2 and V4 each create independent DefaultRM
- No VisaSessionManager needed
- Simpler, faster, more correct

### Key Takeaway

**Lock at the operation level, not the session level.**

Each VISA adapter serializes its own operations. That's all that's needed.

---

## References

- **Full Research**: `VISA_SDK_RESEARCH.md`
- **Current Code**: `/src/hardware/visa_adapter_v4.rs`
- **Tests**: `/tests/` (add dual-session tests here)
- **VISA Docs**: NI-VISA Programmer Reference Manual
- **SCPI Spec**: IEEE 488.2, SCPI-99

