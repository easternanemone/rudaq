# Dover Motion Driver Development Guide

## Overview

Dover Motion Synergy API for SmartStage precision motion control stages (SmartStage XY, Linear, DOF-5, DMCM).

**Reference:** `docs/reference/markdown/dover-motion-api-manual.md`

---

## Communication Interfaces

### Serial (RS232/RS485)

| Parameter | Options | Default |
|-----------|---------|---------|
| Protocol | `Point2Point` (RS232), `MultiDropUsingIdleLineDetection` (RS485) | Point2Point |
| Baud Rate | 9600, 19200, 38400, 57600, 115200 | 115200 |
| Parity | NOPARITY, ODDPARITY, EVENPARITY, MARKPARITY, SPACEPARITY | NOPARITY |
| Stop Bits | ONESTOPBIT, ONE5STOPBITS, TWOSTOPBITS | ONESTOPBIT |
| Flow Control | FLOWCONTROL_NONE | NONE |
| Timeout | 1-10000 ms | 1000 |

**Node IDs:** 1-31 (RS485), 1-127 (CAN). **Never use 0** (broadcast).

### CAN

| Parameter | Options |
|-----------|---------|
| Baud Rate Index | 0-7 (see table below) |
| Timeout | 1-10000 ms |

**CAN Baud Rate Index:**
| Index | Rate |
|-------|------|
| 0 | 1 Mbit/s |
| 1 | 800 kbit/s |
| 2 | 500 kbit/s |
| 3 | 250 kbit/s |
| 4 | 125 kbit/s |
| 5 | 50 kbit/s |
| 6 | 20 kbit/s |
| 7 | 10 kbit/s |

---

## Complete IAxis Interface

### Motion Commands

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `MoveAbsolute` | `position: double` | `Task<InstrumentResult>` | Move to absolute position |
| `MoveRelative` | `distance: double` | `Task<InstrumentResult>` | Move by relative distance |
| `MoveRelativeToFlag` | `maxDistance: double, flagName: string` | `Task<InstrumentResult>` | Move until flag triggered |
| `MoveRelativeUDP` | `repeats: int` | `Task<InstrumentResult>` | User-defined profile move |
| `MoveVelocity` | `direction: bool` | `Task<InstrumentResult>` | Continuous velocity move |
| `Stop` | - | `Task<InstrumentResult>` | Smooth stop with deceleration |
| `EStop` | - | `Task<InstrumentResult>` | Emergency stop (requires re-init) |
| `Wait` | - | `Task<InstrumentResult>` | Wait for move completion |

### Named Motion Commands (Config-Driven)

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `NMoveAbsolute` | `positionName: string` | `Task<InstrumentResult>` | Move to named position |
| `NMoveRelative` | `distanceName: string` | `Task<InstrumentResult>` | Move by named distance |
| `NMoveRelativeToFlag` | `maxDistanceName: string, flagName: string` | `Task<InstrumentResult>` | Named flag-triggered move |

### Motor Control

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `Enable` | - | `Task<InstrumentResult>` | Energize motor |
| `Disable` | - | `Task<InstrumentResult>` | De-energize motor |
| `GetIsEnabled` | `forceRefresh: bool` | `Task<InstrumentResult<bool>>` | Query motor state |

### Position Control

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `GetPosition` | `forceRefresh: bool` | `Task<InstrumentResult<double>>` | Get current position |
| `GetCommandedPosition` | - | `Task<InstrumentResult<double>>` | Get commanded position |
| `ResetPosition` | - | `Task<InstrumentResult>` | Reset commanded to actual |

### Parameter Control

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `SetVelocity` | `velocity: double` | `Task<InstrumentResult>` | Set move velocity |
| `GetVelocity` | `forceRefresh: bool` | `Task<InstrumentResult<double>>` | Query velocity |
| `SetAcceleration` | `acceleration: double` | `Task<InstrumentResult>` | Set acceleration |
| `GetAcceleration` | `forceRefresh: bool` | `Task<InstrumentResult<double>>` | Query acceleration |
| `SelectMoveProfile` | `profileName: string` | `Task<InstrumentResult>` | Load profile from config |

### Trigger on Position (TOP)

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `EnableTriggerOnPosition` | `startPos, endPos, increment: double` | `Task<InstrumentResult>` | Enable GPIO triggers |
| `NEnableTriggerOnPosition` | `startPosName, endPosName, incrementName: string` | `Task<InstrumentResult>` | Named variant |
| `DisableTriggerOnPosition` | - | `Task<InstrumentResult>` | Disable GPIO triggers |
| `GetIsTriggerOnPositionEnabled` | - | `Task<InstrumentResult<bool>>` | Query TOP state |

### Trace Capture (Diagnostics)

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `EnableTraceCapture` | `trigger: TraceTrigger, profileName: string` | `Task<InstrumentResult>` | Start trace capture |
| `NEnableTraceCapture` | `trigger: TraceTrigger, profileName: string` | `Task<InstrumentResult>` | Named variant |
| `GetIsTraceCaptureEnabled` | - | `Task<InstrumentResult<bool>>` | Query trace state |
| `StopTraceCapture` | - | `Task<InstrumentResult>` | Stop capture |
| `SaveTraceCapture` | `filename: string` | `Task<InstrumentResult>` | Export as CSV |

### Flag Operations

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `GetFlagStatus` | `flagName: string` | `Task<InstrumentResult<bool>>` | Check flag active |
| `GetLatchedFlagPosition` | `flagName: string` | `Task<InstrumentResult<double>>` | Position when triggered |

### Recovery

| Method | Parameters | Return | Description |
|--------|------------|--------|-------------|
| `GetRecoveryAction` | - | `Task<InstrumentResult<RecoveryAction>>` | Get required recovery |

---

## Complete AlertCode Enumeration

### Success

| Code | Severity | Description |
|------|----------|-------------|
| `Success` | Low | Command completed successfully |

### Axis Device Alerts

| Code | Severity | Description | Recovery |
|------|----------|-------------|----------|
| `AxisDeviceAlertInvalidConfiguration` | High | Invalid axis config | Check config file |
| `AxisDeviceAlertUnsupportedModelType` | High | Unsupported model | Check hardware |
| `AxisDeviceAlertAlreadyConfigured` | High | Double configuration | Restart |
| `AxisDeviceAlertPositionError` | High | Position error | Reset + Enable |
| `AxisDeviceAlertGeneralError` | High | General error | Re-init |
| `AxisDeviceAlertSettingError` | High | Bad setting | Fix setting |
| `AxisDeviceAlertBump` | High | Collision detected | Check hardware |
| `AxisDeviceAlertHalt` | High | Axis halted | Re-enable |
| `AxisDeviceAlertEstop` | High | E-stop triggered | Re-initialize |
| `AxisDeviceAlertUninitialized` | High | Not initialized | Initialize |
| `AxisDeviceAlertBusy` | High | Axis busy | Wait |
| `AxisDeviceAlertOutOfRange` | High | Value out of range | Check limits |
| `AxisDeviceAlertCallInhibited` | High | Call inhibited | Check safety |
| `AxisDeviceAlertOverTemperature` | High | Motor overheated | Cool down |
| `AxisDeviceAlertCurrentFoldback` | High | Current foldback | Reduce load |
| `AxisDeviceAlertOverVoltage` | High | Voltage too high | Check power |
| `AxisDeviceAlertUnderVoltage` | High | Voltage too low | Check power |
| `AxisDeviceAlertCommunicationsFailure` | High | Comm lost | Check cable |

### System Alerts

| Code | Severity | Description | Recovery |
|------|----------|-------------|----------|
| `StdException` | High | C++ std::exception | Re-init |
| `BoostException` | High | Boost exception | Re-init |
| `UnknownException` | SafetyCritical | Unknown exception | Restart |
| `ManagedException` | High | Managed exception | Re-init |
| `ExecuteScriptError` | High | Script failed | Fix script |
| `PropertyErrorHigh` | High | Property read error | Re-init |
| `PropertyErrorSafetyCritical` | SafetyCritical | Critical property error | Restart |
| `FailedToInitialize` | High | Init failed | Check hardware |
| `ScriptNotFoundError` | Medium | Script missing | Check path |
| `ScriptAborted` | Medium | Script aborted | Retry |

---

## Complete Recovery Actions

| Action | Trigger | Steps |
|--------|---------|-------|
| `NoneRequired` | Ready state | Continue |
| `ResetPositionAndEnable` | Position error | `ResetPosition()` → `Enable()` |
| `ResetPosition` | Position sync needed | `ResetPosition()` |
| `EnableAxis` | Motor disabled | `Enable()` |
| `Reinitialize` | Fatal error | Full re-init |

---

## Complete Configuration Settings

### [Instrument] Section

| Setting | Type | Description |
|---------|------|-------------|
| `LinearUnits` | Enum | Millimetres, Microns, Nanometres |
| `ConvertRelativePathsToAbsolute` | bool | Path conversion |
| `ApplicationVersionString` | string | App version |
| `ConfigurationFilename` | path | Config file path |
| `ProgramDataFolder` | path | Data directory |
| `SupportFolder` | path | Support files |

### [SerialComms] Section

| Setting | Type | Values |
|---------|------|--------|
| `SerialAddress` | int | 0-255 |
| `SerialBaudRate` | enum | 9600, 19200, 38400, 57600, 115200 |
| `SerialProtocol` | enum | Point2Point, MultiDrop |
| `SerialTimeout` | int | ms |

### [CANComms] Section

| Setting | Type | Values |
|---------|------|--------|
| `CanAddress` | int | 1-127 |
| `CanBaudRate` | int | 0-7 (index) |
| `CanTimeout` | int | ms |

### [XAxisNamedPositions] Section

```toml
[XAxisNamedPositions]
Home = 0.0
PositiveLimit = 100.0
NegativeLimit = -100.0
Center = 50.0
Load = 25.0
Unload = 75.0
```

### [XAxisNamedDistances] Section

```toml
[XAxisNamedDistances]
SmallStep = 0.1
MediumStep = 1.0
LargeStep = 10.0
JogStep = 0.01
```

### [XAxisNamedFlags] Section

```toml
[XAxisNamedFlags]
HomeSwitch = HomeFlag
PositiveLimit = LimitFlagPositive
NegativeLimit = LimitFlagNegative
EncoderIndex = IndexFlag
```

### [XAxisMoveProfile_*] Sections

```toml
[XAxisMoveProfile_Default]
Velocity = 10.0           # units/sec
Acceleration = 200.0      # units/sec²
Deceleration = 200.0      # units/sec²
Jerk = 0.0               # units/sec³

[XAxisMoveProfile_Fast]
Velocity = 50.0
Acceleration = 500.0
Deceleration = 500.0

[XAxisMoveProfile_Slow]
Velocity = 1.0
Acceleration = 50.0
Deceleration = 50.0
```

### [XAxisTraceProfile_*] Sections

```toml
[XAxisTraceProfile_MoveTrace]
Variable1 = CommandedVelocity
Variable2 = ActualVelocity
Variable3 = PositionError
Variable4 = CommandedAcceleration
```

---

## Complete Trace Variables

### Current Loop Variables

| Variable | Description |
|----------|-------------|
| `PhaseAReference` | Phase A reference |
| `PhaseAError` | Phase A error |
| `PhaseAActualCurrent` | Phase A measured current |
| `PhaseAIntegratorSum` | Phase A integrator |
| `PhaseAIntegratorContribution` | Phase A integrator contribution |
| `CurrentLoopAOutput` | Phase A output |
| `PhaseBReference` | Phase B reference |
| `PhaseBError` | Phase B error |
| `PhaseBActualCurrent` | Phase B measured current |
| `PhaseBIntegratorSum` | Phase B integrator |
| `PhaseBIntegratorContribution` | Phase B integrator contribution |
| `CurrentLoopBOutput` | Phase B output |

### Field Oriented Control (FOC) Variables

| Variable | Description |
|----------|-------------|
| `FOCDReference` | D-axis reference |
| `FOCDError` | D-axis error |
| `FOCQReference` | Q-axis reference |
| `FOCQError` | Q-axis error |
| `FOCDOutput` | D-axis output |
| `FOCQOutput` | Q-axis output |

### Position Loop Variables

| Variable | Description |
|----------|-------------|
| `CommandedPosition` | Target position |
| `CommandedVelocity` | Target velocity |
| `CommandedAcceleration` | Target acceleration |
| `ActualPosition` | Measured position |
| `ActualVelocity` | Measured velocity |
| `PositionError` | Position error |
| `PositionIntegrator` | Position integrator |
| `PositionDerivative` | Position derivative |
| `PositionOutput` | Position loop output |

### System Variables

| Variable | Description |
|----------|-------------|
| `MotorCurrent` | Total motor current |
| `MotorTemperature` | Motor temperature |
| `BusVoltage` | Bus voltage |
| `BusCurrent` | Bus current |

---

## Position Loop Tuning Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `Kp` | int | Proportional gain |
| `Ki` | int | Integral gain |
| `Kd` | int | Derivative gain |
| `Kaff` | int | Acceleration feedforward |
| `Kvff` | int | Velocity feedforward |
| `Kout` | int | Output scaling |
| `DerivativeTime` | int | Derivative time constant |
| `Ilimit` | int | Current limit |

---

## Current Loop Tuning Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `KpCurrent` | UINT16 | Proportional gain |
| `KiCurrent` | UINT16 | Integral gain |
| `IlimitCurrent` | UINT16 | Current limit |

---

## Biquad Filter Settings

| Parameter | Type | Values |
|-----------|------|--------|
| `FilterType` | enum | None, LowPass, HighPass, BandPass, BandStop |
| `Frequency` | double | Center frequency (Hz) |
| `QualityFactor` | double | Filter Q |
| `K` | int | Gain coefficient |
| `A1`, `A2` | int | Feedback coefficients |
| `B0`, `B1`, `B2` | int | Feedforward coefficients |

---

## Motion Complete Settings

| Parameter | Type | Description |
|-----------|------|-------------|
| `Mode` | enum | How completion is detected |
| `PositionErrorLimit` | double | Acceptable position error |
| `SettleTimeMs` | double | Time after reaching position |
| `SettleWindow` | double | Position tolerance window |

---

## State Machines

### Connection States

```
Disconnected → Connecting → Connected
     ↑                          ↓
     └──────────────────────────┘
```

### Instrument States

```
Uninitialized → Ready ←→ Running
                  ↓
                Error → (requires re-init or restart)
                  ↓
            FatalError → (requires restart)
```

---

## Configuration Example

```toml
[[devices]]
id = "x_axis"
type = "dover_motion"
enabled = true

[devices.config]
port = "/dev/ttyUSB0"
baudrate = 115200
protocol = "Point2Point"
node_id = 1
timeout_ms = 1000

[devices.config.named_positions]
home = 0.0
center = 50.0
max = 100.0

[devices.config.move_profile]
velocity = 10.0
acceleration = 200.0
deceleration = 200.0
```
