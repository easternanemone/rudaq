# daq-experiment

Bluesky-inspired experiment orchestration for rust-daq.

## Overview

This crate provides the experiment execution layer, implementing patterns from
[Bluesky](https://blueskyproject.io/) for structured scientific data acquisition:

- **Plans**: Declarative experiment sequences that yield commands
- **RunEngine**: State machine that executes plans and emits documents
- **Document Model**: Structured metadata (Start, Descriptor, Event, Stop)

## Key Concepts

### Plans

Plans are declarative sequences that describe *what* to do, not *how*:

```rust,ignore
use daq_experiment::plans::{GridScan, LineScan, Count};

// Simple count: take 10 readings
let plan = Count::new(detector.clone(), 10);

// Line scan: move stage, read at each point
let plan = LineScan::new(
    stage.clone(),    // Movable device
    -10.0, 10.0,      // Start, end positions
    21,               // Number of points
    detector.clone(), // Readable device
);

// Grid scan: 2D raster pattern
let plan = GridScan::new(
    stage_x.clone(), -10.0, 10.0, 11,  // X axis
    stage_y.clone(), -5.0, 5.0, 6,     // Y axis
    detector.clone(),
);
```

### RunEngine

The RunEngine executes plans and manages state:

```rust,ignore
use daq_experiment::run_engine::RunEngine;

let engine = RunEngine::new(registry);

// Queue a plan
let run_id = engine.queue(plan).await?;

// Execute (emits documents as it runs)
engine.run().await?;

// Or with pause/resume support
engine.pause().await?;  // Pauses at next checkpoint
engine.resume().await?;
engine.abort().await?;  // Emergency stop
```

### Document Model

All runs emit structured documents following the Bluesky pattern:

```
StartDoc        → Run metadata, scan_id, plan_name
  │
  ├─ DescriptorDoc → Data stream description (what fields, types)
  │
  ├─ EventDoc      → Actual data points (seq_num, readings, timestamps)
  │   (repeated)
  │
  └─ StopDoc       → Exit status, reason, timing summary
```

```rust,ignore
// Subscribe to documents
let mut rx = engine.subscribe();
tokio::spawn(async move {
    while let Ok(doc) = rx.recv().await {
        match doc {
            Document::Start(s) => println!("Run {} started", s.uid),
            Document::Event(e) => println!("Point {}: {:?}", e.seq_num, e.data),
            Document::Stop(s) => println!("Run finished: {}", s.exit_status),
            _ => {}
        }
    }
});
```

## State Machine

The RunEngine follows a well-defined state machine:

```
                    queue()
     ┌──────────────────────────────────────┐
     │                                      ▼
   IDLE ──run()──▶ RUNNING ──complete──▶ IDLE
                      │
                   pause()
                      ▼
                   PAUSED ──resume()──▶ RUNNING
                      │
                   abort()
                      ▼
                  ABORTING ──cleanup──▶ IDLE
```

## Plan Commands

Plans yield these commands to the RunEngine:

| Command | Description |
|---------|-------------|
| `MoveTo` | Move a stage to absolute position |
| `Read` | Read from a detector |
| `Trigger` | Trigger a device (camera arm) |
| `Wait` | Wait for duration or condition |
| `Checkpoint` | Safe pause point |
| `EmitEvent` | Emit an event document |
| `CreateDescriptor` | Define a new data stream |

## Built-in Plans

| Plan | Description |
|------|-------------|
| `Count` | Take N readings from a detector |
| `LineScan` | 1D scan along a single axis |
| `GridScan` | 2D raster scan |
| `VoltageScan` | Scan analog output voltage |
| `TimeSeries` | Periodic readings over time |
| `TriggeredAcquisition` | Hardware-triggered frame capture |

## Custom Plans

Implement the `Plan` trait for custom sequences:

```rust,ignore
use daq_experiment::plans::{Plan, PlanCommand, PlanContext};

struct MyCustomPlan {
    // plan state
}

impl Plan for MyCustomPlan {
    fn name(&self) -> &str { "my_custom_plan" }
    
    async fn run(
        &mut self,
        ctx: &mut PlanContext,
    ) -> Result<(), anyhow::Error> {
        // Emit start
        ctx.emit_start()?;
        
        // Do your sequence
        ctx.move_to("stage", 0.0).await?;
        ctx.checkpoint()?;  // Safe pause point
        ctx.read("detector").await?;
        
        // Emit stop
        ctx.emit_stop("success")?;
        Ok(())
    }
}
```

## Integration with Scripting

Plans can be created from Rhai scripts:

```rhai
// my_experiment.rhai
let stage = get_stage("sample_stage");
let camera = get_camera("main_camera");

// Create and queue a grid scan
let plan = grid_scan(stage, -100.0, 100.0, 21, camera);
queue_plan(plan);

// Or build a custom sequence
let plan = plan_builder()
    .move_to(stage, 0.0)
    .trigger(camera)
    .read(camera)
    .build();
queue_plan(plan);
```

## Example

Complete acquisition workflow:

```rust,ignore
use daq_experiment::run_engine::RunEngine;
use daq_experiment::plans::GridScan;
use daq_hardware::DeviceRegistry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Setup
    let registry = DeviceRegistry::new();
    // ... register devices ...
    
    let engine = RunEngine::new(Arc::new(registry));
    
    // Create plan
    let stage_x = registry.get_movable("stage_x").unwrap();
    let stage_y = registry.get_movable("stage_y").unwrap();
    let camera = registry.get_frame_producer("camera").unwrap();
    
    let plan = GridScan::new(
        stage_x, -50.0, 50.0, 11,
        stage_y, -50.0, 50.0, 11,
        camera,
    );
    
    // Execute
    engine.queue(plan).await?;
    engine.run().await?;
    
    Ok(())
}
```

## See Also

- [`common`](../common/) - Capability traits and frame types
- [`daq-hardware`](../daq-hardware/) - Device registry and drivers
- [`daq-scripting`](../daq-scripting/) - Rhai scripting integration
- [`daq-storage`](../daq-storage/) - Data persistence

## References

- [Bluesky Documentation](https://blueskyproject.io/bluesky/)
- [Event Model Specification](https://blueskyproject.io/event-model/)
