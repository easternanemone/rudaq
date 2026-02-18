# Andor SDK3 Driver Development Guide

## Overview

Andor SDK3 is a feature-based camera API for Andor Technologies cameras (Zyla, Neo, iStar, Kymera, SONA, Marana, Balor families).

**Libraries:** `atcore.dll` (Windows) / `libatcore.so` (Linux)  
**Reference:** `docs/reference/markdown/andor-sdk3.md`

---

## Core Concepts

### Wide Character Strings (AT_WC)

All feature names use 16-bit wide character strings:

```rust
use widestring::WideCString;
let feature = WideCString::from_str("ExposureTime").unwrap();
```

### Device Handles

| Handle | Description |
|--------|-------------|
| `AT_H` | Camera handle from `AT_Open()` |
| `AT_HANDLE_SYSTEM` | System-level features (`DeviceCount`, library versions) |

---

## Complete API Reference

### Library Initialization

```c
int AT_InitialiseLibrary();   // MUST be called first
int AT_FinaliseLibrary();     // Call at shutdown
int AT_Open(int DeviceIndex, AT_H* Handle);
int AT_OpenDevice(AT_WC* Device, AT_H* Handle);
int AT_Close(AT_H Handle);
```

### Integer Features (AT_64)

```c
int AT_SetInt(AT_H Hndl, AT_WC* Feature, AT_64 Value);
int AT_GetInt(AT_H Hndl, AT_WC* Feature, AT_64* Value);
int AT_GetIntMin(AT_H Hndl, AT_WC* Feature, AT_64* MinValue);
int AT_GetIntMax(AT_H Hndl, AT_WC* Feature, AT_64* MaxValue);
```

### Float Features (double)

```c
int AT_SetFloat(AT_H Hndl, AT_WC* Feature, double Value);
int AT_GetFloat(AT_H Hndl, AT_WC* Feature, double* Value);
int AT_GetFloatMin(AT_H Hndl, AT_WC* Feature, double* MinValue);
int AT_GetFloatMax(AT_H Hndl, AT_WC* Feature, double* MaxValue);
```

### Boolean Features (AT_BOOL)

```c
int AT_SetBool(AT_H Hndl, AT_WC* Feature, AT_BOOL Value);  // AT_TRUE=1, AT_FALSE=0
int AT_GetBool(AT_H Hndl, AT_WC* Feature, AT_BOOL* Value);
```

### Enumerated Features

```c
int AT_SetEnumIndex(AT_H Hndl, AT_WC* Feature, int Value);
int AT_SetEnumString(AT_H Hndl, AT_WC* Feature, AT_WC* String);
int AT_GetEnumIndex(AT_H Hndl, AT_WC* Feature, int* Value);
int AT_GetEnumCount(AT_H Hndl, AT_WC* Feature, int* Count);
int AT_GetEnumStringByIndex(AT_H Hndl, AT_WC* Feature, int Index, AT_WC* String, int StringLength);
int AT_IsEnumIndexAvailable(AT_H Hndl, AT_WC* Feature, int Index, AT_BOOL* Available);
int AT_IsEnumIndexImplemented(AT_H Hndl, AT_WC* Feature, int Index, AT_BOOL* Implemented);
```

### Command Features

```c
int AT_Command(AT_H Hndl, AT_WC* Feature);
```

### String Features

```c
int AT_SetString(AT_H Hndl, AT_WC* Feature, AT_WC* Value);
int AT_GetString(AT_H Hndl, AT_WC* Feature, AT_WC* Value, int StringLength);
int AT_GetStringMaxLength(AT_H Hndl, AT_WC* Feature, int* MaxStringLength);
```

### Feature Meta-Functions

```c
int AT_IsImplemented(AT_H Hndl, AT_WC* Feature, AT_BOOL* Implemented);
int AT_IsReadOnly(AT_H Hndl, AT_WC* Feature, AT_BOOL* ReadOnly);
int AT_IsReadable(AT_H Hndl, AT_WC* Feature, AT_BOOL* Readable);
int AT_IsWritable(AT_H Hndl, AT_WC* Feature, AT_BOOL* Writable);
```

### Feature Callbacks

```c
typedef int (AT_EXP_CONV *FeatureCallback)(AT_H Hndl, const AT_WC* Feature, void* Context);
int AT_RegisterFeatureCallback(AT_H Hndl, AT_WC* Feature, FeatureCallback EvCallback, void* Context);
int AT_UnregisterFeatureCallback(AT_H Hndl, AT_WC* Feature, FeatureCallback EvCallback, void* Context);
```

### Buffer Management

```c
int AT_QueueBuffer(AT_H Hndl, AT_U8* Ptr, int PtrSize);
int AT_WaitBuffer(AT_H Hndl, AT_U8** Ptr, int* PtrSize, unsigned int Timeout);
int AT_Flush(AT_H Hndl);
```

---

## Complete Feature List

### Acquisition Control

| Feature | Type | Access | Description |
|---------|------|--------|-------------|
| `AcquisitionStart` | Command | W | Start image acquisition |
| `AcquisitionStop` | Command | W | Stop image acquisition |
| `CycleMode` | Enum | RW | Acquisition mode |
| `FrameCount` | Integer | RW | Number of frames for Fixed mode |
| `FrameRate` | Float | RW | Acquisition frame rate (fps) |

**CycleMode Values:**
- `Continuous` - Stream frames continuously
- `Fixed` - Acquire FrameCount frames then stop
- `Single` - Acquire one frame (deprecated on some cameras)

### Image Properties

| Feature | Type | Access | Description |
|---------|------|--------|-------------|
| `AOIWidth` | Integer | RW | Width of AOI in super-pixels |
| `AOIHeight` | Integer | RW | Height of AOI in super-pixels |
| `AOILeft` | Integer | RW | Left offset of AOI in sensor pixels |
| `AOITop` | Integer | RW | Top offset of AOI in sensor pixels |
| `AOIHBin` | Integer | RW | Horizontal binning factor |
| `AOIVBin` | Integer | RW | Vertical binning factor |
| `AOIBinning` | Enum | RW | Symmetric binning (overrides HBin/VBin) |
| `AOIStride` | Integer | R | Bytes per row including padding |
| `ImageSizeBytes` | Integer | R | Total frame buffer size in bytes |
| `FullAOIControl` | Boolean | R | Whether camera supports full AOI control |
| `VerticallyCentreAOI` | Boolean | RW | Auto-center AOI vertically |

### Exposure & Timing

| Feature | Type | Access | Range | Description |
|---------|------|--------|-------|-------------|
| `ExposureTime` | Float | RW | Camera-dependent | Exposure duration in seconds |
| `PixelReadoutRate` | Enum | RW | - | Speed of pixel readout |

**PixelReadoutRate Values:**
- `100 MHz`
- `200 MHz`
- `280 MHz`
- `560 MHz` (camera-dependent)

### Pixel Encoding

| Feature | Type | Access | Description |
|---------|------|--------|-------------|
| `PixelEncoding` | Enum | RW | Pixel data format |

**PixelEncoding Values:**

| Value | Bits | Bytes/Pixel | Description |
|-------|------|-------------|-------------|
| `Mono8` | 8 | 1 | 8-bit monochrome |
| `Mono12` | 12 | 2 | 12-bit in 16-bit container |
| `Mono12Packed` | 12 | 1.5 | Packed 12-bit (2 pixels in 3 bytes) |
| `Mono16` | 16 | 2 | Full 16-bit |
| `Mono32` | 32 | 4 | 32-bit (diagnostic) |

### Trigger Configuration

| Feature | Type | Access | Description |
|---------|------|--------|-------------|
| `TriggerMode` | Enum | RW | Trigger configuration |
| `SoftwareTrigger` | Command | W | Issue software trigger |

**TriggerMode Values:**
- `Internal` - Camera-controlled triggering
- `External` - External trigger per frame
- `External Start` - External trigger starts sequence
- `External Exposure` - External controls exposure
- `Software` - Software-triggered via command

### Sensor Control

| Feature | Type | Access | Range | Description |
|---------|------|--------|-------|-------------|
| `SensorCooling` | Boolean | RW | - | Enable thermoelectric cooling |
| `SensorTemperature` | Float | R | - | Current sensor temperature (°C) |
| `TemperatureControl` | Enum | RW | - | Target temperature setpoint |
| `TemperatureStatus` | Enum | R | - | Cooling system status |

**TemperatureStatus Values:**
- `Cooler Off` - Cooling disabled
- `Cooling` - Actively cooling
- `Stabilised` - Target temperature reached
- `Not Stabilised` - Target not yet reached
- `Drift` - Temperature drifting
- `Fault` - Cooling system error
- `Sensor Over Temperature` - Sensor too hot

### Pre-Amplifier

| Feature | Type | Access | Description |
|---------|------|--------|-------------|
| `PreAmpGain` | Enum | RW | Pre-amplifier gain setting |
| `PreAmpGainControl` | Enum | RW | Gain mode |

**PreAmpGainControl Values:**
- `12-bit (low noise)` - 12-bit with lowest noise
- `16-bit (low noise & high well capacity)` - 16-bit balanced mode

### Metadata

| Feature | Type | Access | Description |
|---------|------|--------|-------------|
| `MetadataEnable` | Boolean | RW | Include metadata in data stream |
| `MetadataFrameInfo` | Boolean | RW | Include frame info block |
| `MetadataTimestamp` | Boolean | RW | Include timestamp block |
| `TimestampClock` | Integer | R | Current timestamp counter value |
| `TimestampClockFrequency` | Integer | R | Timestamp clock frequency (Hz) |
| `TimestampClockReset` | Command | W | Reset timestamp to zero |

### Device Information

| Feature | Type | Access | Description |
|---------|------|--------|-------------|
| `SerialNumber` | String | R | Camera serial number |
| `CameraName` | String | R | Camera model name |
| `FirmwareVersion` | String | R | Firmware version |
| `DeviceCount` | Integer | R | Number of connected cameras (system handle) |
| `SoftwareVersion` | String | R | Library software version |

---

## Complete Error Code List

| Code | Constant | Description |
|------|----------|-------------|
| 0 | `AT_SUCCESS` | Function call successful |
| 1 | `AT_ERR_NOTINITIALISED` | SDK not initialized |
| 2 | `AT_ERR_NOTIMPLEMENTED` | Feature not implemented for this camera |
| 3 | `AT_ERR_READONLY` | Feature is read-only |
| 4 | `AT_ERR_NOTREADABLE` | Feature currently not readable |
| 5 | `AT_ERR_NOTWRITABLE` | Feature currently not writable |
| 6 | `AT_ERR_OUTOFRANGE` | Value outside min/max limits |
| 7 | `AT_ERR_INDEXNOTAVAILABLE` | Enum index not available |
| 8 | `AT_ERR_INDEXNOTIMPLEMENTED` | Enum index not implemented |
| 9 | `AT_ERR_EXCEEDEDMAXSTRINGLENGTH` | String exceeds max length |
| 10 | `AT_ERR_CONNECTION` | Hardware connection error |
| 11 | `AT_ERR_NODATA` | No data available |
| 12 | `AT_ERR_INVALIDHANDLE` | Invalid device handle |
| 13 | `AT_ERR_TIMEDOUT` | AT_WaitBuffer timeout |
| 14 | `AT_ERR_BUFFERFULL` | Input queue at capacity |
| 15 | `AT_ERR_INVALIDSIZE` | Buffer size doesn't match frame |
| 16 | `AT_ERR_INVALIDALIGNMENT` | Buffer not 8-byte aligned |
| 17 | `AT_ERR_COMM` | Hardware communication error |
| 18 | `AT_ERR_STRINGNOTAVAILABLE` | String not available |
| 19 | `AT_ERR_STRINGNOTIMPLEMENTED` | String not implemented |
| 20 | `AT_ERR_NULL_FEATURE` | NULL feature name |
| 21 | `AT_ERR_NULL_HANDLE` | NULL device handle |
| 22 | `AT_ERR_NULL_IMPLEMENTED_VAR` | NULL implemented variable |
| 23 | `AT_ERR_NULL_READABLE_VAR` | NULL readable variable |
| 24 | `AT_ERR_NULL_READONLY_VAR` | NULL readonly variable |
| 25 | `AT_ERR_NULL_WRITABLE_VAR` | NULL writable variable |
| 26 | `AT_ERR_NULL_MINVALUE` | NULL min value |
| 27 | `AT_ERR_NULL_MAXVALUE` | NULL max value |
| 28 | `AT_ERR_NULL_VALUE` | NULL value |
| 29 | `AT_ERR_NULL_STRING` | NULL string |
| 30 | `AT_ERR_NULL_COUNT_VAR` | NULL count variable |
| 31 | `AT_ERR_NULL_ISAVAILABLE_VAR` | NULL availability variable |
| 32 | `AT_ERR_NULL_MAXSTRINGLENGTH` | NULL max string length |
| 33 | `AT_ERR_NULL_EVCALLBACK` | NULL event callback |
| 34 | `AT_ERR_NULL_QUEUE_PTR` | NULL queue pointer |
| 35 | `AT_ERR_NULL_WAIT_PTR` | NULL wait pointer |
| 36 | `AT_ERR_NULL_PTRSIZE` | NULL pointer size |
| 37 | `AT_ERR_NOMEMORY` | Memory allocation failed |
| 38 | `AT_ERR_DEVICEINUSE` | Device already in use |
| 100 | `AT_ERR_HARDWARE_OVERFLOW` | Camera buffer overflow |

---

## Buffer Management

### Critical: 8-Byte Alignment

All buffers **MUST** be 8-byte aligned:

```rust
fn align_buffer(ptr: *mut u8) -> *mut u8 {
    let addr = ptr as usize;
    ((addr + 7) & !7) as *mut u8
}
```

### Buffer Queue Architecture

```
Application → [Input Queue (FIFO)] → SDK Processing → [Output Queue (FIFO)] → Application
                   ↑                                           ↓
            AT_QueueBuffer()                            AT_WaitBuffer()
```

### Buffer Lifecycle

1. **Pre-acquisition:** Query `ImageSizeBytes`, allocate aligned buffers
2. **Queue buffers:** Call `AT_QueueBuffer()` for each buffer (10+ recommended)
3. **Start:** Call `AT_Command("AcquisitionStart")`
4. **Loop:** `AT_WaitBuffer()` → process → re-queue
5. **Stop:** Call `AT_Command("AcquisitionStop")` then `AT_Flush()`

### Image Stride

Each row may have padding for alignment:

```rust
let stride = get_int(handle, "AOIStride")?;
let width_pixels = get_int(handle, "AOIWidth")?;

for row in 0..height {
    let row_start = (row * stride) as usize;
    // Process width_pixels of data at &buffer[row_start..]
}
```

### Metadata Block Format

When `MetadataEnable=true`, metadata is appended to image data:

```
[Image Pixels][Metadata Block 0][Block 1]...[Block N]
                    ↑
           Parse backwards from end
```

**Block Format:** `[Data][CID: 4 bytes][Length: 4 bytes]`

| CID | Name | Content |
|-----|------|---------|
| 0 | Frame Data | Image pixels |
| 1 | FPGA Ticks | 64-bit timestamp counter |
| 7 | Frame Info | AOI, encoding, stride metadata |

---

## Callback System

### Registration

```c
int MyCallback(AT_H Hndl, const AT_WC* Feature, void* Context) {
    // Handle feature change - DO NOT modify features here
    return AT_CALLBACK_SUCCESS;
}

AT_RegisterFeatureCallback(handle, L"ExposureTime", MyCallback, &my_context);
```

### Callback Rules

1. Called immediately after registration (for initialization)
2. Called when feature value or characteristics change
3. Must complete quickly - never block
4. **NEVER** modify features inside callback (causes deadlock)
5. Multiple callbacks can be registered per feature

---

## Camera-Specific Notes

### Supported Cameras

- **Neo, Zyla** - regcam library
- **Sona, Marana, Balor** - chamcam library
- **Apogee** - apogee library
- **SimCam** - simcam library (simulator)

### Feature Availability

Features vary by camera. Always check with `AT_IsImplemented()`:

```rust
fn has_feature(handle: AT_H, name: &str) -> bool {
    let mut implemented: AT_BOOL = 0;
    unsafe {
        AT_IsImplemented(handle, wide(name).as_ptr(), &mut implemented) == AT_SUCCESS
            && implemented == AT_TRUE
    }
}
```

### Pre-Amp/Encoding Coupling

When `PreAmpGainControl` changes:
- **12-bit → 16-bit:** Mono12/Mono12Packed → Mono16
- **16-bit → 12-bit:** Non-Mono32 → Mono12/Mono12Packed
- **Mono32:** Never auto-changes

---

## Configuration Example

```toml
[[devices]]
id = "camera_main"
type = "andor_sdk3"
enabled = true

[devices.config]
device_index = 0
pixel_encoding = "Mono16"
exposure_ms = 100.0
cooling_target = -20
```
