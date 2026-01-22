# Codebase Concerns

**Analysis Date:** 2026-01-21

## Tech Debt

**Server-Side unwrap()/expect() in Production Code:**
- Issue: gRPC server implementation uses `.unwrap()` and `.expect()` in multiple paths that can trigger panics under error conditions
- Files: `crates/daq-server/src/grpc/health_service.rs`, `crates/daq-server/src/grpc/server.rs`, `crates/daq-server/src/grpc/scan_service.rs`, `crates/daq-server/src/grpc/plugin_service.rs`
- Impact: Server crashes on Mutex poisoning in health checks, protobuf conversion failures, or channel errors rather than returning proper error responses
- Fix approach: Replace all `.unwrap()` calls with proper `?` operator or explicit error handling; use `into_inner()` for poisoned mutexes with recovery path (pattern already established in `crates/daq-driver-pvcam/src/components/acquisition.rs` lines 215-224)
- Related commits: bd-86hi, bd-5oss (attempted fixes in specific areas; systematic fix needed across server)

**Large File Complexity - PVCAM Acquisition Module:**
- Issue: `crates/daq-driver-pvcam/src/components/acquisition.rs` (3802 LOC) contains complex lifecycle management for EOF callbacks, buffer modes, and frame loss detection
- Files: `crates/daq-driver-pvcam/src/components/acquisition.rs`
- Impact: High risk of regression when modifying acquisition logic; difficult to test all state transitions
- Fix approach: Extract callback context management into separate module; separate EOF callback logic from frame retrieval loop logic; add state machine tests for mode transitions (CIRC_OVERWRITE ↔ CIRC_NO_OVERWRITE)

**Global Static Callback Context (bd-static-ctx-2026-01-12):**
- Issue: PVCAM uses global static `GLOBAL_CALLBACK_CTX` instead of passing context pointer to SDK callback
- Files: `crates/daq-driver-pvcam/src/components/acquisition.rs` lines 427-443
- Impact: Only one camera instance can be active at a time; multiple concurrent camera connections will corrupt shared state
- Fix approach: Investigate SDK callback parameter handling; potentially use index-based lookup table or thread-local storage for multi-camera support; document single-instance limitation in driver API

**PVCAM Callback Reliability Workaround (bd-callback-reliability-2026-01-12):**
- Issue: Code explicitly handles poisoned mutex in callback context to avoid deadlocks
- Files: `crates/daq-driver-pvcam/src/components/acquisition.rs` lines 196-227
- Impact: Masks root cause of mutex poisoning; frames may be counted but lost if panic occurs during callback
- Fix approach: Determine why mutex is being poisoned; implement panic hook to prevent callback thread panic from poisoning; consider using message queue instead of condvar for callback signaling

**Large EGUIPanel Components:**
- Issue: Multiple large UI panels that manage complex state
- Files: `crates/daq-egui/src/panels/image_viewer.rs` (2709 LOC), `crates/daq-egui/src/panels/instrument_manager/mod.rs` (1743 LOC)
- Impact: Difficult to test UI logic in isolation; state management prone to inconsistency
- Fix approach: Extract state management from UI rendering; implement Model-View pattern with testable state machines

**Ring Buffer Unsafe Code Complexity:**
- Issue: Lock-free ring buffer uses 539+ unsafe blocks across codebase for memory manipulation, pointer arithmetic, and memory mapping
- Files: `crates/daq-storage/src/ring_buffer.rs` and related
- Impact: Any error in unsafe code can cause memory corruption, data loss, or undefined behavior
- Fix approach: Audit unsafe code with security-focused review; add invariant checks at unsafe boundaries; consider bounded mutability constraints; document safety invariants for each unsafe block

## Known Bugs

**Multi-Camera PVCAM Limitation (Implicit):**
- Symptoms: Global static callback context means only one PVCAM instance can safely operate
- Files: `crates/daq-driver-pvcam/src/components/acquisition.rs` lines 427-443
- Trigger: Attempting to open multiple cameras simultaneously
- Workaround: Sequence camera operations; use separate daemon processes per camera
- Mitigation: Current code doesn't explicitly prevent this - silent data corruption likely

**Ring Buffer Stream ID Overflow:**
- Symptoms: Cross-process readers may miss buffer re-initialization
- Files: `crates/daq-storage/src/ring_buffer.rs` (stream_id: AtomicU64 at line 75)
- Trigger: After 2^64 re-initializations (extremely unlikely but possible in long-running systems)
- Workaround: None (overflow wraps to 0, confusing readers)
- Priority: Low (requires ~584 billion re-initializations)

**PVCAM Frame Loss Detection Edge Case:**
- Symptoms: Frame discontinuities may not be detected if FrameNr overflows
- Files: `crates/daq-driver-pvcam/src/components/acquisition.rs` lines 44-49
- Trigger: FrameNr is i32; after 2^31 frames (~5-6 hours at 100 FPS), wraps to negative
- Workaround: Restart acquisition before overflow
- Mitigation: Loss detection algorithm doesn't account for i32 overflow; should check for wrap-around

## Security Considerations

**gRPC Port Binding (Default Public):**
- Risk: Default configuration in `config/config.v4.toml` binds to `0.0.0.0` making server accessible from any network interface
- Files: `crates/daq-server/src/grpc/server.rs` (server creation), config files
- Current mitigation: CORS list configured, but no authentication by default (`auth_enabled = false`)
- Recommendations:
  1. Default to `127.0.0.1` (loopback only) in development configs
  2. Require explicit bind address configuration in production
  3. Enable authentication in production deployments
  4. Add warning log when binding to `0.0.0.0` without auth

**Serial Port Privilege Escalation:**
- Risk: HAL drivers open `/dev/ttyUSB*` and `/dev/ttyS*` which require elevated permissions or user group membership
- Files: All `crates/daq-driver-*/src/*.rs` serial port drivers
- Current mitigation: None documented
- Recommendations:
  1. Document required user group setup (`dialout` on Linux)
  2. Implement capability-aware error messages if permission denied
  3. Consider privilege separation for serial access

**Script Size DoS (MAX_SCRIPT_SIZE):**
- Risk: Scripts uploaded via gRPC are limited to 1MB but no rate limiting on uploads
- Files: `crates/daq-core/src/limits.rs` line 60, `crates/daq-server/src/grpc/plugin_service.rs`
- Current mitigation: Size limit only
- Recommendations:
  1. Add per-client rate limiting on script uploads
  2. Implement timeout for script execution
  3. Add memory limits for script engine (Rhai evaluation)

**Frame Data Size Validation:**
- Risk: `MAX_FRAME_BYTES` (100 MB) may allow OOM DoS on clients with limited memory
- Files: `crates/daq-core/src/limits.rs` lines 56, 97-100
- Current mitigation: Size validation exists
- Recommendations:
  1. Make frame size configurable per deployment
  2. Implement per-client streaming backpressure (already implemented per commit messages but verify)
  3. Monitor server-side buffer usage

**Unsafe Pointer Access in PVCAM Callback:**
- Risk: EOF callback dereferences raw pointers without validation
- Files: `crates/daq-driver-pvcam/src/components/acquisition.rs` lines 446-496
- Current mitigation: NULL checks performed, frame_info dereferenced safely
- Recommendations:
  1. Formalize invariants: GLOBAL_CALLBACK_CTX must remain valid during callback registration
  2. Add fence instructions for visibility across callback thread
  3. Consider using crossbeam channels instead of manual pointer management

## Performance Bottlenecks

**PVCAM Fast-Path Frame Loss (bd-fast-path-2026-01-17):**
- Problem: Previous implementation without fast-path check caused deadlocks when multiple frames arrived during interrupt coalescing
- Files: `crates/daq-driver-pvcam/src/components/acquisition.rs` lines 246-249 (comment references issue)
- Cause: Frame arrival rate can exceed processing rate; condvar wakeup only triggers one frame drain cycle
- Improvement path: Current fix checks `pending_frames` before condvar wait (fast path); verify that coalesced multiple frames are properly consumed

**Ring Buffer Seqlock Retry Loop:**
- Problem: Readers may spin on seqlock failures if writer holds lock during critical section
- Files: `crates/daq-storage/src/ring_buffer.rs` (seqlock pattern, lines 93-98)
- Cause: High-frequency writes (10k+ per sec) can cause reader stalls
- Improvement path: Measure contention with performance profiling; consider RCU pattern or reader-friendly lock

**Image Viewer Frame Downsampling:**
- Problem: `downsample_2x2` and `downsample_4x4` executed on main gRPC thread for each client
- Files: `crates/daq-proto/src/downsample.rs` (1463 LOC), `crates/daq-server/src/grpc/hardware_service.rs` (uses it)
- Cause: No async offloading for downsampling
- Improvement path: Spawn downsampling to worker thread pool; pre-cache common downsampled versions

**Hardware Initialization Synchronous Blocking:**
- Problem: Device registration in registry awaits hardware connections; if hardware is slow/unresponsive, entire server startup blocks
- Files: `crates/daq-hardware/src/registry.rs` (async device registration)
- Cause: No timeout or background initialization
- Improvement path: Implement non-blocking device discovery; use separate background task for lazy initialization

## Fragile Areas

**Device Registry Dynamic Dispatch (bd-4x6q):**
- Files: `crates/daq-hardware/src/registry.rs`, `crates/daq-server/src/grpc/hardware_service.rs`
- Why fragile: Capability traits are checked at runtime via `as` casting; no compile-time guarantee device supports requested capability
- Safe modification:
  1. Add `get_capabilities()` method to Device trait that returns known capabilities
  2. Cache capabilities on Device creation
  3. Return explicit error before attempting cast (already done but verify all paths)
- Test coverage: `crates/daq-server/src/grpc/error_mapping_tests.rs` covers error cases; add tests for capability queries

**Parameter Observable Mutations (Reactive State):**
- Files: `crates/daq-core/src/parameter.rs`, `crates/daq-core/src/observable.rs`
- Why fragile: Parameters are mutated via async callbacks from hardware; subscription channels can be full, causing frame drops
- Safe modification:
  1. Verify all parameter mutations use `set()` method (don't bypass via Arc<Mutex<>>)
  2. Test what happens when mpsc channel for observers is full
  3. Implement clear contract: "Changes are best-effort; full channels drop oldest changes"
- Test coverage: Add tests for parameter changes during backpressure

**ELL14 Multi-Rotator Bus (RS-485):**
- Files: `crates/daq-hardware/src/drivers/ell14.rs` (3181 LOC)
- Why fragile: Shared bus with multiple devices; one stuck device can deadlock entire bus
- Safe modification:
  1. Per-device timeout on command/response
  2. Bus-level communication logging to diagnose stuck devices
  3. Add "bus recovery" procedure (toggle DTR, full bus reset)
- Test coverage: `crates/rust-daq/tests/hardware_ell14_protocol_features.rs` (1445 LOC) covers protocol; add bus congestion tests

**Comedi DAQ Driver Initialization (bd-fgsx):**
- Files: `crates/daq-driver-comedi/src/lib.rs` (parallel epic bd-fgsx underway for validation)
- Why fragile: Real-time kernel timing constraints; any missed deadline causes discontinuous data
- Safe modification:
  1. Implement backpressure calculation (bd-prgo task exists for this)
  2. Monitor buffer overruns via Comedi ring buffer statistics
  3. Log timestamps to detect scheduling jitter
- Test coverage: Hardware validation tests in bd-y54s; streaming acquisition tests in bd-fgsx.3

## Scaling Limits

**Single Global Callback Context (PVCAM):**
- Current capacity: 1 camera instance
- Limit: Attempting 2+ concurrent cameras causes shared state corruption
- Scaling path: Replace global static with index-based context lookup or thread-local storage; implement per-camera acquisition loops

**MAX_STREAMS_PER_CLIENT (3 concurrent streams):**
- Current capacity: 3 simultaneous frame streams per client
- Limit: 4th stream rejected; client must stop one to open another
- Scaling path: Make MAX_STREAMS_PER_CLIENT configurable; implement backpressure queue instead of hard rejection

**gRPC Health Check Mutex (daq-server):**
- Current capacity: Unbounded health check requests
- Limit: Statuses HashMap locked during every health check; high concurrency causes mutex contention
- Scaling path: Replace `std::sync::Mutex` with `parking_lot::Mutex` or RwLock; use sharded lock for read-heavy workload

**Ring Buffer Memory Mapping:**
- Current capacity: File-based; size determined at creation, fixed at runtime
- Limit: Cannot grow buffer without recreating file (requires stopping acquisition)
- Scaling path: Consider implementing growable mmap; or implement buffer tiering (fast circular + slow archive)

## Dependencies at Risk

**PVCAM SDK Bindings (pvcam-sys):**
- Risk: Unmaintained wrapper around closed-source C library; SDK version pinned to 7.1.1.118
- Impact: Cannot easily upgrade to newer PVCAM versions; breaking changes in SDK require crate updates
- Migration plan:
  1. Evaluate libpvcam (if published) as alternative
  2. Create abstraction layer (already done in `components/connection.rs`) for easier future migration
  3. Document SDK compatibility matrix

**tokio-serial (Serial Communication):**
- Risk: Less actively maintained than tokio ecosystem; platform-specific issues
- Impact: Serial port timeouts, platform-specific permission errors need custom handling
- Migration plan: Implement wrapper trait in `daq-hardware` to abstract serial layer; allows swapping with `serialport` crate if needed

**egui Docking (egui-dock):**
- Risk: External UI library; breaking API changes possible
- Impact: GUI panel layout serialization may break across versions
- Migration plan: Version-pin egui-dock; implement layout migration tests; consider feature flag for fallback to undocked UI

**Figment Configuration (config loading):**
- Risk: Multiple config merging layers (environment, files) can cause unexpected behavior
- Impact: Configuration precedence bugs hard to debug
- Migration plan: Already implemented `crates/rust-daq/src/config/versioning.rs` for config history; add debug logging for config resolution

## Missing Critical Features

**Multi-Camera PVCAM Support:**
- Problem: Only one camera can operate due to global static callback context
- Blocks: Concurrent stereo imaging, multi-sensor synchronization
- Mitigation: Document limitation; provide sequential operation workaround
- Related issue: bd-rlc0 epic addresses this with zero-copy Arc-backed frames (Phase 1-5)

**Watchdog for Orphaned Plans (bd-c9z1):**
- Problem: Long-running plans can hang if hardware becomes unresponsive
- Blocks: Production deployments requiring fault tolerance
- Current state: Issue tracked but not implemented
- Recommendation: Implement plan timeout with forced cleanup

**Authentication System:**
- Problem: gRPC server has `auth_enabled = false` by default; no API token/credential system
- Blocks: Secure remote operation; multi-user deployments
- Current state: Architecture prepared but not implemented (see `crates/daq-server/src/grpc/server.rs` auth fields)

**Comprehensive Error Recovery:**
- Problem: Serial port errors often result in retry loops without exponential backoff
- Blocks: Graceful handling of temporarily unavailable hardware
- Current state: Per-device error handling varies; no unified retry policy
- Recommendation: Implement circuit breaker pattern for device communication

## Test Coverage Gaps

**gRPC Server Unwrap Paths:**
- What's not tested: Error cases in health service that trigger `.unwrap()` on mutex
- Files: `crates/daq-server/src/grpc/health_service.rs` lines 32, 54, 79
- Risk: Mutex poisoning scenario (panic in concurrent health check) not tested
- Priority: High - production crash scenario
- Test approach: Create mock task that panics during health check, verify server survives

**PVCAM Callback Reliability (Multi-Frame Coalescing):**
- What's not tested: High frame rate coalescing behavior (multiple frames arriving during single condvar wakeup)
- Files: `crates/daq-driver-pvcam/src/components/acquisition.rs` lines 246-260
- Risk: Frame loss or deadlock under load not detected by unit tests
- Priority: High - data acquisition correctness
- Test approach: Hardware test with frame loss injection; simulate high frame rate scenarios

**Ring Buffer Seqlock Under Contention:**
- What's not tested: Reader retry loops under high writer contention
- Files: `crates/daq-storage/src/ring_buffer.rs` (seqlock lines 93-98)
- Risk: Reader stalls not measured in benchmarks
- Priority: Medium - performance regression
- Test approach: Benchmark tool with concurrent reader/writer; measure p99 latency

**ELL14 Bus Recovery (Single Device Failure):**
- What's not tested: What happens when one rotator doesn't respond; does bus hang?
- Files: `crates/daq-hardware/src/drivers/ell14.rs` (3181 LOC)
- Risk: One stuck device blocks all rotators
- Priority: High - hardware reliability
- Test approach: Hardware test with device timeout injection; verify bus recovery procedures

**Serial Port Permission Denied:**
- What's not tested: Driver behavior when `/dev/ttyUSB*` permission denied
- Files: All `crates/daq-driver-*/src/*.rs` serial port open paths
- Risk: Cryptic permission errors instead of helpful guidance
- Priority: Medium - user experience
- Test approach: Unit tests with mocked serial port access denied

---

*Concerns audit: 2026-01-21*
