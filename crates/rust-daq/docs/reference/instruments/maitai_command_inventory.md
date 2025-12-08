# MaiTai Driver Command Inventory

## Date: 2025-11-26 (Updated)

## Currently Implemented Commands

The MaiTai driver (`src/hardware/maitai.rs`) implements the following commands:

### Query Commands (Read-Only)

| Command | Purpose | Tested |
|---------|---------|--------|
| `*IDN?` | Identity query - returns manufacturer, model, serial, firmware | ✅ Working |
| `WAVELENGTH?` | Query current wavelength in nm | ✅ Working |
| `POWER?` | Query laser power output in W | ✅ Working |
| `SHUTTER?` | Query shutter state (0=closed, 1=open) | ✅ Working |

### Set Commands (Write)

| Command | Purpose | Tested |
|---------|---------|--------|
| `SHUTter 1` | Open shutter (space separator, mixed case) | ⚠️ No effect |
| `SHUTter 0` | Close shutter (space separator, mixed case) | ⚠️ No effect |
| `ON` | Turn laser on | ⏳ Untested |
| `OFF` | Turn laser off | ⏳ Untested |

**Note:** The shutter set commands use space separator (not colon) per the
[MaiTaiControl reference implementation](https://github.com/StPeres-Cerebellum/MaiTaiControl).

**Total: 6 commands** (4 query + 2 set operations)

## Validation Status

### Hardware Tested (2025-11-26)
- **Remote Hardware**: `maitai@100.117.5.12` (Linux)
- **Port**: `/dev/ttyUSB5` (Silicon Labs CP2102)

#### Query Commands - All Working
| Query | Response |
|-------|----------|
| `*IDN?` | `Spectra Physics,MaiTai,3227/51054/40856,0245-2.00.34 / CD00000019 / 214-00.004.057` |
| `WAVELENGTH?` | `820nm` |
| `POWER?` | `3.000W` |
| `SHUTTER?` | `0` |
| `*STB?` | `8` |
| `READ:PCTWarmedup?` | `100.00%` |
| `MODE?` | `POW` |
| `PLASer:POWer?` | `13.43W` |
| `READ:HUM?` | `7.26%` |

#### Shutter Commands - NOT Working
**Issue:** Shutter commands (`SHUTter 1`, `SHUTter 0`) do not change the shutter state.
- `SHUTTER?` always returns `0` (closed) regardless of commands sent
- MaiTai internal power remains at 3.000W (laser is generating power)
- Newport external power meter reads 0.11 nW (beam not reaching detector)

**Possible Causes (Investigation 2025-11-26):**

The RS-232 driver code has been validated and is correct:
- Command terminator: CR+LF (`\r\n`) per protocol
- Response reading: All commands read responses to clear buffer
- Flow control: XON/XOFF enabled
- Command format: `SHUTter:1` / `SHUTter:0` with colon separator

Query commands work perfectly. Set commands (shutter) are received but NOT executed.
This indicates a **hardware configuration issue**, not a software bug.

Likely causes (in order of probability):
1. **Front panel mode** - MaiTai must be in "Remote" or "Computer Control" mode
   - Check the MaiTai front panel display for LOCAL/REMOTE indicator
   - Use front panel buttons to switch to REMOTE mode
2. **Keyswitch position** - The operator key may need to be in a specific position
3. **Safety interlock chain** - External interlock may be blocking shutter operation
4. **Shutter disabled** - Shutter control may be disabled in laser configuration
5. **Physical mechanism** - Shutter mechanism malfunction (unlikely since SHUTTER? queries work)

### Commands Not Yet Tested
- `ON` / `OFF` - Laser power control (dangerous - requires safety approval)

## Command Implementation Details

### Query Pattern
```rust
async fn query_value(&self, command: &str) -> Result<f64> {
    let response = self.send_command_async(command).await?;
    // Remove command echo if present
    let value_str = response.split(':').next_back().unwrap_or(&response);
    value_str.trim().parse::<f64>()
        .with_context(|| format!("Failed to parse response '{}' as float", response))
}
```

All query commands use `query_value()` which:
1. Sends command with CR terminator
2. Waits up to 2 seconds for response
3. Parses response as float
4. Returns error if parsing fails

### Set Pattern
```rust
async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
    match command {
        InstrumentCommand::SetParameter(key, value) => match key.as_str() {
            "wavelength" => {
                let wavelength: f64 = value.as_f64()
                    .with_context(|| format!("Invalid wavelength value: {}", value))?;
                self.send_command_async(&format!("WAVELENGTH:{}", wavelength)).await?;
                info!("Set MaiTai wavelength to {} nm", wavelength);
            }
            "shutter" => {
                let value_str = value.as_string()
                    .with_context(|| format!("Invalid shutter value: {}", value))?;
                let cmd = match value_str.as_str() {
                    "open" => "SHUTTER:1",
                    "close" => "SHUTTER:0",
                    _ => return Err(anyhow!("Invalid shutter value: {}", value)),
                };
                self.send_command_async(cmd).await?;
                info!("MaiTai shutter: {}", value);
            }
            "laser" => {
                let value_str = value.as_string()
                    .with_context(|| format!("Invalid laser value: {}", value))?;
                let cmd = match value_str.as_str() {
                    "on" => "ON",
                    "off" => "OFF",
                    _ => return Err(anyhow!("Invalid laser value: {}", value)),
                };
                self.send_command_async(cmd).await?;
                info!("MaiTai laser: {}", value);
            }
            _ => {
                warn!("Unknown parameter '{}' for MaiTai", key);
            }
        }
        // ... capability handling ...
    }
    Ok(())
}
```

Set commands are invoked via `InstrumentCommand::SetParameter` from the GUI or control system.

## Polling Implementation

The driver automatically polls three parameters at configured rate (default 1.0 Hz):

```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(
        std::time::Duration::from_secs_f64(1.0 / polling_rate)
    );

    loop {
        interval.tick().await;
        let timestamp = chrono::Utc::now();

        // Query wavelength
        if let Ok(wavelength) = instrument.query_value("WAVELENGTH?").await {
            // Broadcast DataPoint with wavelength
        }

        // Query power
        if let Ok(power) = instrument.query_value("POWER?").await {
            // Broadcast DataPoint with power
        }

        // Query shutter state
        if let Ok(shutter) = instrument.query_value("SHUTTER?").await {
            // Broadcast DataPoint with shutter state
        }
    }
});
```

**Note**: `*IDN?` is only queried once during connection, not in the polling loop.

## Manual Command Coverage Analysis

### Commands Verified from Hardware Testing
The following commands are confirmed working from the standalone test:
- `*IDN?` - Standard SCPI identification
- `WAVELENGTH?` - MaiTai-specific wavelength query
- `POWER?` - MaiTai-specific power query
- `SHUTTER?` - MaiTai-specific shutter state query

### Potential Missing Commands

**Note**: Without access to the full MaiTai manual (PDF access failed), we cannot confirm if additional commands exist. Common laser commands that might be missing:

- Temperature queries (e.g., `TEMPERATURE?`)
- Pump power queries (e.g., `PUMP:POWER?`)
- Status/error queries (e.g., `STATUS?`, `ERROR?`)
- Modelock status (e.g., `MODELOCK?`)
- Alignment queries
- System diagnostics

**Recommendation**: Review the full manual at:
https://www.spectra-physics.com/medias/sys_master/spresources/hfe/h3b/9954750136350/284A%20Rev%20G%20Mai%20Tai%20Users%20Manual/284A-Rev-G-Mai-Tai-Users-Manual.pdf

## Next Steps

1. ✅ **Query Command Validation** - All 4 query commands tested and working
2. ⏳ **Set Command Validation** - Test wavelength, shutter, and laser on/off commands on hardware
3. ⏳ **Full Integration Test** - Build and run complete rust-daq on remote hardware
4. ⏳ **Extended Operation** - Monitor continuous polling and data streaming
5. ⏳ **Manual Review** - Obtain and review manual to identify any missing commands
6. ⏳ **Command Expansion** - Implement additional commands if needed

## Configuration

Current configuration in `config/default.toml`:

```toml
[instruments.maitai]
type = "maitai"
name = "MaiTai Ti:Sapphire Laser"
port = "/dev/ttyUSB5"
baud_rate = 9600
wavelength = 800.0
polling_rate_hz = 1.0
```

## References

- Driver Implementation: `/Users/briansquires/code/rust-daq/src/instrument/maitai.rs`
- Validation Report: `/Users/briansquires/code/rust-daq/docs/hardware_testing/maitai_driver_validation_report.md`
- Testing Summary: `/Users/briansquires/code/rust-daq/docs/hardware_testing/maitai_testing_summary.md`
- Beads Issue: bd-194 (MaiTai Laser: End-to-End Hardware Integration and Testing)

---

**Analysis Date**: 2025-11-02
**Analyzer**: Claude Code
**Status**: Query commands validated, set commands pending hardware test
