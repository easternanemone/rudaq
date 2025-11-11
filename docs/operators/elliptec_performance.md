# Elliptec ELL14 Performance Characteristics

This document details the performance characteristics and operational limits of the Thorlabs Elliptec ELL14 rotation mount, based on official manufacturer specifications and internal testing observations.

## Performance Specifications

| Parameter                   | Value                                   | Notes                                                                                             |
| --------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------- |
| **Maximum Rotation Speed**  | 430 °/s                                 | Natural variability may occur; speed may increase with usage.                                     |
| **Settling Time**           | ~20-50 ms (typical)                     | Inferred from p99 response time in hardware tests (`tests/elliptec_hardware_test.rs`).              |
| **Bidirectional Accuracy**  | 0.4° (6.98 mrad)                        | Maximum deviation from true position.                                                             |
| **Homing Repeatability**    | 0.1° (1.75 mrad)                        | Precision of returning to the home position.                                                      |
| **Bidirectional Repeatability** | 0.05° (873 µrad)                        | Maximum difference between clockwise and counter-clockwise movements to the same position.      |
| **Minimum Lifetime**        | >600,000 Revolutions (100 km)           | Under recommended operating conditions.                                                           |
| **Continuous Operation**    | Not Recommended                         | Recommended duty cycle is < 40%. Duty cycles > 60% should be limited to a few seconds to avoid overheating. |

*Note: Performance specifications are based on a 64 g load with a moment of inertia of 6600 g·mm².*

## Position Encoding and Resolution

A notable discrepancy exists between the official documentation and the value used in the `rust_daq` driver for pulses per revolution.

-   **Official Specification (`ELL14` Hardware Manual):** 143,360 counts/revolution
-   **`rust_daq` Driver (`src/instruments_v2/elliptec.rs`):** 136,533 counts/revolution

This difference may be due to firmware variations or a specific hardware revision. The driver's value has been validated against the hardware used in this project (`tests/elliptec_hardware_test.rs`).

-   **Encoder Resolution (Driver):** ~0.0026°/count (360° / 136,533)
-   **Minimum Incremental Motion:** 0.002° (34.9 µrad)

## Operational Limits

-   **Maximum Load:** 50 g (must be centered in the mount).
-   **Duty Cycle:** For operation at max speed or full power, the duty cycle should be less than 40% where possible and never exceed 60% for more than a few seconds.
-   **Movement Path:** It is recommended to move in the shortest path (e.g., from 350° to 5°, move clockwise 15° rather than counter-clockwise 345°).
-   **Environment:** The device is sensitive to magnetic fields, which can affect homing and positioning.

For further details, refer to the official Thorlabs documentation for the [ELL14 rotation mount](https://www.thorlabs.com/newgrouppage9.cfm?objectgroup_id=12829).
