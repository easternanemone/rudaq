# PVCAM SDK Reference for rust-daq

This document provides a comprehensive reference for the PVCAM (Programmable Virtual Camera Access Method) SDK version 3.10.x, compiled from the official Teledyne documentation to support the rust-daq PVCAM driver implementation.

**Source:** https://docs.teledynevisionsolutions.com/pvcam-sdk/

---

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Header Files](#header-files)
4. [Basic Types](#basic-types)
5. [Core Functions](#core-functions)
6. [Camera Parameters](#camera-parameters)
7. [Data Structures](#data-structures)
8. [Acquisition Modes](#acquisition-modes)
9. [Callbacks](#callbacks)
10. [Metadata](#metadata)
11. [Best Practices](#best-practices)
12. [Error Handling](#error-handling)
13. [Deprecated Features](#deprecated-features)
14. [rust-daq Implementation Notes](#rust-daq-implementation-notes)

---

## Overview

PVCAM is an ANSI C library providing camera control and data acquisition for Teledyne Photometrics and QImaging cameras. The SDK installer creates an environment variable `PVCAM_SDK_PATH` pointing to binaries and headers.

### Supported Hardware

- All modern Teledyne Photometrics cameras (Prime BSI, etc.)
- Teledyne QImaging cameras (Retiga, LUMO, ELECTRO, MicroPublisher)

---

## Architecture

```
┌─────────────────────┐
│  Application Layer  │  ← Your Rust application
├─────────────────────┤
│    PVCAM Library    │  ← Shared library (libpvcam.so / pvcam64.dll)
├─────────────────────┤
│   Device Drivers    │  ← USB/PCIe drivers
├─────────────────────┤
│  Camera Hardware    │  ← Physical camera
└─────────────────────┘
```

The PVCAM library:
- Handles camera enumeration and device driver communication
- Abstracts hardware differences from the application layer
- Provides thread-safe access to camera functions

---

## Header Files

Two header files are required (must be included in order):

```c
#include "master.h"  // Must be included FIRST
#include "pvcam.h"   // Main PVCAM definitions
```

---

## Basic Types

Defined in `master.h`:

| Type | Definition | Description |
|------|------------|-------------|
| `rs_bool` | `unsigned short` | Return status (PV_OK=1, PV_FAIL=0) |
| `int8` | `signed char` | 8-bit signed integer |
| `uns8` | `unsigned char` | 8-bit unsigned integer |
| `int16` | `short` | 16-bit signed integer |
| `uns16` | `unsigned short` | 16-bit unsigned integer |
| `int32` | `int` | 32-bit signed integer |
| `uns32` | `unsigned int` | 32-bit unsigned integer |
| `flt32` | `float` | 32-bit floating point |
| `flt64` | `double` | 64-bit floating point |
| `long64` | `signed long long` | 64-bit signed integer |
| `ulong64` | `unsigned long long` | 64-bit unsigned integer |

### Return Values

```c
#define PV_FAIL 0
#define PV_OK   1
#define FALSE   PV_FAIL
#define TRUE    PV_OK
```

---

## Core Functions

### Library Initialization

```c
// Initialize PVCAM library - MUST be called first
rs_bool pl_pvcam_init(void);

// Uninitialize library - closes all devices, frees memory
rs_bool pl_pvcam_uninit(void);

// Get PVCAM version (hexadecimal format)
rs_bool pl_pvcam_get_ver(uns16 *pvcam_version);
```

### Camera Enumeration & Management

```c
// Get total number of connected cameras
rs_bool pl_cam_get_total(int16 *totl_cams);

// Get camera name by index (0 to totl_cams-1)
rs_bool pl_cam_get_name(int16 cam_num, char *camera_name);

// Open camera and get handle
rs_bool pl_cam_open(char *camera_name, int16 *hcam, int16 o_mode);

// Close camera
rs_bool pl_cam_close(int16 hcam);
```

### Parameter Access

```c
// Get parameter attribute (current value, min, max, etc.)
rs_bool pl_get_param(int16 hcam, uns32 param_id, int16 param_attribute, void *param_value);

// Set parameter value
rs_bool pl_set_param(int16 hcam, uns32 param_id, void *param_value);

// Get enumeration value and description at index
rs_bool pl_get_enum_param(int16 hcam, uns32 param_id, uns32 index,
                          int32 *value, char *desc, uns32 length);

// Get length of enumeration description string
rs_bool pl_enum_str_length(int16 hcam, uns32 param_id, uns32 index, uns32 *length);
```

#### Parameter Attributes

| Attribute | Description |
|-----------|-------------|
| `ATTR_AVAIL` | Parameter availability (rs_bool) |
| `ATTR_CURRENT` | Current value |
| `ATTR_MIN` | Minimum value |
| `ATTR_MAX` | Maximum value |
| `ATTR_COUNT` | Array element count |
| `ATTR_ACCESS` | Read/write permissions |
| `ATTR_LIVE` | Queryable during imaging |
| `ATTR_DEFAULT` | Factory default value |

### Error Handling

```c
// Get most recent error code
int16 pl_error_code(void);

// Get error message string
rs_bool pl_error_message(int16 err_code, char *msg);
```

---

## Camera Parameters

Parameters are identified by `PARAM_*` macros combining CLASS, TYPE, and ID.

### Device Driver Parameters (CLASS0)

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_DD_INFO_LENGTH` | INT16 | Length of info message |
| `PARAM_DD_VERSION` | UNS16 | Driver version number |
| `PARAM_DD_INFO` | CHAR_PTR | Info message string |
| `PARAM_CAM_INTERFACE_TYPE` | ENUM | Interface type (USB, PCIe) |
| `PARAM_CAM_INTERFACE_MODE` | ENUM | Interface mode (Control/Imaging) |

### Sensor Geometry (CLASS2)

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_SER_SIZE` | UNS16 | Active columns in sensor |
| `PARAM_PAR_SIZE` | UNS16 | Active rows in sensor |
| `PARAM_PIX_SER_SIZE` | UNS16 | Pixel width (nanometers) |
| `PARAM_PIX_PAR_SIZE` | UNS16 | Pixel height (nanometers) |
| `PARAM_PREMASK` | UNS16 | Masked lines near serial register |
| `PARAM_POSTMASK` | UNS16 | Masked lines far from serial register |
| `PARAM_PRESCAN` | UNS16 | Pixels before first data pixel |
| `PARAM_POSTSCAN` | UNS16 | Pixels after last data pixel |
| `PARAM_FWELL_CAPACITY` | UNS32 | Full-well capacity (electrons) |

### Camera Information

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_CHIP_NAME` | CHAR_PTR | Sensor name |
| `PARAM_SYSTEM_NAME` | CHAR_PTR | System name |
| `PARAM_VENDOR_NAME` | CHAR_PTR | Vendor name |
| `PARAM_PRODUCT_NAME` | CHAR_PTR | Product/model name |
| `PARAM_CAMERA_PART_NUMBER` | CHAR_PTR | Part number |
| `PARAM_HEAD_SER_NUM_ALPHA` | CHAR_PTR | Serial number |
| `PARAM_CAM_FW_VERSION` | UNS16 | Firmware version (hex) |
| `PARAM_PCI_FW_VERSION` | UNS16 | PCI firmware version |

### Speed/Gain Table

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_READOUT_PORT` | ENUM | Active readout port |
| `PARAM_SPDTAB_INDEX` | INT16 | Speed selection index |
| `PARAM_SPDTAB_NAME` | CHAR_PTR | Speed name string |
| `PARAM_GAIN_INDEX` | INT16 | Gain setting (1-16) |
| `PARAM_GAIN_NAME` | CHAR_PTR | Gain name string |
| `PARAM_PIX_TIME` | UNS16 | Pixel conversion time (ns) |
| `PARAM_ACTUAL_GAIN` | UNS16 | Actual e/ADU |
| `PARAM_READ_NOISE` | UNS16 | Read noise at current speed |
| `PARAM_BIT_DEPTH` | INT16 | Native bit depth |

### Image Format

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_IMAGE_FORMAT` | ENUM | Native pixel format |
| `PARAM_IMAGE_FORMAT_HOST` | ENUM | Host-side output format |
| `PARAM_BIT_DEPTH` | INT16 | Native bit depth |
| `PARAM_BIT_DEPTH_HOST` | INT16 | Host-side bit depth |
| `PARAM_IMAGE_COMPRESSION` | ENUM | Native compression |

### Temperature Control

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_TEMP` | INT16 | Sensor temperature (hundredths °C) |
| `PARAM_TEMP_SETPOINT` | INT16 | Target temperature (hundredths °C) |
| `PARAM_FAN_SPEED_SETPOINT` | ENUM | Fan speed selection |
| `PARAM_COOLING_MODE` | ENUM | Cooling type |

### Timing

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_READOUT_TIME` | FLT64 | Readout duration (microseconds) |
| `PARAM_CLEARING_TIME` | INT64 | Clearing time (nanoseconds) |
| `PARAM_POST_TRIGGER_DELAY` | INT64 | Post-trigger delay (ns) |
| `PARAM_PRE_TRIGGER_DELAY` | INT64 | Pre-trigger delay (ns) |

### Shutter Control

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_SHTR_OPEN_MODE` | ENUM | Shutter opening condition |
| `PARAM_SHTR_STATUS` | ENUM | Current shutter state |
| `PARAM_SHTR_OPEN_DELAY` | UNS16 | Open delay (ms) |
| `PARAM_SHTR_CLOSE_DELAY` | UNS16 | Close delay (ms) |

### Acquisition (CLASS3)

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_EXPOSURE_TIME` | UNS64 | Exposure time |
| `PARAM_EXP_RES` | ENUM | Exposure resolution |
| `PARAM_EXP_RES_INDEX` | UNS16 | Resolution table index |
| `PARAM_FRAME_BUFFER_SIZE` | UNS64 | Buffer size range (bytes) |
| `PARAM_CIRC_BUFFER` | BOOLEAN | Circular buffer capability |
| `PARAM_BINNING_SER` | ENUM | Serial binning factor |
| `PARAM_BINNING_PAR` | ENUM | Parallel binning factor |
| `PARAM_CLEAR_CYCLES` | UNS16 | Clear cycle count |
| `PARAM_CLEAR_MODE` | ENUM | Clearing timing mode |
| `PARAM_PMODE` | ENUM | Parallel clocking method |
| `PARAM_EXPOSURE_MODE` | ENUM | Exposure/trigger mode |
| `PARAM_EXPOSE_OUT_MODE` | ENUM | Expose Out signal mode |

### Metadata & ROI

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_METADATA_ENABLED` | BOOLEAN | Enable frame metadata |
| `PARAM_METADATA_RESET_TIMESTAMP` | BOOLEAN | Reset timestamp |
| `PARAM_ROI_COUNT` | UNS16 | Configured ROI count |

### S.M.A.R.T. Streaming

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_SMART_STREAM_MODE_ENABLED` | BOOLEAN | Enable S.M.A.R.T. mode |
| `PARAM_SMART_STREAM_MODE` | UNS16 | Streaming mode |
| `PARAM_SMART_STREAM_EXP_PARAMS` | VOID_PTR | Exposure parameters |
| `PARAM_SMART_STREAM_DLY_PARAMS` | VOID_PTR | Delay parameters |

### Host-Side Processing

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_HOST_FRAME_ROTATE` | ENUM | Virtual rotation (0°/90°/180°/270°) |
| `PARAM_HOST_FRAME_FLIP` | ENUM | Virtual flip (X/Y/XY) |
| `PARAM_HOST_FRAME_SUMMING_ENABLED` | BOOLEAN | Frame summing toggle |
| `PARAM_HOST_FRAME_SUMMING_COUNT` | UNS32 | Frames to sum |
| `PARAM_HOST_FRAME_DECOMPRESSION_ENABLED` | BOOLEAN | Auto-decompression |

### Post-Processing

| Parameter | Type | Description |
|-----------|------|-------------|
| `PARAM_PP_INDEX` | INT16 | PP feature index |
| `PARAM_PP_FEAT_NAME` | CHAR_PTR | Feature name |
| `PARAM_PP_FEAT_ID` | UNS16 | Feature module ID |
| `PARAM_PP_PARAM_INDEX` | INT16 | Parameter index |
| `PARAM_PP_PARAM_NAME` | CHAR_PTR | Parameter name |
| `PARAM_PP_PARAM` | UNS32 | Parameter value |
| `PARAM_PP_PARAM_ID` | UNS16 | Parameter ID |

---

## Data Structures

### rgn_type (Region of Interest)

```c
typedef struct {
    uns16 s1;    // First pixel in serial register (X start)
    uns16 s2;    // Last pixel in serial register (X end)
    uns16 sbin;  // Serial binning factor
    uns16 p1;    // First row in parallel register (Y start)
    uns16 p2;    // Last row in parallel register (Y end)
    uns16 pbin;  // Parallel binning factor
} rgn_type;
```

**Image dimensions calculation:**
- Width: `(s2 - s1 + 1) / sbin`
- Height: `(p2 - p1 + 1) / pbin`

### FRAME_INFO

Structure returned by callbacks and frame retrieval functions:

| Field | Type | Description |
|-------|------|-------------|
| `FrameNr` | int32 | Frame number (1-based, per acquisition) |
| `TimeStamp` | uns64 | Frame timestamp |
| `ReadoutTime` | uns32 | Readout duration |
| `TimeStampBOF` | uns32 | Begin-of-frame timestamp |
| `TimeStampEOF` | uns32 | End-of-frame timestamp |

### md_frame_header (Embedded Metadata)

Located before each frame when metadata is enabled:

| Field | Type | Size | Description |
|-------|------|------|-------------|
| `signature` | uns32 | 4B | Must equal `PL_MD_FRAME_SIGNATURE` |
| `version` | uns8 | 1B | Header version (typically 1-3) |
| `frameNr` | uns32 | 4B | 1-based frame counter |
| `roiCount` | uns16 | 2B | Number of ROIs (≥1) |
| `timestampBOF` | uns32 | 4B | Begin-of-frame timestamp |
| `timestampEOF` | uns32 | 4B | End-of-frame timestamp |
| `timestampResNs` | uns32 | 4B | Timestamp resolution (ns) |
| `exposureTime` | uns32 | 4B | Exposure duration |
| `exposureTimeResNs` | uns32 | 4B | Exposure resolution (ns) |
| `bitDepth` | uns8 | 1B | Bit depth (10, 13, 14, 16) |
| `colorMask` | uns8 | 1B | Color mode enum |
| `flags` | uns8 | 1B | Frame flags |
| `extendedMdSize` | uns16 | 2B | Extended metadata size |
| `imageFormat` | uns8 | 1B | Image format (v2+) |
| `imageCompression` | uns8 | 1B | Compression type (v2+) |

### md_frame_roi_header

32-byte header before each ROI data block:

| Field | Type | Size | Description |
|-------|------|------|-------------|
| `roiNr` | uns16 | 2B | ROI ID (1-based) |
| `timestampBOR` | uns32 | 4B | Begin of ROI readout |
| `timestampEOR` | uns32 | 4B | End of ROI readout |
| `roi` | rgn_type | 12B | ROI coordinates |
| `flags` | uns8 | 1B | ROI flags |
| `extendedMdSize` | uns16 | 2B | Extended metadata size |
| `roiDataSize` | uns32 | 4B | ROI data size (v2+) |

### md_frame (Decoded Metadata)

Helper structure for decoded frame metadata:

| Field | Type | Description |
|-------|------|-------------|
| `header` | md_frame_header* | Pointer to header in buffer |
| `extMdData` | void* | Extended metadata pointer |
| `extMdDataSize` | uns16 | Extended metadata size |
| `impliedRoi` | rgn_type | Calculated implied ROI |
| `roiArray` | md_frame_roi* | Array of ROI descriptors |
| `roiCapacity` | uns16 | ROI capacity |
| `roiCount` | uns16 | Decoded ROI count |

### smart_stream_type

S.M.A.R.T. streaming configuration:

```c
typedef struct {
    uns16 entries;  // Number of entries
    uns32 *params;  // Parameter values array
} smart_stream_type;
```

---

## Acquisition Modes

### Sequential Acquisition (Single/Multiple Frames)

```c
// Setup sequential acquisition
rs_bool pl_exp_setup_seq(
    int16 hcam,              // Camera handle
    uns16 exp_total,         // Number of exposures
    uns16 rgn_total,         // Number of regions
    const rgn_type *rgn_array, // Region array
    int16 exp_mode,          // Exposure mode
    uns32 exposure_time,     // Exposure time
    uns32 *exp_bytes         // OUTPUT: bytes required
);

// Start sequential acquisition (non-blocking)
rs_bool pl_exp_start_seq(int16 hcam, void *pixel_stream);

// Finish sequential acquisition
rs_bool pl_exp_finish_seq(int16 hcam, void *pixel_stream, int16 hbuf);
```

### Continuous Acquisition (Streaming)

```c
// Setup continuous acquisition with circular buffer
rs_bool pl_exp_setup_cont(
    int16 hcam,              // Camera handle
    uns16 rgn_total,         // Number of regions
    const rgn_type *rgn_array, // Region array
    int16 exp_mode,          // Exposure mode
    uns32 exposure_time,     // Exposure time
    uns32 *exp_bytes,        // OUTPUT: frame size in bytes
    int16 buffer_mode        // CIRC_NO_OVERWRITE or CIRC_OVERWRITE
);

// Start continuous acquisition
rs_bool pl_exp_start_cont(int16 hcam, void *pixel_stream, uns32 size);

// Stop continuous acquisition
rs_bool pl_exp_stop_cont(int16 hcam, int16 cam_state);

// Abort acquisition immediately
rs_bool pl_exp_abort(int16 hcam, int16 cam_state);
```

### Frame Retrieval

```c
// Get most recently acquired frame
rs_bool pl_exp_get_latest_frame(int16 hcam, void **frame);

// Get most recent frame with metadata
rs_bool pl_exp_get_latest_frame_ex(int16 hcam, void **frame, FRAME_INFO *frame_info);

// Get oldest unretrieved frame
rs_bool pl_exp_get_oldest_frame(int16 hcam, void **frame);

// Get oldest frame with metadata
rs_bool pl_exp_get_oldest_frame_ex(int16 hcam, void **frame, FRAME_INFO *frame_info);

// Release oldest frame for overwrite
rs_bool pl_exp_unlock_oldest_frame(int16 hcam);
```

### Buffer Modes

| Mode | Description |
|------|-------------|
| `CIRC_NO_OVERWRITE` | Frames wait until retrieved; acquisition pauses when full |
| `CIRC_OVERWRITE` | Oldest frames overwritten when buffer full (may not be supported on all cameras) |

**Prime BSI Limitations:**
- `CIRC_OVERWRITE` fails with Error 185 (Invalid Configuration) when EOF callbacks are registered
- `CIRC_NO_OVERWRITE` may stall after ~85 frames at high frame rates due to buffer management issues

**Workaround:** Use sequence mode streaming instead of circular buffer mode (see below).

### Exposure Modes

| Mode | Description |
|------|-------------|
| `TIMED_MODE` | Internal timing, software triggered |
| `EXT_TRIG_INTERNAL` | External trigger, internal exposure control |
| `EXT_TRIG_TIMED` | External trigger, timed exposure |
| `EXT_TRIG_FIRST` | Trigger starts, exposure until next trigger |
| `BULB_MODE` | Exposure lasts duration of trigger signal |

### Software Trigger

```c
// Send software trigger to camera
rs_bool pl_exp_trigger(int16 hcam, uns32 *flags, uns32 value);
```

### Sequence Mode Streaming (Prime BSI Workaround)

When circular buffer modes are unreliable, sequence mode can be used for continuous streaming by repeatedly acquiring batches of frames:

```c
// Continuous streaming using sequence mode
while (streaming) {
    // 1. Setup sequence for batch of N frames
    uns32 buffer_bytes;
    pl_exp_setup_seq(hcam, BATCH_SIZE, 1, &region, TIMED_MODE, exposure_ms, &buffer_bytes);

    // 2. Allocate buffer and start acquisition
    void *buffer = malloc(buffer_bytes);
    pl_exp_start_seq(hcam, buffer);

    // 3. Poll for completion
    int16 status;
    uns32 bytes_arrived;
    while (status != READOUT_COMPLETE) {
        pl_exp_check_status(hcam, &status, &bytes_arrived);
    }

    // 4. Extract and process frames from buffer
    for (int i = 0; i < BATCH_SIZE; i++) {
        void *frame_ptr = buffer + (i * frame_size);
        process_frame(frame_ptr);
    }

    free(buffer);
}
```

**Configuration:**
- `BATCH_SIZE`: 10 frames provides good balance between latency (~150ms at 10ms exposure) and throughput
- `TIMED_MODE`: Software-triggered internal exposure works reliably
- Polling: Use `pl_exp_check_status()` with `READOUT_COMPLETE` (status == 3)

**Advantages:**
- Works reliably on Prime BSI where circular buffer modes fail
- No callback registration conflicts
- Predictable buffer management

**Trade-offs:**
- Slightly higher CPU overhead from batch restarts
- First-frame latency proportional to batch size × exposure time
- ~20-25 FPS achievable at 10ms exposure (vs theoretical ~35 FPS with optimal circular buffer)

---

## Callbacks

### Callback Types

```c
// Legacy (deprecated)
typedef void(PV_DECL * PL_CALLBACK_SIG_LEGACY)(void);

// With context (deprecated)
typedef void(PV_DECL * PL_CALLBACK_SIG_EX)(void *pContext);

// With frame info (deprecated)
typedef void(PV_DECL * PL_CALLBACK_SIG_EX2)(const FRAME_INFO *pFrameInfo);

// Current recommended: frame info + context
typedef void(PV_DECL * PL_CALLBACK_SIG_EX3)(const FRAME_INFO *pFrameInfo, void *pContext);
```

### Callback Registration

```c
// Register callback (use PL_CALLBACK_SIG_EX3)
rs_bool pl_cam_register_callback_ex3(
    int16 hcam,              // Camera handle
    int32 callback_event,    // PL_CALLBACK_EOF for end-of-frame
    void *callback,          // Callback function pointer
    void *context            // User context pointer
);

// Unregister callback
rs_bool pl_cam_deregister_callback(int16 hcam, int32 callback_event);
```

### Callback Events

| Event | Description |
|-------|-------------|
| `PL_CALLBACK_BOF` | Begin of frame (not recommended) |
| `PL_CALLBACK_EOF` | End of frame (recommended) |
| `PL_CALLBACK_CHECK_CAMS` | Camera status change |

### Callback Best Practices

1. **Use `PL_CALLBACK_EOF`** - Most efficient for frame-ready notification
2. **Keep callbacks short** - Signal a condvar/semaphore, don't process frames
3. **Use context pointer** - Pass class instance for OOP integration
4. **Don't call PVCAM functions in callback** - May cause deadlock

---

## Metadata

### Metadata Functions

```c
// Create frame structure for known ROI count
rs_bool pl_md_create_frame_struct_cont(md_frame **pFrame, uns16 roiCount);

// Create frame structure from buffer
rs_bool pl_md_create_frame_struct(md_frame **pFrame, void *pSrcBuf, uns32 srcBufSize);

// Decode metadata from buffer
rs_bool pl_md_frame_decode(md_frame *pDstFrame, void *pSrcBuf, uns32 srcBufSize);

// Recompose multi-ROI frame to displayable image
rs_bool pl_md_frame_recompose(void *pDstBuf, uns16 offX, uns16 offY,
                               uns16 dstWidth, uns16 dstHeight, md_frame *pSrcFrame);

// Release frame structure
rs_bool pl_md_release_frame_struct(md_frame *pFrame);

// Read extended metadata
rs_bool pl_md_read_extended(md_ext_item_collection *pOutput,
                            void *pExtMdPtr, uns32 extMdSize);
```

### FRAME_INFO Management

```c
// Create FRAME_INFO structure
rs_bool pl_create_frame_info_struct(FRAME_INFO **new_frame);

// Release FRAME_INFO structure
rs_bool pl_release_frame_info_struct(FRAME_INFO *frame_to_delete);
```

---

## Best Practices

### Initialization Sequence

1. Call `pl_pvcam_init()` once at application start
2. Enumerate cameras with `pl_cam_get_total()` and `pl_cam_get_name()`
3. Open desired camera with `pl_cam_open()`
4. Query parameters to determine capabilities
5. Configure acquisition parameters
6. Setup and start acquisition

### Shutdown Sequence

1. Stop acquisition (`pl_exp_stop_cont()` or `pl_exp_abort()`)
2. Deregister callbacks
3. Close camera (`pl_cam_close()`)
4. Uninitialize library (`pl_pvcam_uninit()`)

### Frame Retrieval (Continuous Mode)

**Recommended Pattern (Callback + Polling):**

```c
// 1. Register EOF callback to signal when frames ready
pl_cam_register_callback_ex3(hcam, PL_CALLBACK_EOF, eof_callback, context);

// 2. In callback: signal condvar (don't retrieve frame here)
void eof_callback(const FRAME_INFO *info, void *ctx) {
    signal_condvar(ctx);
}

// 3. In processing thread: wait for signal, then drain frames
while (running) {
    wait_for_condvar(ctx, timeout);
    while (pl_exp_get_oldest_frame_ex(hcam, &frame, &info) == PV_OK) {
        process_frame(frame, info);
        pl_exp_unlock_oldest_frame(hcam);
    }
}
```

### Frame Loss Detection

Track `FRAME_INFO.FrameNr` for discontinuities:

```c
static int32 expected_frame = 1;

void process_frame(void *data, FRAME_INFO *info) {
    if (info->FrameNr != expected_frame) {
        int lost = info->FrameNr - expected_frame;
        log_warning("Lost %d frames", lost);
    }
    expected_frame = info->FrameNr + 1;
}
```

### Buffer Sizing

Use `PARAM_FRAME_BUFFER_SIZE` to query recommended buffer size:

```c
uns64 min_size, max_size;
pl_get_param(hcam, PARAM_FRAME_BUFFER_SIZE, ATTR_MIN, &min_size);
pl_get_param(hcam, PARAM_FRAME_BUFFER_SIZE, ATTR_MAX, &max_size);

// Allocate buffer within recommended range
uns32 buffer_size = choose_size_in_range(frame_bytes, num_frames, min_size, max_size);
```

### Parameter Changes During Acquisition

- Most parameters **cannot** be changed while streaming
- Check `ATTR_LIVE` to see if parameter is queryable during imaging
- Stop acquisition before changing ROI, binning, speed, or gain

### Thread Safety

- PVCAM functions are **not** reentrant for the same camera handle
- Use mutex to serialize access to camera handle
- Callbacks run in PVCAM's thread context

---

## Error Handling

### Error Checking Pattern

```c
if (pl_some_function(args) != PV_OK) {
    int16 err_code = pl_error_code();
    char msg[256];
    pl_error_message(err_code, msg);
    handle_error(err_code, msg);
}
```

### Common Error Codes

| Code | Description |
|------|-------------|
| 0 | No error |
| 151 | `PVCAM_VERSION` environment variable not set |
| 185 | Invalid configuration (e.g., unsupported buffer mode) |

### Error Recovery

1. Log error code and message
2. Abort acquisition if active
3. Close and reopen camera if necessary
4. Reinitialize SDK for severe errors

---

## Deprecated Features

**Avoid these deprecated items:**

| Deprecated | Replacement |
|------------|-------------|
| `pl_cam_register_callback` | `pl_cam_register_callback_ex3` |
| `pl_cam_register_callback_ex` | `pl_cam_register_callback_ex3` |
| `pl_cam_register_callback_ex2` | `pl_cam_register_callback_ex3` |
| `PARAM_FRAME_ROTATE` | `PARAM_HOST_FRAME_ROTATE` |
| `PARAM_FRAME_FLIP` | `PARAM_HOST_FRAME_FLIP` |
| `PARAM_NAME_LEN` | `MAX_PP_NAME_LEN` |
| `PARAM_BOF_EOF_ENABLE` | Use callback acquisition |
| `PARAM_BOF_EOF_COUNT` | Use callback acquisition |
| `PARAM_BOF_EOF_CLR` | Use callback acquisition |

---

## rust-daq Implementation Notes

### Current Implementation Status

The rust-daq PVCAM driver (`crates/daq-driver-pvcam/`) implements:

- **Connection management** (`components/connection.rs`): Init, open, close
- **Acquisition** (`components/acquisition.rs`): EOF callbacks, circular buffer, streaming
- **Features** (`components/features.rs`): Parameter access, enumerations
- **Frame loss detection**: Tracks `FrameNr` discontinuities

### FFI Bindings

Located in `crates/daq-driver-pvcam/pvcam-sys/`:
- Generated from PVCAM SDK headers
- Requires `PVCAM_SDK_DIR` environment variable

### Acquisition Architecture

```
PVCAM SDK                    rust-daq
┌─────────────────┐         ┌─────────────────────────────────┐
│ Camera Hardware │         │ CallbackContext                 │
│                 │         │ ├─ pending_frames: AtomicU32    │
│ EOF Interrupt ──┼────────►│ ├─ condvar: Condvar             │
│                 │ callback│ ├─ mutex: Mutex                 │
│                 │         │ └─ shutdown: AtomicBool         │
└─────────────────┘         └────────────┬────────────────────┘
                                         │ signal
                                         ▼
                            ┌─────────────────────────────────┐
                            │ Frame Retrieval Loop            │
                            │ ├─ wait on condvar              │
                            │ ├─ pl_exp_get_oldest_frame_ex   │
                            │ └─ broadcast Frame to channels  │
                            └─────────────────────────────────┘
```

### Key Implementation Details

1. **Sequence mode streaming** (default for Prime BSI) - acquires batches of 10 frames using `pl_exp_setup_seq`/`pl_exp_start_seq`, then restarts for continuous streaming
2. **Fallback callback-based acquisition** using `pl_cam_register_callback_ex3` with `PL_CALLBACK_EOF` (for cameras that support circular buffer)
3. **Counter-based pending frames** to avoid losing events during processing
4. **Frame loss detection** via `FrameNr` tracking
5. **Dynamic buffer sizing** using `PARAM_FRAME_BUFFER_SIZE`

### Streaming Architecture (Sequence Mode)

```
┌─────────────────────────────────────────────────────────┐
│ frame_loop_sequence (blocking thread)                   │
│                                                         │
│  while streaming:                                       │
│    1. pl_exp_setup_seq(BATCH_SIZE=10 frames)           │
│    2. pl_exp_start_seq(buffer)                         │
│    3. poll pl_exp_check_status until READOUT_COMPLETE  │
│    4. extract frames from buffer                        │
│    5. send to broadcast channel (frame_tx)             │
│    6. repeat                                            │
└─────────────────────────────────────────────────────────┘
```

### Hardware-Specific Notes (Prime BSI)

- **Sensor**: GS2020, 2048x2048 pixels
- **SDK Version**: PVCAM 3.10.2.5
- **Streaming Mode**: Sequence mode (circular buffer unreliable - Error 185 / stalls)
- **Batch Size**: 10 frames (~150ms latency at 10ms exposure)
- **Achievable FPS**: ~23 FPS at 10ms exposure (full 2048x2048)
- **Required Environment**: `PVCAM_VERSION`, `PVCAM_SDK_DIR`, `LD_LIBRARY_PATH`

### Testing

```bash
# Environment setup
source /etc/profile.d/pvcam.sh
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH

# Smoke test
export PVCAM_SMOKE_TEST=1
cargo test --features pvcam_sdk --test pvcam_sdk_smoke -- --nocapture

# Full hardware tests
cargo test --features 'instrument_photometrics,pvcam_sdk,hardware_tests' \
  --test hardware_pvcam_validation -- --nocapture --test-threads=1
```

---

## References

- **Official Documentation**: https://docs.teledynevisionsolutions.com/pvcam-sdk/
- **rust-daq PVCAM Driver**: `crates/daq-driver-pvcam/`
- **FFI Bindings**: `crates/daq-driver-pvcam/pvcam-sys/`
- **Troubleshooting**: `docs/troubleshooting/PVCAM_SETUP.md`
