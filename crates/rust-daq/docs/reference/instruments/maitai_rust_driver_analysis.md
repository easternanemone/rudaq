# MaiTai Rust Driver Analysis - Serial I/O Verification

## Date: 2025-11-02

## Summary

The Rust MaiTai driver implementation uses **correct** bidirectional serial I/O that is equivalent to the successful bash file descriptor method. The driver should work properly with the flow control fix already in place.

## Serial I/O Architecture Analysis

### Rust Implementation (CORRECT)

**SerialAdapter** (src/adapters/serial.rs:10-20):
```rust
#[derive(Clone)]
pub struct SerialAdapter {
    port: Arc<Mutex<Box<dyn SerialPort>>>,
}
```

**serial_helper::send_command_async()** (src/instrument/serial_helper.rs:18-68):
```rust
pub async fn send_command_async(
    mut adapter: SerialAdapter,
    // ...
) -> Result<String> {
    // Write using adapter
    adapter.write(command_with_term.into_bytes()).await?;

    // Read using SAME adapter (same underlying SerialPort)
    loop {
        let mut buffer = Vec::new();
        let bytes_read = adapter.read(&mut buffer).await?;
        response.extend_from_slice(&buffer[..bytes_read]);

        if buffer[..bytes_read].contains(&delimiter) {
            break;
        }
    }

    Ok(response)
}
```

**Key Points:**
- Single `SerialPort` instance wrapped in `Arc<Mutex<>>`
- Both `write()` and `read()` operate on the **same** underlying file descriptor
- `Mutex` ensures sequential, non-interleaved operations
- This is **equivalent** to bash's `exec 3<>"$PORT"` method

### Comparison: Bash Methods

**SUCCESSFUL Method (File Descriptor):**
```bash
exec 3<>"$PORT"        # Open ONCE for bidirectional I/O
printf "*IDN?\r" >&3   # Write to FD 3
read -u 3 response     # Read from SAME FD 3
exec 3<&-              # Close FD 3
```

**Result**: Full MaiTai responses received
- `*IDN?` → `Spectra Physics,MaiTai,3227/51054/40856,0245-2.00.34 / CD00000019 / 214-00.004.057`
- `WAVELENGTH?` → `820nm`
- `POWER?` → `3.000W`
- `SHUTTER?` → `0`

**FAILED Method (Separate Redirections):**
```bash
echo "*IDN?\r" > "$PORT"              # Opens NEW file descriptor for write
response=$(dd if="$PORT" ...)         # Opens DIFFERENT file descriptor for read
```

**Result**: No responses or incomplete responses ('0')

### Why Rust Implementation is Equivalent to SUCCESS Case

| Aspect | Bash File Descriptor | Rust SerialAdapter |
|--------|---------------------|-------------------|
| **File descriptor** | Single FD (3) opened once | Single `SerialPort` in `Arc<Mutex<>>` |
| **Write operation** | `printf >&3` (FD 3) | `adapter.write()` (same port) |
| **Read operation** | `read -u 3` (FD 3) | `adapter.read()` (same port) |
| **State maintenance** | FD stays open between ops | `Arc<Mutex<>>` maintains state |
| **Synchronization** | Shell sequential execution | `Mutex` sequential locking |

## Flow Control Validation

**MaiTai Driver Configuration** (src/instrument/maitai.rs:120-124):
```rust
let port = serialport::new(port_name, baud_rate)
    .timeout(std::time::Duration::from_millis(500))
    .flow_control(serialport::FlowControl::Software)  // ✅ CORRECT (XON/XOFF)
    .open()
    .with_context(|| format!("Failed to open serial port '{}' for MaiTai", port_name))?;
```

**Confirmed Settings:**
- Port: `/dev/ttyUSB5` (Silicon Labs CP2102 USB-to-UART)
- Baud rate: 9600
- Data bits: 8, No parity, 1 stop bit (8N1)
- Flow control: **SOFTWARE (XON/XOFF)** ✅ Correct per manual
- Terminator: CR (`\r`) - line 68 of maitai.rs

## Comparison with Other Instruments

### Newport 1830C (src/instrument/newport_1830c.rs:190)
```rust
let port = serialport::new(port_name, baud_rate)
    .timeout(std::time::Duration::from_millis(100))
    .open()  // No flow control - uses default (None)
```
- Uses same `SerialAdapter` pattern
- No flow control required for Newport
- Successfully communicating

### Elliptec ELL14 (src/instrument/elliptec.rs:152-156)
```rust
let port = serialport::new(port_name, baud_rate)
    .timeout(std::time::Duration::from_millis(100))
    .flow_control(serialport::FlowControl::Hardware)  // RTS/CTS for RS-485
    .open()
```
- Uses same `SerialAdapter` pattern
- Hardware flow control for RS-485 multidrop
- Different protocol from MaiTai

## Conclusion

**The Rust MaiTai driver implementation is CORRECT and should work properly.**

### Why the driver should work:

1. ✅ **Correct I/O method**: Uses single `SerialPort` instance for bidirectional communication (equivalent to bash file descriptor method)
2. ✅ **Correct flow control**: `FlowControl::Software` (XON/XOFF) per manual specification
3. ✅ **Correct terminator**: CR (`\r`) validated via bash testing
4. ✅ **Correct port settings**: 9600 baud, 8N1
5. ✅ **Correct port**: `/dev/ttyUSB5` confirmed via bash testing

### Previous communication issues were likely due to:

1. **Flow control error** (now fixed) - was using `Hardware` instead of `Software`
2. **Bash testing artifacts** - initial '0' response and subsequent "no responses" were from bash tests using the FAILED I/O method (separate redirections), not from the Rust driver

### Recommended Next Steps

1. **Test Rust driver** with the DAQ application:
   ```bash
   cd /Users/briansquires/code/rust-daq
   cargo run --features instrument_serial
   ```

2. **Verify MaiTai connection** in application logs:
   - Should see: "Connecting to MaiTai laser: maitai"
   - Should see: "MaiTai identity: Spectra Physics,MaiTai,..."
   - Should see: "MaiTai laser 'maitai' connected successfully"

3. **Monitor data stream** to confirm continuous polling:
   - Wavelength readings (820nm expected)
   - Power readings (~3.0W expected)
   - Shutter state (0 expected)

4. **If issues occur**, enable debug logging:
   ```bash
   RUST_LOG=debug cargo run --features instrument_serial
   ```

## Technical Notes

### Serial Port Locking in Rust

The `serialport` crate properly manages file descriptors:
- Opens `/dev/ttyUSB5` once during `serialport::new().open()`
- Returns `Box<dyn SerialPort>` wrapping the underlying POSIX file descriptor
- All `read()` and `write()` operations use the same FD
- FD is closed when the `SerialPort` is dropped

This is fundamentally different from bash's `> "$PORT"` which opens a new FD for each redirection.

### Why Arc<Mutex<>> is Necessary

The `Arc<Mutex<>>` wrapper serves two purposes:
1. **Shared ownership**: Allows cloning the `SerialAdapter` for use in multiple contexts (polling loop, command handling)
2. **Thread safety**: Prevents concurrent access to the serial port from multiple async tasks

Without the `Mutex`, concurrent `read()` and `write()` operations could interleave, corrupting the serial communication.

## Files Verified

- ✅ `/Users/briansquires/code/rust-daq/src/adapters/serial.rs` - SerialAdapter implementation
- ✅ `/Users/briansquires/code/rust-daq/src/instrument/serial_helper.rs` - Command/response helper
- ✅ `/Users/briansquires/code/rust-daq/src/instrument/maitai.rs` - MaiTai driver with flow control fix
- ✅ `/Users/briansquires/code/rust-daq/config/default.toml` - MaiTai configuration

## Validation Status

- ✅ Flow control corrected to SOFTWARE (XON/XOFF)
- ✅ Bash file descriptor method validated MaiTai communication
- ✅ Rust serial I/O architecture verified as equivalent to working bash method
- ⏳ **Pending**: End-to-end Rust driver test with DAQ application
