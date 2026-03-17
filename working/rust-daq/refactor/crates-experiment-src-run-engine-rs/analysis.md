Refactoring Analysis: crates/experiment/src/run_engine.rs

Target:
- Path: crates/experiment/src/run_engine.rs
- Scope: file
- Concern: extreme complexity, file size (monolith), and violation of SRP

Summary:
The `RunEngine` is the central orchestrator of the system, responsible for executing declarative experiment plans. At 3,766 lines, it has grown into a massive monolith that mixes state management, command execution, queueing, hardware readiness validation, data orchestration, and watchdog lifecycle management. It severely violates the Single Responsibility Principle (SRP) and creates immense friction for AI coding agents and human developers alike.

Code Smells Identified:

1. Long File / God Object (`RunEngine`)
   - Location: `run_engine.rs` (3,766 lines)
   - Description: The `RunEngine` struct and its implementation encompass too many domains: state transitions, plan parsing, hardware validation, queueing, and error recovery.
   - Impact: High cognitive load, frequent merge conflicts, context window exhaustion for AI agents.

2. Long Functions (`execute_plan`, `process_command`, `readiness_issues_for_devices`)
   - Location: Inside `impl RunEngine`
   - Description: Deeply nested and extremely long functions handling complex asynchronous hardware interactions.
   - Impact: Extremely hard to test in isolation, high risk of regressions when modifying async control flow.

3. Tight Coupling of State and Execution
   - Location: Throughout `run_engine.rs`
   - Description: The execution logic (e.g., `execute_plan`) is tightly coupled to the `run_context` Mutex, intertwining execution steps with global state updates.
   - Impact: Difficult to refactor execution logic without impacting state transitions; hinders concurrent execution capabilities.

Suggested Refactorings:

1. Extract `state_machine.rs`
   - Type: Extract Module
   - Target: `EngineState` and core transition logic (`start`, `pause`, `resume`, `abort`).
   - Rationale: Isolates state rules, making them easier to test and reason about independently from execution.
   - Blast Radius: `run_engine.rs`, `server` crate (gRPC endpoints interacting with state).
   - Risk: Medium — Must ensure async state changes remain atomic.
   - Test Impact: Requires new unit tests for state transitions.

2. Extract `executor.rs`
   - Type: Extract Module / Strategy Pattern
   - Target: `execute_plan` and `process_command` functions.
   - Rationale: Separates the interpretation and execution of `PlanCommand` variants from the engine's lifecycle management. This should ideally take a context object.
   - Blast Radius: `run_engine.rs`, `integration-tests`
   - Risk: High — Execution relies on complex `tokio::select!` loops and cancellation safety. Extreme care needed.
   - Test Impact: Allows for unit testing individual command execution logic by mocking the hardware registry.

3. Extract `readiness.rs`
   - Type: Extract Module
   - Target: `CalibrationFreshness`, `RunReadinessIssue`, and `readiness_issues_for_devices`.
   - Rationale: Hardware readiness checks are purely functional and distinct from engine execution.
   - Blast Radius: `run_engine.rs`, `server` (which likely queries readiness).
   - Risk: Low — Pure functional extraction.
   - Test Impact: High value. Can easily write tests for various hardware state permutations.

4. Extract `task_queue.rs`
   - Type: Extract Module
   - Target: `plan_queue` and `QueuedPlan` logic.
   - Rationale: Queue management is a distinct concern from execution.
   - Blast Radius: `run_engine.rs`
   - Risk: Low — Standard data structure extraction.
   - Test Impact: Straightforward unit tests for FIFO queue logic.

5. Extract `watchdog.rs`
   - Type: Extract Module
   - Target: Activity tracking and the background watchdog task (`spawn_watchdog`).
   - Rationale: Lifecycle management should not clutter the main engine file.
   - Blast Radius: `run_engine.rs`
   - Risk: Low — Isolated background task.
   - Test Impact: Can be tested via timing/mocking channels.

Recommended Order:
1. Extract `task_queue.rs` (Lowest risk, fewest dependencies).
2. Extract `readiness.rs` (Low risk, pure functions, immediately reduces file size).
3. Extract `watchdog.rs` (Low risk, isolated background task).
4. Extract `state_machine.rs` (Medium risk, establishes clear boundaries before touching execution).
5. Extract `executor.rs` (Highest risk, deeply coupled, requires careful async refactoring; do this last once other concerns are cleared out).