# PVCAM Circular Buffer & Callback Parity Review
**Timestamp:** 2026-01-12

## Summary
- Rust follows the SDK callback pattern (store FRAME_INFO, call `pl_exp_get_latest_frame` in the callback, signal condvar) but diverges in buffer mode, buffer sizing, and per-frame memory ownership.
- Major performance gap stems from per-frame heap allocation/copy in Rust; C++ uses zero-copy from the circular buffer.
- Rust mixes `get_latest_frame` (callback) with `get_oldest_frame`/unlock in NO_OVERWRITE mode, unlike the C++ sample’s single latest-frame path.
- Metadata decoding and extensive validation in the Rust hot path add overhead absent in the C++ sample.

## Key Code References
- Rust callback pattern: `crates/driver-pvcam/src/components/acquisition.rs:369-416`
- Rust buffer setup/start & callback registration: `crates/driver-pvcam/src/components/acquisition.rs:1033-1161`
- Rust circular buffer allocation & size: `crates/driver-pvcam/src/components/acquisition.rs:1110-1154`
- Rust frame drain (callback + FIFO fallback) and per-frame copy: `crates/driver-pvcam/src/components/acquisition.rs:2346-2845` (copy at `sdk_bytes.to_vec()` around 2691-2704)
- C++ reference (SDK sample): `/opt/pvcam/sdk/examples/code_samples/src/LiveImage/LiveImage.cpp:21-63` (callback), `95-142` (buffer alloc/start), `143-206` (main loop wait/consume)

## Alignment with SDK Sample
- **Matches:** Callback stores `FRAME_INFO`, calls `pl_exp_get_latest_frame` in callback, signals condvar.
- **Differs:**  
  - Buffer mode: Rust prefers `CIRC_NO_OVERWRITE` (fallback from overwrite); C++ sample uses `CIRC_OVERWRITE`.  
  - Buffer sizing: Rust uses PARAM_FRAME_BUFFER_SIZE + 1s heuristic (clamped 32–255); C++ sample uses fixed frame count (20).  
  - Frame ownership: Rust copies each frame (`to_vec`); C++ zero-copies from circular buffer.  
  - Retrieval path: Rust mixes callback latest-frame with `pl_exp_get_oldest_frame_ex` + unlock in NO_OVERWRITE; C++ uses latest-frame only.  
  - Extra Rust logic: metadata decode, duplicate/zero-frame checks, stall detection/restart, multi-channel fan-out.

## Findings
1. **Per-frame heap allocation/copy** (Rust)  
   - Location: `acquisition.rs:2691-2704` (`sdk_bytes.to_vec()`).  
   - Impact: ~8 MB alloc/copy per 2048×2048 frame; at 50 FPS this is hundreds of MB/s in allocations. C++ keeps zero-copy pointers into the circular buffer.

2. **Buffer mode mismatch**  
   - Rust defaults to NO_OVERWRITE after probing, to avoid error 185; C++ sample runs OVERWRITE. Semantics differ (FIFO drain vs overwrite).

3. **Mixed retrieval paths**  
   - Rust: callback (latest-frame) plus FIFO (`get_oldest_frame` + `unlock_oldest_frame`) when NO_OVERWRITE. C++: latest-frame only.

4. **Hot-path overhead**  
   - Metadata decode, zero-frame sampling, duplicate detection, stall/restart logic, multi-channel broadcast/reliable delivery—all absent in the C++ sample. These add CPU and latency in the Rust loop.

5. **Buffer sizing strategy**  
   - Rust: SDK recommendation or ~1s worth, clamped 32–255. C++ sample: fixed 20 frames. Differences can change overrun/stall behavior and footprint.

## Proposed Modifications (with rationale)
1. **Introduce zero-copy frame ownership (pool/lease)**  
   - Replace per-frame `to_vec` with a slot-lease model (owning the circular buffer slot until consumers finish), or a reusable pool that avoids heap alloc/copy.  
   - Rationale: Match C++ zero-copy behavior; primary FPS bottleneck removal.

2. **Unify retrieval path**  
   - In OVERWRITE mode, rely solely on callback + `pl_exp_get_latest_frame` (no FIFO). In NO_OVERWRITE, keep FIFO path but guard with explicit mode flag.  
   - Rationale: Closer to SDK sample, fewer code paths, less risk of stale/duplicate frames.

3. **Buffer mode alignment & configurability**  
   - Try OVERWRITE first when hardware advertises it; keep NO_OVERWRITE fallback. Expose a config toggle to compare behaviors and match C++ semantics for benchmarking.  
   - Rationale: Align semantics; allow A/B against the sample.

4. **Lean hot path**  
   - Make metadata decode and validation (zero-frame/duplicate) optional/tunable; move heavier checks off the frame loop when performance-critical.  
   - Rationale: Reduce per-frame CPU overhead while keeping diagnostics available.

5. **Explicit buffer sizing options**  
   - Keep current PARAM_FRAME_BUFFER_SIZE heuristic, but add a “fixed frame count” option (e.g., 20 or 255) for parity testing with the SDK sample.  
   - Rationale: Facilitate apples-to-apples comparison and controlled memory footprint.

6. **Post-send unlock discipline (NO_OVERWRITE path)**  
   - Ensure unlock happens only after consumers finish with the slot (or after copy if copy is retained); if moving to zero-copy, gate unlock on lease completion.  
   - Rationale: Prevent buffer starvation/stall and mirror C++ behavior where buffer is reused only after consumption.

## Design Outlines (draft)
- **Zero-copy ownership (bd-svfi):** Slot lease tied to FRAME_INFO, unlocking performed when the lease drops. Provide pooled fallback that clones only for reliable/buffered fan-out channels needing ownership. Keep metadata pointer/length in the lease to avoid extra parsing when disabled. Retain `pl_exp_unlock_oldest_frame` only in NO_OVERWRITE mode after lease completion; no unlock in overwrite mode.
- **Buffer mode policy (bd-hsxc):** Probe OVERWRITE first; if unsupported or error 185, fall back to NO_OVERWRITE. Add config toggle to force mode for parity testing. Retrieval path matches mode: OVERWRITE uses callback + latest-frame only (no FIFO); NO_OVERWRITE uses FIFO oldest-frame + unlock. Log selected mode and frame count, warn when forced fallback occurs.

## Validation Plan
- Benchmark Rust with/without zero-copy against C++ sample on the same hardware (Prime BSI, maitai).  
- Measure FPS, CPU, allocation rate; confirm no stalls/overruns.  
- Test both buffer modes (OVERWRITE vs NO_OVERWRITE) with identical frame counts.  
- If metadata is enabled, verify frame_bytes and pixel offset handling under zero-copy.
