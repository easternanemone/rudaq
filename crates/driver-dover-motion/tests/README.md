# Dover Motion Driver Tests

## Test Suite

### Mock Mode Tests (Always Run)

These tests run without hardware and verify basic driver functionality:

- `mock_driver_initialization` - Driver creation with mock hardware
- `mock_basic_motion` - Absolute/relative moves, position query, stop
- `mock_trigger_on_position` - TOP enable/disable/query
- `test_invalid_top_parameters` - Error handling for invalid parameters

Run with:
```bash
cargo nextest run -p driver-dover-motion
```

### Hardware Tests (Gated by Environment Variable)

These tests require real Dover Motion hardware:

- `hardware_device_connection` - Connect to device, read position
- `hardware_small_move` - Execute 0.1mm move and verify accuracy

Run with:
```bash
export DOVER_MOTION_SMOKE_TEST=1
export DOVER_CONFIG_PATH="C:\\ProgramData\\Dover Motion\\SmartStage.xml"
export DOVER_AXIS_NAME="X"
cargo nextest run --profile hardware --features hardware -p driver-dover-motion
```

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DOVER_MOTION_SMOKE_TEST` | Yes (hardware) | - | Set to `1` to enable hardware tests |
| `DOVER_CONFIG_PATH` | No | `mock://smartstage` | Path to Dover Motion XML config |
| `DOVER_AXIS_NAME` | No | `X` | Axis name to test |

## Hardware Requirements

- Dover Motion SmartStage (or compatible)
- Windows with Dover Motion SDK installed
- USB or Ethernet connection configured

## Test Coverage

- ✓ Driver initialization
- ✓ Parameter access
- ✓ Absolute motion
- ✓ Relative motion
- ✓ Position query
- ✓ Motion stop
- ✓ Wait for settle
- ✓ Trigger-On-Position (TOP) enable/disable
- ✓ TOP state query
- ✓ Error handling for invalid TOP parameters
