# Thorlabs Elliptec Stage Communication Protocol

This document provides a comprehensive summary of the communication protocol for Thorlabs Elliptec series motorized stages (e.g., ELL4, ELL8, ELL9, ELL14, ELL17, ELL18, ELL20). It is based on the "ELLx modules protocol manual_Issue7.pdf".

## 1. Protocol Structure

### 1.1. Physical Layer

-   **Interface**: RS-232 Serial
-   **Baud Rate**: 9600
-   **Data Bits**: 8
-   **Parity**: None
-   **Stop Bits**: 1
-   **Flow Control**: None

### 1.2. Packet Structure

All communication consists of sending ASCII strings.

#### Command Packet

The command packet format is: `A<ID>[<data>]`

-   `A`: A single ASCII character representing the device address (`0`-`9`, `A`-`F`). The default address is `0`.
-   `<ID>`: A two-character ASCII command identifier (e.g., `ho` for home, `ma` for move absolute).
-   `[<data>]`: Optional ASCII-encoded data payload, whose length and format depend on the command.

**Note**: While not explicitly stated in the manual, serial command-line interfaces typically require a command terminator. A Carriage Return (`<CR>`, ASCII 13) is a common convention and should be assumed.

#### Response Packet

The response packet format is: `A<ID><data>`

-   `A`: The single-character address of the responding device.
-   `<ID>`: A two-character ASCII response identifier (e.g., `PO` for position, `GS` for status).
-   `<data>`: An ASCII-encoded data payload. The length and format depend on the response type.

## 2. All Available Commands

Commands are case-sensitive.

| Command             | Message ID | Data Format                                    | Description                                                                                             |
| ------------------- | ---------- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Home                | `ho`       | None                                           | Starts the homing sequence. The motor moves to the 'zero' datum position.                               |
| Move Absolute       | `ma`       | 8-char hex string (32-bit position)            | Moves the motor to an absolute position. See Section 4 for position encoding.                           |
| Move Relative       | `mr`       | 8-char hex string (32-bit signed displacement) | Moves the motor by a relative amount. Data is a two's complement signed 32-bit integer.                 |
| Move Forward (Jog)  | `fw`       | None                                           | Starts the motor moving in the forward direction. Stops when a `st` (Stop) command is received.         |
| Move Backward (Jog) | `bw`       | None                                           | Starts the motor moving in the backward direction. Stops when a `st` (Stop) command is received.        |
| Stop                | `st`       | None                                           | Stops any motor movement.                                                                               |
| Get Position        | `gp`       | None                                           | Requests the current motor position. The device will respond with a `PO` (Position) packet.             |
| Set Home Offset     | `so`       | 8-char hex string (32-bit position)            | Sets the offset from the 'zero' datum that is used for the `ho` command.                                |
| Get Home Offset     | `go`       | None                                           | Requests the configured home offset. The device responds with a `PO` (Position) packet.                 |
| Get Status          | `gs`       | None                                           | Requests the device status. The device responds with a `GS` (Status) packet.                            |
| Get Information     | `in`       | None                                           | Requests device information. The device responds with an `IN` (Info) packet.                            |
| Change Address      | `ca`       | 1-char new address (`0`-`F`)                   | Changes the device's address. The change is stored in non-volatile memory.                              |
| Set Jog Step Size   | `sj`       | 8-char hex string (32-bit position)            | Sets the step size for a single jog move (`mj`).                                                        |
| Get Jog Step Size   | `gj`       | None                                           | Requests the configured jog step size. The device responds with a `PO` (Position) packet.               |
| Move Jog            | `mj`       | 1-char direction (`f` or `b`)                  | Executes a single jog move of the configured step size in the specified direction (forward/backward). |

## 3. Response Formats and Status Codes

### 3.1. Response Types

| Response Type | Message ID | Data Format                                                                                                                                                             | Description                                                                                                                            |
| ------------- | ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Position      | `PO`       | 8-char hex string (32-bit position)                                                                                                                                     | Reports the current motor position or a configured position value (e.g., home offset).                                                 |
| Status        | `GS`       | 4-char hex string (16-bit status word)                                                                                                                                  | Reports the device's status flags. See below for the bitfield breakdown.                                                               |
| Information   | `IN`       | 14-char ASCII string: `ELLn TTTT SSSSSSSS`<br>`n`: Module type<br>`TTTT`: Thread/drive type<br>`SSSSSSSS`: Serial number                                                  | Reports device hardware information.                                                                                                   |
| Error         | `ER`       | 2-char hex string (8-bit error code)                                                                                                                                    | Reports that an error has occurred. See Section 5 for error codes.                                                                     |
| No Error      | `OK`       | None                                                                                                                                                                    | Sent in response to commands that do not return data (e.g., `ca`, `so`). This response is not documented but is typical for such devices. |

### 3.2. Status Codes (for `GS` Response)

The `GS` response returns a 16-bit status word, formatted as a 4-character hex string (e.g., `0001`).

| Bit | Hex Value | Description                                                                                                |
| --- | --------- | ---------------------------------------------------------------------------------------------------------- |
| 0   | `0001`    | Motor is energized.                                                                                        |
| 1   | `0002`    | Motor is moving.                                                                                           |
| 2   | `0004`    | Motor is moving in the forward direction.                                                                  |
| 3   | `0008`    | Motor is moving in the backward direction.                                                                 |
| 4   | `0010`    | The motor has reached the forward limit.                                                                   |
| 5   | `0020`    | The motor has reached the backward limit.                                                                  |
| 6   | `0040`    | The motor is in a disabled state.                                                                          |
| 7   | `0080`    | Homing is in progress.                                                                                     |
| 8   | `0100`    | The motor has been homed (the 'zero' datum is valid).                                                      |
| 9   | `0200`    | An error has occurred. The specific error code can be retrieved by sending a `gs` command again immediately. |

**Note**: Bits are combined. A status of `000B` (`0001` + `0002` + `0008`) means the motor is energized, moving, and in the backward direction.

## 4. Position Encoding/Decoding

The device position is a 32-bit unsigned integer that maps to a physical angle.

### 4.1. Formula

The conversion between a physical angle and the device's position value is given by:

`Position = P * (Angle / 360)`

Where:
-   `Position`: The 32-bit integer value sent to/from the device.
-   `Angle`: The desired angle in degrees.
-   `P`: A device-specific constant representing the number of motor "pulses per revolution".

The value of `P` for the **ELL14** rotator is **136,533**. This value may differ for other models.

### 4.2. Encoding Example (Angle to Hex)

To move an ELL14 rotator at address `0` to **90.0 degrees**:

1.  **Calculate Position**:
    `Position = 136533 * (90 / 360) = 34133.25`
    Round to the nearest integer: `34133`.

2.  **Convert to Hex**:
    `34133` in decimal is `8555` in hexadecimal.

3.  **Format as 8-char String**:
    Pad with leading zeros to create an 8-character string: `00008555`.

4.  **Construct Command**:
    The final command is `0ma00008555`.

### 4.3. Decoding Example (Hex to Angle)

If a `0PO00010E24` response is received from an ELL14:

1.  **Parse Hex String**:
    The position data is `00010E24`.

2.  **Convert to Decimal**:
    `10E24` in hexadecimal is `69156` in decimal.

3.  **Calculate Angle**:
    Rearrange the formula: `Angle = (Position / P) * 360`
    `Angle = (69156 / 136533) * 360 = 180.00...`
    The angle is **180.0 degrees**.

## 5. Error Codes and Handling

If the status response indicates an error (bit 9 is set), sending the `gs` command again will return an `ER` response with an error code.

| Error Code | Description               |
| ---------- | ------------------------- |
| `01`       | Communication time-out    |
| `02`       | Mechanical time-out       |
| `03`       | Command not understood    |
| `04`       | Parameter out of range    |
| `05`       | Module isolated           |
| `06`       | Module out of range       |
| `07`       | Homing error              |
| `08`       | Motor error               |
| `09`       | Internal error (firmware) |

**Example**:
-   Send: `0gs`
-   Receive: `0GS0201` (Error bit 9 is set)
-   Send: `0gs`
-   Receive: `0ER04` (Parameter out of range error)

## 6. Multidrop Addressing

-   Up to 16 devices can be connected on the same communication bus.
-   Each device must have a unique address from `0` to `F`.
-   The factory default address for all devices is `0`.
-   The address can be changed using the `ca` command (e.g., `0ca1` changes the address of device `0` to `1`). The new address is saved to non-volatile memory.

## 7. Example Command/Response Sequences

#### Example 1: Home Device '0'

1.  **Client sends Home command:**
    `0ho`
2.  **Device '0' starts moving and reports status:**
    `0GS0082` (Homing in progress, Motor is moving)
3.  *(Client waits for movement to stop)*
4.  **Device '0' reports final status:**
    `0GS0101` (Homed, Motor is energized)

#### Example 2: Move Device '1' to 180 degrees and Get Position

1.  **Client calculates position for 180° (for ELL14):**
    `Position = 136533 * (180/360) = 68266.5` -> `68267` -> `00010ABB`
2.  **Client sends Move Absolute command:**
    `1ma00010ABB`
3.  **Device '1' starts moving and reports status:**
    `1GS000B` (Energized, Moving, Backward direction)
4.  *(Client waits for movement to stop)*
5.  **Device '1' reports final status:**
    `1GS0101` (Homed, Motor is energized)
6.  **Client sends Get Position command:**
    `1gp`
7.  **Device '1' responds with its current position:**
    `1PO00010ABB`

## 8. Timing Requirements or Constraints

The manual specifies the following timing requirements:

-   Allow a delay of **100ms** after sending a command before expecting a response.
-   Allow a further delay of **100ms** after receiving a response before sending the next command.

This implies a minimum 200ms cycle time per command-response pair.

## 9. Implementation Notes for ELL14

-   **Pulses per revolution**: 136,533 (not 143,360)
-   **Position range**: 0 to 136,533 (0° to 360°)
-   **Address format**: Single hex digit (0-F), not just decimal
-   **Response validation**: Check address match and parse status codes
-   **Error handling**: Check GS status bit 9, then query GS again for ER code
-   **Timing**: Respect 100ms delays for reliable communication
