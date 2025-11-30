// Orchestrated Experiment Example (bd-73yh.5)
//
// Demonstrates the Bluesky-inspired RunEngine experiment orchestration.
// Uses declarative plans instead of imperative loops.
//
// Key concepts:
// - Plans: Declarative generators that describe what should happen
// - RunEngine: Executes plans, handles pause/resume, emits documents
// - Documents: Structured data stream (Start, Descriptor, Event, Stop)

// =============================================================================
// Example 1: Simple Line Scan
// =============================================================================

print("=== Example 1: Line Scan ===");

// Create a 1D scan: move stage_x from 0 to 10mm in 11 steps
// Read power_meter at each position
let scan = line_scan("stage_x", 0.0, 10.0, 11, "power_meter");

print(`Created plan: ${plan_type(scan)}`);
print(`Total points: ${num_points(scan)}`);

// Queue the plan - returns a unique run ID
let run_id = run_engine.queue(scan);
print(`Queued run: ${run_id}`);

// Start execution (non-blocking, runs in background)
run_engine.start();
print(`State: ${run_engine.state()}`);

// Wait for completion (in real use, you'd subscribe to documents)
while run_engine.state() != "Idle" {
    let progress = run_engine.current_progress();
    if progress >= 0 {
        print(`Progress: ${progress}/${num_points(scan)}`);
    }
    sleep(0.1);
}

print("Line scan complete!\n");

// =============================================================================
// Example 2: Grid Scan with Metadata
// =============================================================================

print("=== Example 2: Grid Scan with Metadata ===");

// Create a 2D grid scan: 5x3 = 15 points
let grid = grid_scan(
    "stage_x", 0.0, 4.0, 5,   // X: 0, 1, 2, 3, 4 mm
    "stage_y", 0.0, 2.0, 3,   // Y: 0, 1, 2 mm
    "camera"                   // Detector
);

print(`Grid scan: ${num_points(grid)} points`);

// Add experimental metadata
let metadata = #{
    sample: "GaAs wafer #42",
    operator: "Alice",
    notes: "Initial characterization scan"
};

let grid_run = run_engine.queue_with_metadata(grid, metadata);
print(`Queued grid scan: ${grid_run}`);

// Start and wait
run_engine.start();
while run_engine.state() != "Idle" {
    sleep(0.1);
}

print("Grid scan complete!\n");

// =============================================================================
// Example 3: Count (Repeated Measurements)
// =============================================================================

print("=== Example 3: Count Measurements ===");

// Take 10 readings at current position with 0.5s delay between each
let cnt = count(10, "power_meter", 0.5);

print(`Count plan: ${num_points(cnt)} measurements`);

run_engine.queue(cnt);
run_engine.start();

while run_engine.state() != "Idle" {
    sleep(0.1);
}

print("Count complete!\n");

// =============================================================================
// Example 4: Pause/Resume Demonstration
// =============================================================================

print("=== Example 4: Pause/Resume ===");

// Create a longer scan
let long_scan = line_scan("stage_x", 0.0, 100.0, 101, "power_meter");
run_engine.queue(long_scan);
run_engine.start();

// Let it run for a bit
sleep(0.5);

// Pause at checkpoint
print("Pausing scan...");
run_engine.pause();
print(`State after pause: ${run_engine.state()}`);

// Do something while paused (e.g., adjust sample)
sleep(0.5);

// Resume
print("Resuming scan...");
run_engine.resume();
print(`State after resume: ${run_engine.state()}`);

// Wait for completion
while run_engine.state() != "Idle" {
    sleep(0.1);
}

print("Pause/resume demo complete!\n");

// =============================================================================
// Summary
// =============================================================================

print("=== All Examples Complete ===");
print("");
print("The RunEngine orchestration provides:");
print("  - Declarative plans (line_scan, grid_scan, count)");
print("  - Structured document stream (Start, Descriptor, Event, Stop)");
print("  - Pause/resume at checkpoints");
print("  - Metadata attachment for experiment tracking");
print("  - Decoupled data acquisition from storage/visualization");
