// Multi-Angle Data Acquisition Script
// ===========================================
// Acquires data at multiple stage positions while
// rotating a polarization element.
//
// Equipment:
// - MockStage (or ESP300/Newport stage)
// - MockCamera (or PVCAM camera)
// - Elliptec ELL14 rotator (optional)
// - Newport 1830-C power meter (optional)
//
// This is a template for complex multi-dimensional scans.
// ===========================================

// Configuration - Mock hardware for testing
let USE_MOCK = true;

// Scan parameters
let STAGE_POSITIONS = [0.0, 5.0, 10.0, 15.0, 20.0];  // mm
let ROTATOR_ANGLES = [0.0, 45.0, 90.0, 135.0];       // degrees
let FRAMES_PER_POINT = 3;

print("==========================================");
print("  MULTI-ANGLE DATA ACQUISITION");
print("  Stage positions: " + STAGE_POSITIONS.len());
print("  Rotator angles: " + ROTATOR_ANGLES.len());
print("  Frames per point: " + FRAMES_PER_POINT);
print("  Total frames: " + STAGE_POSITIONS.len() * ROTATOR_ANGLES.len() * FRAMES_PER_POINT);
print("==========================================");
print("");

// Initialize instruments
print("[1/4] Initializing hardware...");

let stage;
let camera;
let rotator;
let power_meter;

if USE_MOCK {
    stage = create_mock_stage();
    print("  MockStage: Created");

    camera = create_mock_camera(1920, 1080);
    print("  MockCamera (1920x1080): Created");

    // Mock power meter
    power_meter = create_mock_power_meter(2.5);
    print("  MockPowerMeter: Created");
} else {
    // Real hardware configuration
    stage = create_esp300("/dev/ttyUSB1", 1);
    print("  ESP300 Stage: Connected");

    camera = create_pvcam("Primary BSI");
    print("  PVCAM Camera: Connected");

    rotator = create_elliptec("/dev/ttyUSB0", "2");
    print("  ELL14 Rotator: Connected");

    power_meter = create_newport_1830c("/dev/ttyS0");
    print("  Newport 1830-C: Connected");
}

// Home stage
print("");
print("[2/4] Homing stage...");
stage.move_abs(0.0);
sleep(1.0);
print("  Stage at home position");

// Arm camera
print("");
print("[3/4] Arming camera...");
camera.arm();
print("  Camera armed");

// Run acquisition
print("");
print("[4/4] Starting multi-angle acquisition...");
print("");
print("  Position | Angle | Frame | Power (W)");
print("  ---------|-------|-------|----------");

let all_data = [];
let total_frames = 0;

for pos in STAGE_POSITIONS {
    // Move stage to position
    stage.move_abs(pos);
    sleep(0.2);  // settling

    for angle in ROTATOR_ANGLES {
        // Move rotator (if available)
        if rotator != () {
            rotator.move_abs(angle);
            sleep(0.2);
        }

        // Acquire frames
        for frame_idx in 0..FRAMES_PER_POINT {
            // Read power
            let power = power_meter.read();

            // Trigger camera
            camera.trigger();
            total_frames += 1;

            // Record data point
            let data_point = #{
                stage_mm: pos,
                angle_deg: angle,
                frame: frame_idx + 1,
                power_w: power,
                total_frame: total_frames
            };
            all_data.push(data_point);

            // Format output
            let pos_str = if pos < 10.0 { "   " + pos } else { "  " + pos };
            let angle_str = if angle < 100.0 { "  " + angle } else { " " + angle };

            print(pos_str + "    |" + angle_str + "  |   " + (frame_idx + 1) + "   | " + power);
        }
    }
}

// Return to home
print("");
print("[CLEANUP] Returning to home...");
stage.move_abs(0.0);
sleep(0.5);

print("");
print("==========================================");
print("  ACQUISITION COMPLETE");
print("  Total frames: " + total_frames);
print("  Data points: " + all_data.len());
print("==========================================");

// Return collected data
all_data
