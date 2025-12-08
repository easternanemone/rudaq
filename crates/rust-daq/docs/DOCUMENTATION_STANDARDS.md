# Documentation Standards

This document defines **mandatory** documentation standards for the rust-daq project. All developers and AI agents **MUST** follow these rules without exception.

## Enforcement

Documentation is enforced via Cargo lints in `Cargo.toml`:

```toml
[lints.rust]
missing_docs = "warn"  # Will be upgraded to "deny" once baseline is established
```

**Goal**: Zero `missing_docs` warnings in CI.

---

## Core Principles

### 1. Document Intent, Not Mechanics

```rust
// BAD: Describes what the code does mechanically
/// Increments the counter by one.
fn advance(&mut self) { self.count += 1; }

// GOOD: Describes WHY and WHEN to use it
/// Advances to the next acquisition frame.
///
/// Call this after processing each frame to maintain synchronization
/// with the hardware trigger signal.
fn advance(&mut self) { self.count += 1; }
```

### 2. Every Public Item Must Be Documented

**No exceptions.** This includes:
- Modules (`//!` at top of file)
- Structs and enums
- All struct fields (even "obvious" ones)
- All enum variants
- Functions and methods
- Traits and trait methods
- Type aliases
- Constants

### 3. Documentation Is Part of the API

If you change behavior, update the documentation in the **same commit**.

---

## Module Documentation

Every module file **MUST** start with a module-level doc comment:

```rust
//! Camera acquisition pipeline.
//!
//! This module handles frame acquisition from PVCAM-compatible cameras,
//! including buffering, triggering, and metadata extraction.
//!
//! # Architecture
//!
//! ```text
//! Hardware → RingBuffer → HDF5Writer
//!              ↓
//!          FFT Pipeline
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use rust_daq::hardware::pvcam::PvcamDriver;
//!
//! let camera = PvcamDriver::open("cam0")?;
//! camera.set_exposure(0.1)?;
//! let frame = camera.acquire_single().await?;
//! ```
```

### Required Sections for Modules

| Section | When Required |
|---------|---------------|
| Brief description | Always |
| `# Architecture` | Complex modules with multiple components |
| `# Example` | Public API modules |
| `# Safety` | Modules with unsafe code |
| `# Panics` | If any public function can panic |
| `# Errors` | If functions return Result types |

---

## Struct Documentation

```rust
/// Configuration for ring buffer memory allocation.
///
/// The ring buffer uses memory-mapped files for zero-copy data transfer
/// between the acquisition thread and storage writers.
///
/// # Example
///
/// ```rust
/// let config = RingBufferConfig {
///     capacity_bytes: 1024 * 1024 * 100, // 100 MB
///     num_channels: 4,
///     backing_file: Some(PathBuf::from("/tmp/buffer.dat")),
/// };
/// ```
pub struct RingBufferConfig {
    /// Total buffer capacity in bytes.
    ///
    /// Must be a power of 2 for efficient index wrapping.
    /// Minimum: 4096 bytes. Maximum: 1 GB.
    pub capacity_bytes: usize,

    /// Number of independent data channels.
    ///
    /// Each channel gets `capacity_bytes / num_channels` of buffer space.
    pub num_channels: u32,

    /// Optional backing file for memory-mapped storage.
    ///
    /// If `None`, uses anonymous memory mapping (data lost on crash).
    /// If `Some`, enables crash recovery and inspection with external tools.
    pub backing_file: Option<PathBuf>,
}
```

### Field Documentation Rules

1. **Always document every field** - even if "obvious"
2. **Include units** for numeric fields (bytes, seconds, Hz, etc.)
3. **Document constraints** (min/max values, valid ranges)
4. **Document default behavior** if applicable
5. **Explain None/Some semantics** for Option fields

---

## Enum Documentation

```rust
/// State of the run engine during experiment execution.
///
/// The engine progresses through states in a defined order:
/// `Idle` → `Running` → (`Paused` ↔ `Running`) → `Idle`
///
/// # State Diagram
///
/// ```text
///          start()
/// [Idle] ─────────→ [Running] ←──┐
///   ↑                    │       │ resume()
///   │ complete/abort     │pause()│
///   │                    ↓       │
///   └─────────────── [Paused] ───┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    /// No plan is executing. Ready to accept new plans.
    Idle,

    /// Actively executing a plan. Emitting events.
    Running,

    /// Execution suspended. Can resume or abort.
    ///
    /// Hardware remains in its current state (shutters open, stages positioned).
    Paused,

    /// Emergency stop in progress. Cannot resume.
    ///
    /// All hardware is being returned to safe state.
    Aborting,

    /// Hard stop completed. Manual intervention required.
    ///
    /// This state indicates a safety condition was triggered.
    /// Check hardware before resuming operations.
    Halted,
}
```

---

## Function Documentation

```rust
/// Moves the stage to an absolute position.
///
/// This is a blocking operation that waits for the move to complete
/// before returning. For non-blocking moves, use [`move_abs_async`].
///
/// # Arguments
///
/// * `position` - Target position in millimeters. Must be within
///   the stage's travel limits (see [`get_limits`]).
///
/// # Returns
///
/// The actual position reached, which may differ from the target
/// due to encoder resolution or limit clamping.
///
/// # Errors
///
/// * [`StageError::OutOfRange`] - Position outside travel limits
/// * [`StageError::Communication`] - Serial communication failed
/// * [`StageError::NotHomed`] - Stage requires homing first
///
/// # Example
///
/// ```rust,no_run
/// let stage = Esp300::connect("/dev/ttyUSB0").await?;
/// let actual = stage.move_abs(25.0).await?;
/// println!("Reached position: {:.3} mm", actual);
/// ```
///
/// # Panics
///
/// Panics if called from within an async context without a runtime.
/// Use the async variant in async code.
pub async fn move_abs(&self, position: f64) -> Result<f64, StageError> {
    // ...
}
```

### Required Sections for Functions

| Section | When Required |
|---------|---------------|
| Brief description | Always |
| `# Arguments` | Functions with parameters |
| `# Returns` | Non-void functions (unless obvious) |
| `# Errors` | Functions returning `Result` |
| `# Panics` | Functions that can panic |
| `# Safety` | Unsafe functions |
| `# Example` | Public API functions |

---

## Trait Documentation

```rust
/// Capability for devices that can move to absolute positions.
///
/// Implement this trait for stages, rotation mounts, and other
/// positioning hardware.
///
/// # Implementation Notes
///
/// - All positions are in the device's native units (usually mm or degrees)
/// - Implementations must handle concurrent access safely
/// - The `wait_settled` method should use hardware-specific settling criteria
///
/// # Example Implementation
///
/// ```rust,ignore
/// #[async_trait]
/// impl Movable for MyStage {
///     async fn move_abs(&self, position: f64) -> Result<()> {
///         self.send_command(&format!("PA{}", position)).await?;
///         self.wait_settled().await
///     }
///     // ...
/// }
/// ```
#[async_trait]
pub trait Movable: Send + Sync {
    /// Moves to an absolute position.
    ///
    /// Blocks until the move completes or times out.
    async fn move_abs(&self, position: f64) -> Result<()>;

    /// Returns the current position.
    ///
    /// For encodered stages, this reads the encoder.
    /// For open-loop stages, this returns the commanded position.
    async fn get_position(&self) -> Result<f64>;

    /// Waits for the stage to settle after a move.
    ///
    /// Default implementation polls `get_position` until stable.
    /// Override for hardware-specific settling detection.
    async fn wait_settled(&self) -> Result<()> {
        // default implementation
    }
}
```

---

## Safety Documentation

For `unsafe` code, **SAFETY comments are mandatory** (see M-UNSAFE guideline):

```rust
/// Reads raw frame data from the camera buffer.
///
/// # Safety
///
/// Caller must ensure:
/// - `buffer` points to valid memory of at least `size` bytes
/// - No other thread is writing to `buffer` during this call
/// - The camera has been armed and a frame is ready
///
/// # Example
///
/// ```rust,ignore
/// let mut buffer = vec![0u8; frame_size];
/// // SAFETY: buffer is freshly allocated with correct size,
/// // we have exclusive access, and check_frame_ready() returned true
/// unsafe {
///     camera.read_frame_raw(buffer.as_mut_ptr(), buffer.len())?;
/// }
/// ```
pub unsafe fn read_frame_raw(&self, buffer: *mut u8, size: usize) -> Result<()> {
    // ...
}
```

**Inside unsafe blocks**, always add a `// SAFETY:` comment:

```rust
// SAFETY: pl_cam_close is called with a valid handle obtained from pl_cam_open.
// The handle is owned by this struct and will not be used after this call.
unsafe {
    pl_cam_close(self.handle);
}
```

---

## Error Type Documentation

```rust
/// Errors that can occur during stage operations.
///
/// # Error Handling Strategy
///
/// - `Communication` errors are usually transient; retry with backoff
/// - `OutOfRange` errors indicate programmer error; fix the calling code
/// - `Hardware` errors require operator intervention
#[derive(Debug, thiserror::Error)]
pub enum StageError {
    /// Serial port communication failed.
    ///
    /// Check cable connections and port permissions.
    #[error("communication error: {0}")]
    Communication(#[from] std::io::Error),

    /// Requested position is outside travel limits.
    ///
    /// Use [`Stage::get_limits`] to query valid range.
    #[error("position {position} outside limits [{min}, {max}]")]
    OutOfRange {
        /// The requested position
        position: f64,
        /// Minimum allowed position
        min: f64,
        /// Maximum allowed position
        max: f64,
    },

    /// Stage reported a hardware fault.
    ///
    /// The stage may need to be power-cycled or serviced.
    #[error("hardware fault: {0}")]
    Hardware(String),
}
```

---

## Generated Code (Proto/gRPC)

For generated code (protobuf, gRPC), document in the `.proto` file:

```protobuf
// Position information for a movable device.
//
// All positions are in device-native units (mm for linear stages,
// degrees for rotation stages).
message PositionInfo {
  // Device identifier (e.g., "stage_x", "rotation_1")
  string device_id = 1;

  // Current position in device units
  double current_position = 2;

  // Target position if a move is in progress, absent otherwise
  optional double target_position = 3;

  // True if the device is currently moving
  bool is_moving = 4;
}
```

---

## Documentation Review Checklist

Before submitting code, verify:

- [ ] All public items have doc comments
- [ ] Module has `//!` header explaining purpose
- [ ] Structs document all fields with units/constraints
- [ ] Functions document arguments, returns, errors, panics
- [ ] Unsafe code has SAFETY comments (both doc and inline)
- [ ] Examples compile (test with `cargo test --doc`)
- [ ] No `missing_docs` warnings from `cargo check`

---

## AI Agent Instructions

When AI agents add documentation:

1. **Read the implementation first** - understand what the code does
2. **Focus on WHY, not WHAT** - explain intent and usage, not mechanics
3. **Include realistic examples** - show actual use cases
4. **Document edge cases** - what happens with None, empty, zero?
5. **Use consistent terminology** - match existing project vocabulary
6. **Don't over-document** - internal helpers can have brief docs
7. **Verify with `cargo doc`** - ensure documentation renders correctly

---

## Exceptions

The following are **not required** to have documentation:

1. Private items (non-`pub`)
2. Test modules and test functions
3. Build scripts (`build.rs`)
4. Example binaries (but should have comments explaining usage)
5. Items with `#[doc(hidden)]` attribute

To suppress warnings for intentionally undocumented items:

```rust
#[expect(missing_docs, reason = "internal implementation detail")]
pub(crate) struct InternalHelper;
```

---

## References

- [Rust API Guidelines - Documentation](https://rust-lang.github.io/api-guidelines/documentation.html)
- [The Rustdoc Book](https://doc.rust-lang.org/rustdoc/)
- [Microsoft Pragmatic Rust Guidelines](../CODEBASE_ANALYSIS.md#section-11-comprehensive-microsoft-rust-guidelines-analysis)
