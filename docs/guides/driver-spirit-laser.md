# SPIRIT 1040 Laser Driver Development Guide

## Overview

Coherent SPIRIT 1040 Ti:Sapphire femtosecond laser with CANopen and TCP/IP control interfaces.

**Reference:** `docs/reference/markdown/spirit-laser.md`

---

## Communication Interfaces

### TCP/IP Interface (Recommended)

| Parameter | Value |
|-----------|-------|
| Port | 9000 |
| Protocol | Text-based, case-sensitive |
| Terminators | `\n`, `\r`, `\r\n` accepted |

**Query Format:**
```
Request:  PARAMETER_NAME\n
Response: OK VALUE PARAMETER_NAME\r\n
Error:    ERROR ERROR_NUMBER PARAMETER_NAME\r\n
```

**Control Format:**
```
Request:  PARAMETER_NAME=VALUE\n
Response: OK PARAMETER_NAME\r\n
Error:    ERROR ERROR_NUMBER PARAMETER_NAME\r\n
```

### CANopen Interface

| Parameter | Value |
|-----------|-------|
| Node ID | 0x0E (fixed) |
| Default Baud | 125 kBit/s |
| RxPDO1 | 0x200 + 0x0E = 0x20E |
| TxPDO1 | 0x180 + 0x0E = 0x18E |
| SDO Master | 0x600 + 0x0E = 0x60E |
| SDO Slave | 0x580 + 0x0E = 0x58E |

---

## Complete State Machine

### State Values

| State | Name | Description |
|-------|------|-------------|
| 0 | Initializing | Power-on self-test |
| 1 | No Operation | Ready but idle |
| 2 | Warming Up | Thermal stabilization (~30 min from cold) |
| 3 | Init Laser Off | Transitioning to off |
| 4 | Laser Off | Off state, safe for service |
| 5 | Init Laser Standby | Transitioning to standby |
| 6 | Laser Standby | Warm, diode current on, HV off |
| 7 | Init Laser On | Transitioning to on |
| 8 | Laser On | Fully operational, emission enabled |
| 9 | Manual Mode | Manual override active |
| 10 | Init Error | Error condition detected |
| 11 | Error | Locked in error state |

### State Transitions

```
Control = 0 (Off):     Any → 3 → 4
Control = 1 (Standby): Any → 2 → 5 → 6
Control = 2 (On):      6 → 7 → 8
Error:                 Any → 10 → 11
Recovery:              11 → (send control command) → target state
```

---

## Complete TCP Parameter List

### System/Common Parameters

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `state_control` | UINT8 | 0-2 | RW | 0=Off, 1=Standby, 2=On |
| `state_actual` | UINT8 | 0-11 | RO | Current state (see state machine) |
| `common_warnings` | UINT16 | bitmask | RO | Warning flags |
| `common_errors` | UINT16 | bitmask | RO | Error flags |
| `master_off_shutdown_time` | UINT16 | 0-60 s | RW | Auto-shutdown timeout (0=disabled) |
| `runtime` | UINT32 | minutes | RO | Laser diode runtime |
| `onoff_statistic` | UINT32 | count | RO | On/off cycle count |

### Temperature Control Parameters

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `temperature_control_warnings` | UINT16 | bitmask | RO | Chiller warnings |
| `temperature_control_errors` | UINT16 | bitmask | RO | Chiller errors |
| `chiller_current_output` | Float | A | RO | Chiller current |
| `chiller_flow_rate` | Float | L/min | RO | Coolant flow rate |
| `chiller_temperature` | Float | °C | RO | Chiller temperature |

### Seeder/Oscillator Parameters

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `seeder_control` | UINT8 | 0-2 | RW | Seeder control state |
| `seeder_state_actual` | UINT8 | 0-11 | RO | Seeder state |
| `seeder_current_diode` | Float | mA | RO | Diode current |
| `seeder_voltage_diode` | Float | V | RO | Diode voltage |
| `seeder_temperature_diode` | Float | °C | RO | Diode temperature |
| `seeder_temperature_crystal` | Float | °C | RO | Crystal temperature |
| `seeder_warnings` | UINT16 | bitmask | RO | Seeder warnings |
| `seeder_errors` | UINT16 | bitmask | RO | Seeder errors |

### Amplifier Parameters

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `amplifier_control` | UINT8 | 0-2 | RW | Amplifier control state |
| `amplifier_state_actual` | UINT8 | 0-11 | RO | Amplifier state |
| `amplifier_current_diode` | Float | mA | RO | Diode current |
| `amplifier_voltage_diode` | Float | V | RO | Diode voltage |
| `amplifier_temperature_diode` | Float | °C | RO | Diode temperature |
| `amplifier_temperature_crystal` | Float | °C | RO | Crystal temperature |
| `amplifier_warnings` | UINT16 | bitmask | RO | Amplifier warnings |
| `amplifier_errors` | UINT16 | bitmask | RO | Amplifier errors |

### Switching Unit / PDG Parameters

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `switching_unit_warnings` | UINT16 | bitmask | RO | PDG warnings |
| `switching_unit_errors` | UINT16 | bitmask | RO | PDG errors |
| `pdg_gate_voltage_actual` | Float | V | RO | PDG gate voltage |
| `pdg_seed_level_actual` | Float | V | RO | PDG seed level |
| `pulse_picker_divider` | UINT32 | 0-10000 | RW | Pulse picker integer divider |

### Power Control Parameters

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `power_control_warnings` | UINT16 | bitmask | RO | Power control warnings |
| `power_control_errors` | UINT16 | bitmask | RO | Power control errors |
| `power_setpoint_percentage` | UINT8 | 0-100 | RW | Attenuation % (0=min atten) |
| `power_setpoint` | Float | W | RW | Absolute power setpoint |
| `power_actual` | Float | W | RO | Measured output power |
| `shutter_control` | UINT8 | 0-1 | RW | 0=Closed, 1=Open |
| `shutter_state` | UINT8 | 0-1 | RO | Actual shutter state |
| `shutter_relay_state` | UINT8 | 0-1 | RO | Safety relay state |

### Controller Parameters

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `controller_warnings` | UINT32 | bitmask | RO | Controller warnings |
| `controller_errors` | UINT32 | bitmask | RO | Controller errors |
| `supply_voltage_24v` | Float | V | RO | 24V supply voltage |
| `electronics_temperature` | Float | °C | RO | Electronics temperature |

### SHG Parameters (Optional - Green Wavelength Models)

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `shg_warnings` | UINT16 | bitmask | RO | SHG warnings |
| `shg_errors` | UINT16 | bitmask | RO | SHG errors |
| `wavelength_setpoint` | UINT8 | 0-1 | RW | 0=IR (1040nm), 1=Green (520nm) |
| `shg_crystal_temperature` | Float | °C | RO | SHG crystal temperature |

### Repetition Rate Parameters

| Parameter | Type | Range | R/W | Description |
|-----------|------|-------|-----|-------------|
| `repetition_rate` | UINT8 | 0-4 | RW | Rate selector (model dependent) |
| `repetition_rate_actual` | UINT16 | kHz | RO | Actual rep rate |

---

## Complete CANopen Object Dictionary

### System Objects

| Index | Subindex | Name | Type | Min | Max | PDO/SDO | R/W |
|-------|----------|------|------|-----|-----|---------|-----|
| 0x2000 | 0 | Control | UINT8 | 0 | 2 | PDO | RW |
| 0x2001 | 0 | Status | UINT8 | 0 | 11 | PDO | RO |
| 0x2002 | 0 | Runtime | UINT32 | - | - | SDO | RO |
| 0x2003 | 0 | OnOff Statistic | UINT32 | - | - | SDO | RO |
| 0x2004 | 0 | Baudrate | UINT8 | 0 | 7 | SDO | RW |
| 0x2005 | 0 | Master Off Timeout | UINT16 | 0 | 60 | SDO | RW |
| 0x2006 | 0 | Article Number | UINT32 | - | - | SDO | RO |

**Baudrate Values:**
| Value | Rate |
|-------|------|
| 0 | 1000 kBit/s |
| 1 | 800 kBit/s |
| 2 | 500 kBit/s |
| 3 | 250 kBit/s |
| 4 | 125 kBit/s (default) |
| 5 | 100 kBit/s |
| 6 | 50 kBit/s |
| 7 | 20 kBit/s |

### Warning/Error Objects

| Index | Subindex | Name | Type | PDO/SDO | R/W |
|-------|----------|------|------|---------|-----|
| 0x2100 | 0 | Common Warnings | UINT16 | PDO | RO |
| 0x2101 | 0 | Common Errors | UINT16 | PDO | RO |
| 0x2102 | 0 | Chiller Warnings | UINT16 | SDO | RO |
| 0x2103 | 0 | Chiller Errors | UINT16 | SDO | RO |
| 0x2104 | 0 | Seeder Warnings | UINT16 | SDO | RO |
| 0x2105 | 0 | Seeder Errors | UINT16 | SDO | RO |
| 0x2106 | 0 | Amplifier Warnings | UINT16 | SDO | RO |
| 0x2107 | 0 | Amplifier Errors | UINT16 | SDO | RO |
| 0x2108 | 0 | PDG Warnings | UINT16 | SDO | RO |
| 0x2109 | 0 | PDG Errors | UINT16 | SDO | RO |
| 0x210A | 0 | Power Control Warnings | UINT16 | SDO | RO |
| 0x210B | 0 | Power Control Errors | UINT16 | SDO | RO |

### Power/Shutter Objects

| Index | Subindex | Name | Type | Min | Max | PDO/SDO | R/W |
|-------|----------|------|------|-----|-----|---------|-----|
| 0x2500 | 0 | Attenuator SP | UINT8 | 0 | 100 | SDO | RW |
| 0x2501 | 0 | Pulse Picker Divider | UINT32 | 0 | 10000 | SDO | RW |
| 0x2504 | 0 | Custom Shutter SP | BYTE | 0 | 1 | SDO | RW |
| 0x2509 | 1 | Power SP (W) | UINT16 | 0 | 60 | SDO | RW |
| 0x2312 | 0 | Power Actual | INT16 | - | - | SDO | RO |
| 0x2313 | 0 | Shutter Actual | BYTE | - | - | SDO | RO |

---

## Complete Warning Bit Definitions

### Common Warnings (0x2100) - UINT16

| Bit | Value | Name | Description |
|-----|-------|------|-------------|
| 0 | 0x0001 | ILOCK | Safety interlock open |
| 1 | 0x0002 | Chiller | Chiller warning |
| 2 | 0x0004 | Oscillator | Oscillator warning |
| 3 | 0x0008 | Amplifier | Amplifier warning |
| 4 | 0x0010 | Switching Unit | PDG warning |
| 5 | 0x0020 | Power Control | Power control warning |
| 6 | 0x0040 | Controller | Electronics warning |
| 14 | 0x4000 | Keyswitch | Keyswitch warning |
| 15 | 0x8000 | Master Off | Master off active |

### Chiller Warnings - UINT16

| Bit | Value | Name | Description |
|-----|-------|------|-------------|
| 0 | 0x0001 | Temperature | Temperature out of range |
| 1 | 0x0002 | Fluid Level | Fluid level limit reached |
| 2 | 0x0004 | Flow | Coolant flow too low |

### Seeder/Oscillator Warnings - UINT16

| Bit | Value | Name | Description |
|-----|-------|------|-------------|
| 0 | 0x0001 | Diode Current | Current out of spec |
| 1 | 0x0002 | Diode Voltage | Voltage out of spec |
| 2 | 0x0004 | Diode Temperature | Temperature out of range |
| 5 | 0x0020 | Crystal Temperature | Crystal temp warning |
| 8 | 0x0100 | Absolute Modelock | Modelock limit (absolute) |
| 9 | 0x0200 | Relative Modelock | Modelock limit (relative) |

### Amplifier Warnings - UINT16

| Bit | Value | Name | Description |
|-----|-------|------|-------------|
| 0 | 0x0001 | Diode Current | Current out of spec |
| 1 | 0x0002 | Diode Voltage | Voltage out of spec |
| 2 | 0x0004 | Diode Temperature | Temperature out of range |
| 5 | 0x0020 | Crystal Temperature | Crystal temp warning |

### PDG/Switching Unit Warnings - UINT16

| Bit | Value | Name | Description |
|-----|-------|------|-------------|
| 0 | 0x0001 | PDG Supply Voltage | Supply voltage error |
| 1 | 0x0002 | PDG I-Lock | PDG interlock error |
| 2 | 0x0004 | PDG Seed Level | Seed level error |
| 3 | 0x0008 | PDG Frequency | Frequency error |
| 5 | 0x0020 | PDG Gate | Gate error |

### Power Control Warnings - UINT16

| Bit | Value | Name | Description |
|-----|-------|------|-------------|
| 0 | 0x0001 | Shutter | Shutter warning |
| 1 | 0x0002 | Attenuator Calibrated | Attenuator cal warning |
| 2 | 0x0004 | Signal LED | Signal LED warning |
| 3 | 0x0008 | Safety Shutter Relay | Safety relay open |

### Controller Warnings - UINT32

| Bit | Value | Name | Description |
|-----|-------|------|-------------|
| 0 | 0x0001 | Supply Voltage 24V | 24V supply error |
| 1 | 0x0002 | Electronics Temp LCR1 | LCR1 temperature |
| 2 | 0x0004 | Amp Diode TEC Temp | TEC temperature |
| 3 | 0x0008 | DSOSC Temp | DSOSC temperature |
| 4 | 0x0010 | PSXTAL Temp | PSXTAL temperature |
| 5 | 0x0020 | HV Communication | HV comm error |
| 6 | 0x0040 | Amp LD Driver Comm | Driver comm error |
| 8 | 0x0100 | Chiller Communication | Chiller comm error |
| 9 | 0x0200 | DSOSC Communication | DSOSC comm error |
| 10 | 0x0400 | Amp Diode TEC Comm | TEC comm error |
| 11 | 0x0800 | PSXTAL Communication | PSXTAL comm error |
| 12 | 0x1000 | PDG Communication | PDG comm error |
| 13 | 0x2000 | High Voltage | HV error |

### SHG Warnings (Optional) - UINT16

| Bit | Value | Name | Description |
|-----|-------|------|-------------|
| 0 | 0x0001 | SHG Crystal Temp | Crystal temperature |
| 5 | 0x0020 | Air Purging Filter | Filter warning |

---

## Complete Error Bit Definitions

Error bits use the **same structure** as warning bits for each subsystem. When an error bit is set, the corresponding subsystem has entered a fault condition requiring attention.

---

## Control Sequences

### Power-Up Sequence

```
1. state_control=1          # Request Standby (triggers warm-up)
2. Poll state_actual until == 6 (Standby)
   - Cold start: ~30 minutes
   - Warm start: ~5 minutes
3. power_setpoint=X         # Set power level (Watts)
4. shutter_control=1        # Open shutter
5. state_control=2          # Request Laser On
6. Poll state_actual until == 8 (Laser On)
7. Verify power_actual matches expectation
```

### Power-Down Sequence

```
1. shutter_control=0        # Close shutter
2. state_control=1          # Request Standby
3. Poll state_actual until == 6 (Standby)
4. (Optional) state_control=0  # Request Off
```

### Error Recovery

```
1. Read common_errors and subsystem errors
2. Identify and address root cause
3. state_control=0          # Force to Off
4. Poll state_actual until == 4 (Laser Off)
5. Clear errors by cycling power if needed
6. Resume normal power-up sequence
```

---

## Configuration Example

```toml
[[devices]]
id = "spirit_laser"
type = "spirit_laser"
enabled = true

[devices.config]
ip = "192.168.1.100"
port = 9000
warmup_timeout_minutes = 35
default_power_watts = 5.0
```
