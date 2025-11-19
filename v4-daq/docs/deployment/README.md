# Rust DAQ V4 - Deployment Documentation

**Version**: 1.0
**Date**: 2025-11-17
**Architecture**: V4-Only (Clean Kameo Actor System)
**Status**: Production Ready

---

## Overview

This directory contains complete production deployment documentation for Rust DAQ V4, a fault-tolerant data acquisition system with 5 instrument actors managed by the Kameo actor framework.

### What is Rust DAQ V4?

Rust DAQ V4 is a production-grade system for coordinating multiple hardware instruments through message-passing actors:

- **5 Instrument Drivers**: SCPI, ESP300, PVCAM, Newport 1830-C, MaiTai
- **Actor-Based Architecture**: Fault-tolerant message-passing with Kameo framework
- **Flexible Storage**: HDF5 and Apache Arrow backends with compression
- **Production Ready**: 108/108 tests passing, 24-hour stability validated

### Instrument Summary

| Instrument | Type | Hardware | Purpose |
|-----------|------|----------|---------|
| **SCPI** | Generic | GPIB/USB/Ethernet | Universal SCPI instrument interface |
| **ESP300** | Motion Controller | Serial RS-232 | 3-axis XYZ stage positioning |
| **PVCAM** | Camera | USB/Ethernet | High-speed frame acquisition |
| **Newport 1830-C** | Power Meter | Serial RS-232 | Wavelength-calibrated power measurement |
| **MaiTai** | Tunable Laser | Serial RS-232 | 690-1040nm Ti:Sapphire laser control |

---

## Documentation Structure

### Quick Start

**New to Rust DAQ?** Start here:

1. **[PRODUCTION_DEPLOYMENT_GUIDE.md](PRODUCTION_DEPLOYMENT_GUIDE.md)** - 15 min read
   - System requirements
   - Step-by-step installation
   - Systemd service setup
   - Deployment verification

2. **[CONFIGURATION_REFERENCE.md](CONFIGURATION_REFERENCE.md)** - Reference
   - Complete TOML configuration schema
   - All 5 instrument types documented
   - Environment variable overrides
   - Example configurations

3. **[TROUBLESHOOTING.md](TROUBLESHOOTING.md)** - Problem solving
   - Common issues and solutions
   - Hardware connection problems
   - Performance optimization
   - Debug procedures

### Example Configuration

**[../config/config.example.v4.toml](../../config/config.example.v4.toml)**
- Complete working example
- All instruments enabled
- Detailed inline comments
- Copy and customize for your deployment

---

## Reading Guide

### For System Administrators (Deployment)

1. **System Requirements** → [PRODUCTION_DEPLOYMENT_GUIDE.md § System Requirements](PRODUCTION_DEPLOYMENT_GUIDE.md#system-requirements)
2. **Installation Steps** → [PRODUCTION_DEPLOYMENT_GUIDE.md § Installation Steps](PRODUCTION_DEPLOYMENT_GUIDE.md#installation-steps)
3. **Systemd Setup** → [PRODUCTION_DEPLOYMENT_GUIDE.md § Systemd Service Setup](PRODUCTION_DEPLOYMENT_GUIDE.md#systemd-service-setup)
4. **Monitoring** → [PRODUCTION_DEPLOYMENT_GUIDE.md § Monitoring and Logging](PRODUCTION_DEPLOYMENT_GUIDE.md#monitoring-and-logging)
5. **Verification** → [PRODUCTION_DEPLOYMENT_GUIDE.md § Deployment Verification](PRODUCTION_DEPLOYMENT_GUIDE.md#deployment-verification)

### For Hardware Engineers (Configuration)

1. **Your Specific Instrument** → [CONFIGURATION_REFERENCE.md § Instrument-Specific Configuration](CONFIGURATION_REFERENCE.md#instrument-specific-configuration)
2. **Example Config** → [config.example.v4.toml](../../config/config.example.v4.toml)
3. **Validation** → [CONFIGURATION_REFERENCE.md § Configuration Validation](CONFIGURATION_REFERENCE.md#configuration-validation)
4. **Your Scenario** → [CONFIGURATION_REFERENCE.md § Example Configurations](CONFIGURATION_REFERENCE.md#example-configurations)

### For Operators (Troubleshooting)

1. **What's Wrong?** → [TROUBLESHOOTING.md § Common Issues and Solutions](TROUBLESHOOTING.md#common-issues-and-solutions)
2. **Check Logs** → [TROUBLESHOOTING.md § Logging Configuration](TROUBLESHOOTING.md#logging-configuration)
3. **Hardware Problems?** → [TROUBLESHOOTING.md § Hardware Connection Problems](TROUBLESHOOTING.md#hardware-connection-problems)
4. **Performance Issues?** → [TROUBLESHOOTING.md § Performance Issues](TROUBLESHOOTING.md#performance-issues)
5. **Full Diagnostics** → [TROUBLESHOOTING.md § Diagnostics Procedures](TROUBLESHOOTING.md#diagnostics-procedures)

### For Developers (Integration)

1. **Architecture** → See main project README
2. **Configuration Loading** → [CONFIGURATION_REFERENCE.md § Configuration Overview](CONFIGURATION_REFERENCE.md#configuration-overview)
3. **Example Configs** → [CONFIGURATION_REFERENCE.md § Example Configurations](CONFIGURATION_REFERENCE.md#example-configurations)
4. **Actor Lifecycle** → [TROUBLESHOOTING.md § Actor Lifecycle Issues](TROUBLESHOOTING.md#actor-lifecycle-issues)

---

## Quick References

### Common Tasks

#### Deploy to Production
```bash
# 1. Follow deployment guide
docs/deployment/PRODUCTION_DEPLOYMENT_GUIDE.md

# 2. Create configuration
cp config/config.example.v4.toml /opt/rust-daq/config/config.v4.toml
nano /opt/rust-daq/config/config.v4.toml

# 3. Install systemd service
sudo cp docs/deployment/systemd-unit-file /etc/systemd/system/rust-daq.service
sudo systemctl daemon-reload
sudo systemctl enable rust-daq
sudo systemctl start rust-daq

# 4. Verify
sudo systemctl status rust-daq
```

#### Configure Instruments
```bash
# 1. Identify your instruments
ls /dev/ttyUSB*              # Serial devices (ESP300, MaiTai, Newport)
ping 192.168.1.100          # Network instruments (SCPI)
pvcam_list_cameras          # PVCAM camera name

# 2. Edit configuration
nano config/config.example.v4.toml

# 3. Validate
./target/release/rust-daq --validate-config config/config.example.v4.toml
```

#### Debug Issues
```bash
# 1. Check service status
sudo systemctl status rust-daq

# 2. View logs
sudo journalctl -u rust-daq -f

# 3. Enable debug logging
export RUST_LOG=debug
sudo systemctl set-environment RUST_LOG=debug
sudo systemctl restart rust-daq

# 4. Run diagnostics
See TROUBLESHOOTING.md § Diagnostics Procedures
```

#### Check Hardware
```bash
# SCPI via network
telnet 192.168.1.100 5025
*IDN?
quit

# Serial instruments
minicom -D /dev/ttyUSB0 -b 19200  # ESP300
minicom -D /dev/ttyUSB1 -b 115200 # MaiTai

# VISA resources
visainfo | grep ASRL  # Serial VISA
visainfo | grep TCPIP # Network VISA
```

---

## Key Sections by Document

### PRODUCTION_DEPLOYMENT_GUIDE.md

**Covers**:
- System requirements (hardware, OS, dependencies)
- Pre-deployment checklist
- Step-by-step installation (user setup, permissions, build)
- Systemd service creation and management
- Logging configuration
- Deployment verification procedures
- Production checklist and rollback procedures

**Read Time**: 30-45 minutes (complete)
**Reference**: 5 minutes (specific section)

### CONFIGURATION_REFERENCE.md

**Covers**:
- Configuration file format (TOML)
- Application configuration (name, logging)
- Actor system configuration (mailbox, timeouts)
- Storage configuration (backend, compression)
- Instrument configuration for all 5 types:
  - SCPI (generic VISA instruments)
  - ESP300 (motion controller)
  - PVCAM (camera)
  - Newport 1830-C (power meter)
  - MaiTai (tunable laser)
- Environment variable overrides
- Configuration validation
- Example configurations (basic, dev, staging, distributed)
- Performance tuning recommendations

**Read Time**: 45-60 minutes (complete)
**Reference**: 2-5 minutes (specific section)

### TROUBLESHOOTING.md

**Covers**:
- Common issues with solutions:
  - Service won't start
  - Actors fail to spawn
  - Hardware timeouts
  - Data loss/corruption
- Logging configuration and analysis
- Debug mode and manual testing
- Hardware connection problems (SCPI, serial, PVCAM)
- Actor lifecycle issues
- Performance optimization
- Data integrity procedures
- Complete diagnostics procedures

**Read Time**: 60 minutes (complete)
**Reference**: 5-10 minutes (specific issue)

### config/config.example.v4.toml

**Contains**:
- Complete working configuration
- All 5 instruments enabled
- Inline documentation for every option
- Common deployment scenarios (dev, high-throughput, low-memory)
- Notes and troubleshooting tips

**Use As**:
- Template for new deployments
- Reference for configuration syntax
- Example of all available options

---

## Instrument Hardware Details

### SCPI Instrument

**Supports**: Any SCPI-compliant instrument
- Power meters, multimeters, oscilloscopes, signal generators, spectrum analyzers, etc.

**Communication**:
- GPIB (via IEEE 488 interface)
- USB (USBTMC protocol)
- Ethernet (TCPIP via VISA)
- Serial (ASRL via VISA)

**Configuration**:
```toml
[[instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
[instruments.config]
resource = "TCPIP0::192.168.1.100::INSTR"  # VISA resource string
timeout_ms = 2000
```

---

### ESP300 Motion Controller

**Hardware**: Newport ESP300 multi-axis motion controller

**Specifications**:
- 3 axes (or fewer if configured)
- Baud: 19200 baud (fixed)
- Flow control: Hardware (RTS/CTS) required
- Terminator: `\r\n`

**Configuration**:
```toml
[[instruments]]
id = "esp300_stage"
type = "ESP300"
[instruments.config]
serial_port = "/dev/ttyUSB0"
axes = 3
```

**Supported Commands**:
- Absolute/relative positioning
- Velocity control
- Homing procedures
- Position feedback (continuous streaming)

---

### PVCAM Instrument

**Hardware**: Photometrics camera (Prime series)

**Specifications**:
- Supported models: PrimeBSI, PrimeSC, Prime95B, etc.
- Interface: USB or Ethernet
- Temperature control: Programmable setpoint
- ROI: Configurable region of interest

**Configuration**:
```toml
[[instruments]]
id = "pvcam_main"
type = "PVCAMInstrument"
[instruments.config]
camera_name = "PrimeBSI"
# temperature_setpoint = -20.0
# roi_width = 2048
```

**SDK Requirement**:
- Photometrics PVCAM SDK must be installed
- Available at: https://www.photometrics.com/support/software

---

### Newport 1830-C Power Meter

**Hardware**: Newport optical power meter (calibrated for specific wavelength)

**Specifications**:
- Baud: 9600 baud (typical for serial)
- Wavelength range: 190-1100 nm
- Calibrated: For specific wavelength (default 1550 nm)
- Units: dBm or mW

**Configuration**:
```toml
[[instruments]]
id = "newport_1830c"
type = "Newport1830C"
[instruments.config]
resource = "ASRL1::INSTR"
wavelength_nm = 1550.0  # C-band telecom standard
```

**Common Wavelengths**:
- 635 nm: Red HeNe laser
- 785 nm: YDLF laser
- 850 nm: Datacom multimode
- 1064 nm: Nd:YAG laser
- 1310 nm: Telecom singlemode
- 1550 nm: C-band (standard)

---

### MaiTai Tunable Laser

**Hardware**: Spectra-Physics MaiTai Ti:Sapphire laser

**Specifications**:
- Wavelength range: 690-1040 nm (tunable)
- Baud: 115200 baud (fixed)
- Flow control: None (disabled)
- Terminator: `\r\n`
- Safety: Requires laser safety officer approval

**Configuration**:
```toml
[[instruments]]
id = "maitai_laser"
type = "MaiTai"
[instruments.config]
serial_port = "/dev/ttyUSB1"
# shutter_open_on_startup = false  # Safe: closed
```

**Safety Requirements**:
- Laser Safety Officer must approve operation
- Shutter defaults to CLOSED (safe)
- Wavelength tuning: ~2 seconds per step
- Power ramp-down required before shutdown

**Typical Operations**:
- Wavelength tuning (690-1040 nm)
- Shutter control (open/close)
- Power measurement and monitoring
- Laser standby mode

---

## Deployment Checklist

### Pre-Deployment (1-2 days before)

- [ ] Review PRODUCTION_DEPLOYMENT_GUIDE.md
- [ ] Verify all 5 instruments available and accessible
- [ ] Test network connectivity to networked instruments
- [ ] Identify serial ports for serial instruments
- [ ] Verify VISA installation and resources
- [ ] Ensure 100+ GB disk space available
- [ ] Plan systemd service installation

### Day of Deployment

- [ ] Create `rustdaq` system user
- [ ] Configure serial port permissions (udev rules)
- [ ] Build or download rust-daq binary
- [ ] Copy configuration template
- [ ] Configure instruments for your hardware
- [ ] Validate configuration syntax
- [ ] Install systemd service
- [ ] Enable and start service
- [ ] Verify service is running

### Post-Deployment (24-48 hours)

- [ ] Monitor logs for errors
- [ ] Verify all 5 actors spawned successfully
- [ ] Test each instrument individually
- [ ] Verify data is being stored
- [ ] Check baseline performance metrics
- [ ] Document any changes made
- [ ] Get operator sign-off

---

## Support and Resources

### Documentation

- **Main Project**: See parent directory README.md
- **Architecture**: V4_ONLY_ARCHITECTURE_PLAN.md
- **Phase Completion**: PHASE_1E_COMPLETION_SUMMARY.md

### Getting Help

1. **Configuration Issues**: → [CONFIGURATION_REFERENCE.md](CONFIGURATION_REFERENCE.md)
2. **Deployment Issues**: → [PRODUCTION_DEPLOYMENT_GUIDE.md](PRODUCTION_DEPLOYMENT_GUIDE.md)
3. **Runtime Problems**: → [TROUBLESHOOTING.md](TROUBLESHOOTING.md)
4. **Instrument-Specific**: → See CONFIGURATION_REFERENCE.md instrument sections

### Reporting Issues

When reporting issues, include:
1. Configuration file (sanitized of sensitive info)
2. Relevant log excerpts (use `journalctl -u rust-daq`)
3. System information (`uname -a`, `free -h`, `df -h`)
4. Hardware information (`lsusb`, `ls /dev/ttyUSB*`, `visainfo`)
5. Steps to reproduce

See [TROUBLESHOOTING.md § Collecting Logs for Support](TROUBLESHOOTING.md#collecting-logs-for-support) for automated diagnostics.

---

## Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0 | 2025-11-17 | Initial production deployment documentation for V4-only architecture |

---

## Document Maintenance

**Maintained By**: Brian Squires
**Last Updated**: 2025-11-17
**Review Frequency**: Quarterly or after major updates

**Contributing**:
- Report issues via project issue tracker
- Suggest improvements via pull request
- Request clarification in documentation

---

## Quick Navigation

**Deployment**: [PRODUCTION_DEPLOYMENT_GUIDE.md](PRODUCTION_DEPLOYMENT_GUIDE.md)
**Configuration**: [CONFIGURATION_REFERENCE.md](CONFIGURATION_REFERENCE.md)
**Troubleshooting**: [TROUBLESHOOTING.md](TROUBLESHOOTING.md)
**Example Config**: [config.example.v4.toml](../../config/config.example.v4.toml)
