# DAQ System Architecture

## 1. Overview

This document outlines the actor-based architecture of the Rust DAQ system. The system is built on [Tokio](https://tokio.rs/), an asynchronous runtime for Rust, and uses message-passing for communication between concurrent components.

The primary goals of this architecture are:
- **Concurrency**: Safely manage multiple instruments, data storage, and a user interface simultaneously
- **Scalability**: Easily add new instruments or data consumers without major refactoring
- **Robustness**: Prevent common concurrency issues like deadlocks and race conditions by avoiding shared mutable state
- **Clarity**: Ensure the flow of data and control is easy to follow and reason about

This design supersedes a previous implementation based on `Arc<Mutex<T>>`, which suffered from lock contention and complex state management.

## 2. Architecture Diagram

```mermaid
graph LR
    subgraph "User Interface (Main Thread)"
        GUI[GUI - egui]
    end

    subgraph "Async Runtime (Tokio)"
        subgraph "Central Coordinator"
            DaqManager[DaqManagerActor]
        end

        subgraph "Data Distribution"
            DataDist[DataDistributor<br/>(tokio::sync::broadcast)]
        end

        subgraph "Worker Tasks (Dynamically Spawned)"
            InstrumentTask[Instrument Task 1..N]
            StorageWriter[Storage Writer Task<br/><i>(Optional)</i>]
        end
    end

    %% Command & Control Flow
    GUI -- DaqCommand<br/>(mpsc channel) --> DaqManager
    DaqManager -- Response<br/>(oneshot channel) --> GUI
    DaqManager -.->|spawns/manages| InstrumentTask
    DaqManager -- InstrumentCommand<br/>(mpsc channel) --> InstrumentTask
    DaqManager -.->|spawns/manages| StorageWriter
    DaqManager -- Shutdown<br/>(oneshot channel) --> StorageWriter

    %% Data Flow
    InstrumentTask -- Measurement --> DataDist
    DataDist -- subscribes --> StorageWriter
    DataDist -- subscribes --> GUI
```

## 3. Core Components

The system is composed of several key actors and tasks that run concurrently.

### 3.1. DaqManagerActor

The `DaqManagerActor` is the central coordinator of the entire system.

- **Ownership**: It is the sole owner of all shared application state, including the list of active instruments, storage configuration, and session metadata
- **Lifecycle Management**: It is responsible for spawning, monitoring, and shutting down all `Instrument` and `Storage Writer` tasks
- **Command Hub**: It runs in a dedicated Tokio task and processes `DaqCommand` messages received from the GUI via an MPSC (multi-producer, single-consumer) channel
- **State Machine**: It acts as the primary state machine for the application (e.g., transitioning between `Idle`, `Acquiring`, and `Recording` states)

**Implementation**: See [`src/app_actor.rs`](src/app_actor.rs)

### 3.2. Instrument Task

Each connected instrument runs in its own dedicated Tokio task.

- **Isolation**: This isolates instrument-specific logic and I/O, preventing a slow or misbehaving instrument from blocking others
- **Responsibilities**:
  1. Communicating with the physical hardware
  2. Receiving `InstrumentCommand`s (e.g., `SetParameter`, `Shutdown`) from the `DaqManagerActor`
  3. Publishing acquired data to the central `DataDistributor`
- **Event Loop**: Uses `tokio::select!` to handle:
  - Data acquisition from instrument stream
  - Commands from actor via mpsc channel (capacity: 32)
  - Idle timeout (1 second)
- **Shutdown**: Implements a graceful shutdown protocol upon receiving a `Shutdown` command, ensuring the hardware is properly disconnected

**Implementation**: See [`src/app_actor.rs`](src/app_actor.rs) (spawn_instrument method, lines 300-463)

### 3.3. Storage Writer Task

This task is responsible for writing acquired data to disk.

- **Dynamic Lifecycle**: It is spawned by the `DaqManagerActor` only when recording is active and is shut down when recording stops
- **Data Consumer**: It subscribes to the `DataDistributor` to receive the same data stream as the GUI
- **Decoupling**: It is completely decoupled from the instruments. It does not know or care where the data comes from, only that it must be written
- **Formats**: Supports CSV, HDF5, and Arrow (via feature flags)
- **Shutdown**: It uses a oneshot channel for a shutdown signal and has a 5-second timeout to ensure all buffered data is flushed to disk before terminating

**Implementation**: See [`src/app_actor.rs`](src/app_actor.rs) (start_recording method, lines 569-649)

### 3.4. DataDistributor

This is not an actor but a central communication primitive.

- **Implementation**: A `tokio::sync::broadcast` channel (capacity: 1024, configurable)
- **Purpose**: Acts as a publish-subscribe hub for instrument data. `Instrument Tasks` are the publishers. The `GUI` and `Storage Writer Task` are subscribers
- **Benefit**: This decouples data producers from consumers. New consumers (e.g., a network streaming service) can be added simply by creating a new subscriber, with no changes required to the instrument tasks
- **Lagging Behavior**: If a subscriber is slow, it will lag and miss messages but will not block the producer or other consumers

**Implementation**: See [`src/measurement/data_distributor.rs`](src/measurement/data_distributor.rs)

## 4. Communication Patterns

The system exclusively uses message passing over asynchronous channels instead of shared memory with mutexes.

### 4.1. tokio::sync::mpsc (Multi-Producer, Single-Consumer)

Used for command queues where one actor receives commands from multiple sources (though in our case, it's typically single-producer).

- **Use Cases**:
  - GUI → `DaqManagerActor` (capacity: 32)
  - `DaqManagerActor` → `Instrument Task` (capacity: 32)
- **Why**: Perfect for sending commands that don't need an immediate, unique response. It provides a buffer to handle command bursts
- **Non-blocking**: Sender can buffer commands without waiting for receiver

### 4.2. tokio::sync::oneshot (Single-Producer, Single-Consumer)

Used for request/response patterns.

- **Use Cases**:
  - `DaqManagerActor` → GUI (for responses to commands like `GetInstrumentList`)
  - `DaqManagerActor` → `Storage Writer Task` (for shutdown)
- **Why**: A lightweight, highly efficient way to send a single value. Ideal for returning the result of an operation or signaling completion
- **Type-safe**: Each command variant has its own response type

### 4.3. tokio::sync::broadcast (Single-Producer, Multi-Consumer)

Used for one-to-many data distribution.

- **Use Case**: The `DataDistributor`
- **Why**: Allows multiple, independent consumers to receive the same stream of data. If one consumer is slow, it will lag and miss messages but will not block the producer or other consumers
- **Capacity**: 1024 messages (configurable in settings)

**Message Protocol**: See [`src/messages.rs`](src/messages.rs)

## 5. State Management

All mutable state that needs to be shared across the application is owned and encapsulated within the `DaqManagerActor`. State is modified *only* by the actor, in its own task, in response to incoming messages. This is the core principle of the actor model and provides the following benefits:

- **No Locks**: Eliminates the need for `Mutex` or `RwLock`, avoiding deadlocks and performance bottlenecks from lock contention
- **Sequential Consistency**: Since the actor processes messages one at a time, state modifications are sequential and predictable
- **Clear Ownership**: The "single writer" principle makes the flow of state changes much easier to reason about
- **Data Race Freedom**: Rust's type system guarantees that the actor has exclusive access to its state

### State Ownership by Component

| State | Owner | Access Pattern |
|-------|-------|----------------|
| Active instruments | `DaqManagerActor` | Exclusive (actor mutates) |
| Storage configuration | `DaqManagerActor` | Exclusive (actor mutates) |
| Recording state | `DaqManagerActor` | Exclusive (actor mutates) |
| Instrument data stream | `DataDistributor` | Shared read (broadcast) |
| Metadata | `DaqManagerActor` | Exclusive (actor mutates) |
| GUI state | `GUI` | Exclusive (GUI owns) |

## 6. Data Flow

Measurement data flows through the system in a unidirectional pipeline:

```
Hardware → Instrument Task → Processor Chain → DataDistributor → Subscribers
```

### 6.1. Acquisition Flow

1. **Instrument Task** acquires data from hardware via async stream
2. **Processor Chain** (optional) transforms data:
   - IIR filtering
   - FFT (time → frequency domain)
   - Trigger detection
   - Custom processors
3. **DataDistributor** broadcasts processed measurements
4. **Subscribers** receive data independently:
   - GUI: Real-time plotting
   - Storage Writer: Persist to disk
   - Future: Network streaming, analysis pipelines

### 6.2. Measurement Types

The system supports multiple measurement types via the `Measurement` enum:

- **Scalar**: Traditional numeric measurements (voltage, current, etc.)
- **Spectrum**: FFT/frequency analysis output
- **Image**: 2D camera/sensor data

See [docs/adr/001-measurement-enum-architecture.md](docs/adr/001-measurement-enum-architecture.md) for design rationale.

## 7. Graceful Shutdown

A coordinated shutdown sequence ensures data integrity and releases system resources cleanly.

### 7.1. Shutdown Sequence

1. **GUI** sends `DaqCommand::Shutdown` to actor
2. **Actor** initiates shutdown protocol:
   - Stop recording (if active)
   - Send `InstrumentCommand::Shutdown` to all instruments
3. **Storage Writer**:
   - Receives shutdown signal via oneshot channel
   - Flushes all buffered data to disk
   - Calls `writer.shutdown()`
   - Exits within 5 seconds or is aborted
4. **Instrument Tasks** (for each instrument):
   - Receive `InstrumentCommand::Shutdown` in event loop
   - Break out of `tokio::select!` loop
   - Call `instrument.disconnect()` for cleanup
   - Exit within 5 seconds or is aborted
5. **Actor** waits for all tasks with timeout
6. **Actor** event loop exits
7. **Tokio Runtime** shuts down

### 7.2. Timeout Policy

- **Per-instrument timeout**: 5 seconds
- **Storage writer timeout**: 5 seconds
- **Total shutdown time**: ~5 seconds + (5 seconds × number of instruments)

### 7.3. Fallback: Forceful Abort

If any task exceeds its timeout:
- Task is aborted via `JoinHandle::abort()`
- Warning is logged
- Shutdown continues for remaining tasks
- Ensures application doesn't hang on non-responsive hardware

**Implementation**: See [`src/app_actor.rs`](src/app_actor.rs) (stop_instrument, stop_recording, shutdown methods)

## 8. Design Rationale

### 8.1. Why Actor Pattern Over Arc\<Mutex\<T\>\>?

The previous architecture used `Arc<Mutex<DaqAppInner>>` for shared state management. This had several drawbacks:

| Arc\<Mutex\<T\>\> (Old) | Actor Pattern (New) |
|-------------------------|---------------------|
| Lock contention under load | No locks, sequential processing |
| Potential for deadlocks | Deadlock impossible |
| Complex reasoning about locking order | Clear message flow |
| Blocking GUI on lock acquisition | Non-blocking message passing |
| Hard to add new consumers | Easy to add subscribers |

### 8.2. Why Message-Passing Over Callbacks?

Message-passing provides:
- **Decoupling**: Sender doesn't need to know about receiver implementation
- **Type Safety**: Each message has a well-defined response type
- **Testability**: Easy to mock actors in tests
- **Composability**: Can add logging, tracing, or validation layers

### 8.3. Why Broadcast Channel for Data?

The broadcast channel provides:
- **Multiple Consumers**: GUI and storage both get the same data
- **Independent Progress**: Slow consumer doesn't block fast ones
- **Extensibility**: Easy to add new data consumers
- **Backpressure**: Lagging receivers are pruned automatically

### 8.4. Why Separate Tasks for Each Instrument?

Isolation benefits:
- **Fault Tolerance**: One instrument crash doesn't affect others
- **Independent I/O**: Slow hardware doesn't block fast instruments
- **Parallelism**: Instruments run truly concurrently on multi-core
- **Graceful Degradation**: Can stop/restart individual instruments

## 9. Configuration

System behavior is controlled via TOML configuration:

```toml
[application]
name = "Rust DAQ"
broadcast_channel_capacity = 1024  # DataDistributor capacity
command_channel_capacity = 32      # mpsc channel capacity

[[instruments.my_instrument]]
type = "mock"
[instruments.my_instrument.params]
channel_count = 4

[[processors.my_instrument]]
type = "iir_filter"
[processors.my_instrument.config]
cutoff_hz = 10.0

[storage]
default_format = "csv"
default_path = "./data"
```

See [`config/default.toml`](config/default.toml) for full configuration schema.

## 10. Testing Strategy

The actor architecture simplifies testing:

### 10.1. Unit Tests

- **Actor logic**: Test state transitions by sending commands
- **Message handling**: Verify correct responses for each command variant
- **Shutdown protocol**: Test graceful and forceful shutdown paths

### 10.2. Integration Tests

- **Multi-instrument**: Test concurrent instrument operation
- **Data flow**: Verify measurements reach all subscribers
- **Session persistence**: Test save/load of application state

### 10.3. Mock Instruments

The `MockInstrument` provides deterministic behavior for testing:
- Configurable data rates
- Simulated errors
- Predictable shutdown behavior

See [`tests/integration_test.rs`](tests/integration_test.rs) for examples.

## 11. Performance Characteristics

### 11.1. Throughput

- **Per-instrument**: Limited by hardware and Tokio scheduler
- **Aggregate**: Scales linearly with number of CPU cores
- **DataDistributor**: 1024 message buffer prevents backpressure

### 11.2. Latency

- **Command latency**: 1-2 ms (mpsc send + actor processing)
- **Data latency**: \<1 ms (instrument → GUI via broadcast)
- **Shutdown latency**: 5 seconds per task (configurable timeout)

### 11.3. Memory Usage

- **Per instrument**: ~100 KB (task stack + buffers)
- **DataDistributor**: ~1 MB (1024 × 1 KB per measurement)
- **Total**: Scales linearly with number of instruments

## 12. Future Enhancements

Potential improvements to the architecture:

1. **Supervision Tree**: Implement actor hierarchy with supervision strategy
2. **Remote Actors**: Extend to distributed system with network-transparent actors
3. **Event Sourcing**: Log all commands for replay and debugging
4. **Dynamic Reconfiguration**: Hot-reload configuration without restart
5. **Actor Metrics**: Collect and expose performance metrics per actor

## 13. References

- [Tokio Documentation](https://tokio.rs/)
- [Actor Model (Wikipedia)](https://en.wikipedia.org/wiki/Actor_model)
- [CLAUDE.md](CLAUDE.md) - Project coding guidelines
- [src/app_actor.rs](src/app_actor.rs) - Actor implementation
- [src/messages.rs](src/messages.rs) - Message protocol
- [docs/adr/001-measurement-enum-architecture.md](docs/adr/001-measurement-enum-architecture.md) - Measurement types

## 14. Change History

| Date | Change | Rationale |
|------|--------|-----------|
| 2025-10-19 | Initial actor architecture | Replace Arc\<Mutex\<T\>\> with message-passing |
| 2025-10-19 | Add graceful shutdown | Ensure clean resource cleanup |
| 2025-10-19 | Document architecture | Improve maintainability |
