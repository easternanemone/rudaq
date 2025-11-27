# Elliptec Rotator Operator Manual

## 1. Introduction

This document provides operators with standard procedures for using Elliptec series motorized rotation stages. Following these guidelines will ensure safe and effective operation.

## 2. Safety Considerations

- **Moving Parts:** The rotator is a motorized device. Keep hands, clothing, and cables clear of the rotation path to prevent entanglement or mechanical jams.
- **Obstructions:** Ensure the rotation path is clear of any obstructions before initiating movement. Collisions can damage the rotator or other equipment.
- **Power Supply:** Use only the specified power supply. Incorrect voltage or current can damage the device.
- **Emergency Stop:** Be aware of how to send a `st` (Stop) command from the control software to halt movement immediately in case of an issue.

## 3. Startup Procedure

1.  **Hardware Connection:**
    -   Connect the Elliptec rotator to its controller using the supplied cable.
    -   Connect the controller to the appropriate power supply.
    -   Connect the controller to the computer using an RS-232 serial cable or USB-to-Serial adapter.

2.  **Software Connection:**
    -   Launch the control software (e.g., Thorlabs APT, a custom application).
    -   Establish a serial connection by selecting the correct COM port and using the following settings:
        -   **Baud Rate**: 9600
        -   **Data Bits**: 8
        -   **Parity**: None
        -   **Stop Bits**: 1

3.  **Verify Connection:**
    -   Send a "Get Information" (`in`) or "Get Status" (`gs`) command to the rotator's address (default is `0`).
    -   A successful connection will return device information or a status code. Upon power-up, the motor will energize, which can be confirmed with the status command.

## 4. Homing Process

The homing process establishes a "zero" reference position for the rotator. Absolute position moves are not reliable until the device has been homed.

1.  **Initiate Homing:**
    -   Send the "Home" (`ho`) command.
    -   The rotator will begin moving to find its datum (zero) position. Do not interrupt this process.

2.  **Monitor Homing:**
    -   The device status will indicate that homing is in progress.

3.  **Confirm Homing:**
    -   Once movement stops, send a "Get Status" (`gs`) command.
    -   A successful homing is indicated when the "Homed" flag is set in the status response. The device is now ready for absolute positioning.

## 5. Position Commands

All position commands are based on a 360-degree circle. The control software typically handles the conversion from degrees to the device's internal units ("pulses").

| Command | Description | Example Use |
| --- | --- | --- |
| **Move Absolute** (`ma`) | Moves the rotator to a specific angle (e.g., 90.0°). This requires the device to be homed first. | `0ma<position_data>` - Move device `0` to 90 degrees. |
| **Move Relative** (`mr`) | Moves the rotator by a specified amount from its current position (e.g., move +10°). | `0mr<displacement_data>` - Rotate device `0` forward by 10 degrees. |
| **Move Forward** (`fw`) | Starts continuous rotation in the forward direction. | Used for manual alignment. Requires a `st` command to stop. |
| **Move Backward** (`bw`) | Starts continuous rotation in the backward direction. | Used for manual alignment. Requires a `st` command to stop. |
| **Stop** (`st`) | Immediately stops any motor movement. | An essential command for stopping a jog or aborting a move. |
| **Get Position** (`gp`) | Requests the current angle of the rotator. | Used to verify the rotator has reached its target position. |

## 6. Typical Use Cases

-   **Automated Scans:** Writing a script to send a sequence of "Move Absolute" commands to rotate a sample or sensor to precise angles for automated data collection.
-   **Polarization Control:** Rotating a polarizer or half-wave plate to control the polarization of a laser beam.
-   **Manual Alignment:** Using the "Move Forward" and "Move Backward" (jog) commands for fine-tuning the angular alignment of an optical component.

### 6.1 Rhai Script Examples

The system provides ready-to-use Rhai scripts for common workflows:

-   **`examples/angular_power_scan.rhai`** - Measures optical power at multiple rotation angles using the Newport 1830-C power meter and Elliptec rotator. Useful for characterizing polarization-dependent transmission.
-   **`examples/multi_angle_acquisition.rhai`** - Acquires camera frames at multiple stage positions while rotating a polarization element. Demonstrates multi-dimensional scans.
-   **`examples/focus_scan.rhai`** - Performs Z-axis focus scans to find the optimal focal plane.

Run scripts with: `cargo run -- run examples/angular_power_scan.rhai`

## 7. Troubleshooting

| Issue | Possible Cause | Solution |
| --- | --- | --- |
| **No response from device** | - Incorrect COM port or address.<br>- Loose cable connections.<br>- No power to the device. | - Verify serial port settings and device address.<br>- Check all cable connections.<br>- Ensure the power supply is on. |
| **Homing fails** | - Physical obstruction in the rotation path.<br>- Motor or sensor error. | - Clear any obstructions and re-issue the `ho` command.<br>- If it fails again, cycle power and retry. If the issue persists, the device may require service. |
| **Move command fails or times out** | - Physical obstruction.<br>- The requested angle is out of range.<br>- The device has not been homed (for absolute moves). | - Check for obstructions.<br>- Ensure the target angle is valid.<br>- Home the device before sending absolute move commands. |
| **Position is inaccurate** | - The device was not homed after power-up.<br>- Incorrect conversion factor (pulses-per-revolution) used in the control software. | - Always home the device after powering it on.<br>- Verify the software is configured for the correct rotator model (e.g., ELL14). |

If an error occurs, the "Get Status" (`gs`) command will indicate an error state. Sending the `gs` command a second time will return a specific error code, which can be used to diagnose the problem further.
