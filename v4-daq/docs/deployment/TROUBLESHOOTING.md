# Troubleshooting Guide - Rust DAQ V4

**Version**: 1.0
**Date**: 2025-11-17
**For**: Production deployments

---

## Table of Contents

1. [Common Issues and Solutions](#common-issues-and-solutions)
2. [Logging Configuration](#logging-configuration)
3. [Debug Mode](#debug-mode)
4. [Hardware Connection Problems](#hardware-connection-problems)
5. [Actor Lifecycle Issues](#actor-lifecycle-issues)
6. [Performance Issues](#performance-issues)
7. [Data Integrity Issues](#data-integrity-issues)
8. [Diagnostics Procedures](#diagnostics-procedures)

---

## Common Issues and Solutions

### Issue 1: Service Fails to Start

**Symptoms**:
```
systemctl status rust-daq
● rust-daq.service - Rust DAQ V4
   Loaded: loaded
   Active: failed
```

**Causes and Solutions**:

#### A. Configuration File Not Found

**Error in logs**:
```
Error: Configuration file not found: /opt/rust-daq/config/config.v4.toml
```

**Solution**:
```bash
# Verify configuration file exists
ls -la /opt/rust-daq/config/config.v4.toml

# If missing, copy from template
sudo cp config/config.example.v4.toml /opt/rust-daq/config/config.v4.toml
sudo chown rustdaq:rustdaq /opt/rust-daq/config/config.v4.toml

# Check file permissions (must be readable by rustdaq user)
sudo -u rustdaq test -r /opt/rust-daq/config/config.v4.toml && echo "Readable" || echo "Not readable"
```

#### B. Invalid Configuration Syntax

**Error in logs**:
```
Error: Parse error at line 42: expected value, found newline
```

**Solution**:
```bash
# Validate configuration
/opt/rust-daq/bin/rust-daq-v4 --validate-config /opt/rust-daq/config/config.v4.toml

# Check TOML syntax (use online validator for complex configs)
# Common mistakes:
# - Using : instead of = for assignment
# - Missing quotes around strings
# - Incorrect nesting (spaces matter in TOML)

# Example: WRONG
[instruments.config]
resource: "TCPIP0::192.168.1.100::INSTR"  # Wrong: uses :

# Example: CORRECT
[instruments.config]
resource = "TCPIP0::192.168.1.100::INSTR"  # Correct: uses =
```

#### C. Permission Denied

**Error in logs**:
```
Error: Permission denied: /var/lib/rust-daq/data
```

**Solution**:
```bash
# Check directory ownership
ls -la /var/lib/rust-daq/

# Should be owned by rustdaq:rustdaq with 750/755 permissions
sudo chown -R rustdaq:rustdaq /var/lib/rust-daq
sudo chmod 750 /var/lib/rust-daq
sudo chmod 755 /var/lib/rust-daq/data

# Verify rustdaq user can access
sudo -u rustdaq test -w /var/lib/rust-daq/data && echo "Writable" || echo "Not writable"
```

#### D. Binary Not Found

**Error in logs**:
```
/opt/rust-daq/bin/rust-daq-v4: No such file or directory
```

**Solution**:
```bash
# Verify binary exists and is executable
ls -la /opt/rust-daq/bin/rust-daq-v4

# Should have rwx------ or rwxr-x--- permissions
# If missing, rebuild or download
cargo build --release --features instrument_serial,storage_hdf5
sudo cp target/release/rust-daq /opt/rust-daq/bin/rust-daq-v4
sudo chmod 755 /opt/rust-daq/bin/rust-daq-v4
```

---

### Issue 2: Actors Fail to Spawn

**Symptoms**:
```
ERROR: Failed to spawn SCPI actor: timeout
ERROR: Actor 'scpi_meter' did not respond within 5000ms
```

**Causes and Solutions**:

#### A. Hardware Not Connected

**Error in logs**:
```
ERROR: SCPI actor failed to initialize: device not found
VISA Error: (1073676290) Timeout occurred
```

**Solution**:
```bash
# Check hardware connection
ping 192.168.1.100  # For network instruments

# Verify VISA resources
visainfo | grep "TCPIP0"

# For serial instruments, check ports
ls -la /dev/ttyUSB*

# Reset hardware:
# Power cycle instrument or
sudo systemctl restart udev
```

#### B. Serial Port In Use

**Error in logs**:
```
ERROR: ESP300 failed to initialize: device busy
Serial port /dev/ttyUSB0 already in use
```

**Solution**:
```bash
# Find what has the port open
lsof /dev/ttyUSB0

# Kill blocking process (be careful!)
sudo kill -9 <PID>

# Or, find the correct port
ls /dev/ttyUSB*
ls /dev/ttyACM*

# Verify no multiple processes are running
ps aux | grep rust-daq

# Update configuration with correct port
# Edit /opt/rust-daq/config/config.v4.toml
# [[instruments]]
# config.serial_port = "/dev/ttyUSB0"  # Correct port
```

#### C. VISA Not Installed

**Error in logs**:
```
ERROR: VISA session initialization failed
Could not initialize VISA: library not found
```

**Solution**:
```bash
# Check VISA installation
which visainfo
visainfo  # Should list resources

# If not installed:
# Ubuntu/Debian: Install National Instruments VISA or Keysight IO Libraries
# Follow instructions at: https://www.ni.com/en-us/support/downloads/drivers/download.ni-visa.html

# Verify shared libraries
ldconfig -p | grep visa

# If using custom VISA path, set environment variable
export LD_LIBRARY_PATH="/path/to/visa/lib:$LD_LIBRARY_PATH"
```

#### D. Timeout Too Short

**Error in logs**:
```
WARN: Actor spawn timeout reached (5000ms)
WARN: Hardware initialization may not be complete
```

**Solution**:
```bash
# Increase timeout in configuration
[actors]
spawn_timeout_ms = 10000  # Increase from 5000 to 10000

# Or via environment variable
export RUST_DAQ_ACTORS_SPAWN_TIMEOUT_MS=10000

# Restart service
sudo systemctl restart rust-daq

# Check if initialization completes with more time
sudo journalctl -u rust-daq -f
```

---

### Issue 3: Hardware Timeouts During Operation

**Symptoms**:
```
WARN: Command timeout: SCPI query took 5000ms, exceeded limit
ERROR: ESP300 motion timeout: move operation stalled
```

**Causes and Solutions**:

#### A. Network Congestion (SCPI/Newport)

**Diagnosis**:
```bash
# Test network latency
ping -c 10 192.168.1.100 | grep rtt

# Should be <10ms for LAN, <50ms for WAN
# If higher, network is congested

# Check network load
iftop -i eth0
```

**Solution**:
```bash
# Use dedicated network for instruments
# Or, increase timeout for specific instrument

[[instruments]]
id = "scpi_meter"
[instruments.config]
resource = "TCPIP0::192.168.1.100::INSTR"
timeout_ms = 5000  # Increase if needed (default: 2000)

# Reduce network traffic:
# - Disable other network services
# - Use 1GbE or faster
# - Separate VLAN for instruments
```

#### B. Serial Port Flow Control Issues

**Symptoms**:
```
WARN: Serial port stalled (RTS/CTS flow control issue)
ERROR: ESP300 command response timeout after 2000ms
```

**Solution**:
```bash
# Check hardware flow control settings
# For ESP300: RTS/CTS must be enabled

# Verify serial port settings:
stty -F /dev/ttyUSB0 -a

# Output should show:
# crtscts (hardware flow control enabled)
# speed 19200 baud (correct baud)

# If not set correctly, may be hardware issue
# Try different USB-serial adapter

# Or, temporarily disable flow control (not recommended):
# Edit config:
[instruments.config]
serial_port = "/dev/ttyUSB0"
flow_control = false  # NOT RECOMMENDED - may lose data
```

#### C. Hardware is Slow to Respond

**Symptoms**:
```
WARN: Instrument response delay: 1500ms (expected <200ms)
```

**Solution**:
```bash
# Test instrument directly:
# For SCPI:
telnet 192.168.1.100 5025
*IDN?  # Should respond quickly

# For serial:
minicom -D /dev/ttyUSB0 -b 19200
id  # Or appropriate command

# If slow:
# 1. Check instrument is not processing previous command
# 2. Verify instrument has power and is properly initialized
# 3. Check instrument firmware for known issues
# 4. Consider upgrading firmware

# Increase timeout as temporary workaround:
timeout_ms = 5000
```

---

### Issue 4: Data Loss or Corruption

**Symptoms**:
```
ERROR: Data integrity check failed: CRC mismatch
WARN: Frames dropped: 5/1000 (expected <1/10000)
```

**Causes and Solutions**:

#### A. Insufficient Disk Space

**Diagnosis**:
```bash
# Check disk space
df -h /var/lib/rust-daq/data

# Should have >10% free space
# If <5% free, disk full may cause data loss
```

**Solution**:
```bash
# Clean old data files
find /var/lib/rust-daq/data -type f -mtime +30 -delete  # Remove >30 days old

# Or, expand storage
# Mount new disk at /var/lib/rust-daq/data
# Migrate existing data

# Monitor disk space
watch -n 60 'df -h /var/lib/rust-daq/data'

# Set up alerts
# If <10% free: send warning
# If <5% free: stop acquisition
```

#### B. Corrupted Storage File

**Symptoms**:
```
ERROR: HDF5 file corrupted: invalid magic number
ERROR: Arrow file parsing failed: checksum mismatch
```

**Solution**:
```bash
# Verify file integrity
file /var/lib/rust-daq/data/*.h5  # Should identify as HDF5

# Check with HDF5 tools (if installed)
h5dump /var/lib/rust-daq/data/data_001.h5

# If corrupted:
# 1. Check for incomplete writes (file size vs expected)
# 2. Check system logs for disk errors: dmesg | grep -i error
# 3. Restore from backup if available

# Prevent future corruption:
# - Enable auto-flush more frequently
[storage]
auto_flush_interval_secs = 10  # Flush every 10 seconds

# - Graceful shutdown
sudo systemctl stop rust-daq  # Wait for clean shutdown
```

#### C. Buffer Overflow on High-Speed Acquisition

**Symptoms**:
```
WARN: Mailbox full for PVCAM actor: dropped 10 frames
ERROR: Frame acquisition rate (5000 fps) exceeds processing rate (4000 fps)
```

**Solution**:
```bash
# Increase actor mailbox capacity
[actors]
default_mailbox_capacity = 500  # Increase from default 100

# Or, optimize frame processing:
# 1. Use faster storage backend (Arrow > HDF5 for speed)
[storage]
default_backend = "arrow"
compression_level = 0  # No compression

# 2. Increase frame timeout
[instruments.config]
frame_timeout_ms = 10000  # More time to process

# 3. Reduce acquisition rate if possible
# 4. Use faster disks (SSD > HDD)

# 5. Check system load
top
# Reduce other system tasks
```

---

## Logging Configuration

### Log Levels

| Level | When to Use | Example |
|-------|-----------|---------|
| `error` | System failures, cannot recover | "Actor crashed", "Hardware error" |
| `warn` | Degraded operation, may recover | "Timeout", "Retry needed" |
| `info` | Normal operations | "Actor started", "Measurement complete" |
| `debug` | Troubleshooting | "Command sent: *IDN?", "Response: HP" |
| `trace` | Deep diagnostics | "Message queued", "Buffer details" |

### Enable Debug Logging

**Temporarily** (for current session):
```bash
# Via environment variable
export RUST_LOG=debug
sudo -E systemctl restart rust-daq

# Or, via systemd
sudo systemctl set-environment RUST_LOG=debug
sudo systemctl restart rust-daq

# View output
sudo journalctl -u rust-daq -f
```

**Permanently** (in configuration):
```toml
[application]
log_level = "debug"
```

**Selectively** (only specific modules):
```bash
export RUST_LOG=rust_daq::actors::scpi=debug,rust_daq::hardware::visa_adapter=debug

# This logs only SCPI and VISA adapter modules in debug
```

### Capture Detailed Logs

```bash
# Save logs to file
sudo journalctl -u rust-daq > rust-daq-logs.txt

# Save with full timestamps
sudo journalctl -u rust-daq -o short-iso > rust-daq-logs.txt

# Save with extended metadata
sudo journalctl -u rust-daq -o json-pretty > rust-daq-logs.json

# Continuously tail logs
sudo journalctl -u rust-daq -f

# Show last 1000 lines
sudo journalctl -u rust-daq -n 1000
```

### Log Analysis

**Find errors**:
```bash
sudo journalctl -u rust-daq | grep -i error
sudo journalctl -u rust-daq | grep -i "ERROR\|WARN"
```

**Find specific actor errors**:
```bash
sudo journalctl -u rust-daq | grep -i "scpi_meter"
sudo journalctl -u rust-daq | grep -i "esp300"
```

**Timeline analysis**:
```bash
# Show entries from specific time range
sudo journalctl -u rust-daq --since "2025-11-17 10:00:00" --until "2025-11-17 11:00:00"

# Show entries from last 1 hour
sudo journalctl -u rust-daq --since "1 hour ago"

# Show entries from last 10 minutes
sudo journalctl -u rust-daq --since "10 minutes ago" -f
```

---

## Debug Mode

### Enable Full Tracing

For maximum diagnostic information:

```bash
# Enable all tracing
export RUST_LOG=trace
export RUST_BACKTRACE=full

# Start in foreground
/opt/rust-daq/bin/rust-daq-v4 --config /opt/rust-daq/config/config.v4.toml

# Or via service
sudo systemctl set-environment RUST_LOG=trace
sudo systemctl set-environment RUST_BACKTRACE=full
sudo systemctl restart rust-daq
sudo journalctl -u rust-daq -f
```

### Test Individual Actors

**Test SCPI without full system**:
```bash
# Use configuration that disables other actors
[[instruments]]
id = "scpi_meter"
type = "ScpiInstrument"
enabled = true

[[instruments]]
id = "esp300"
enabled = false  # Disable

[[instruments]]
id = "pvcam"
enabled = false  # Disable

[[instruments]]
id = "newport"
enabled = false  # Disable

[[instruments]]
id = "maitai"
enabled = false  # Disable
```

### Manual Hardware Testing

```bash
# Test SCPI instrument directly
telnet 192.168.1.100 5025
*IDN?
MEAS:VOLT:DC?
quit

# Test serial instruments
minicom -D /dev/ttyUSB0 -b 19200  # ESP300
minicom -D /dev/ttyUSB1 -b 115200  # MaiTai
minicom -D /dev/ttyACM0 -b 9600  # Newport

# Commands:
# ESP300: "id" -> should echo back
# MaiTai: "LASER?" -> should respond with wavelength
# Newport: "*IDN?" -> should respond with model
```

---

## Hardware Connection Problems

### SCPI Instrument Issues

**Problem**: Cannot connect to network instrument

**Diagnostics**:
```bash
# 1. Test network connectivity
ping 192.168.1.100
# Should respond, latency <10ms

# 2. Test VISA resource discovery
visainfo | grep "TCPIP0"
# Should show the resource

# 3. Test direct TCP connection
telnet 192.168.1.100 5025
# Should connect
*IDN?
# Should respond with instrument identity

# 4. Check network configuration
ifconfig | grep -A2 inet
# Should show ethernet connection on same subnet

# 5. Check firewall
sudo iptables -L | grep 5025
# Should allow TCP 5025 if using default VISA port
```

**Solutions**:
1. Verify IP address: `ifconfig` on instrument (check display or manual)
2. Verify subnet: both host and instrument on same network
3. Check physical cables and switches
4. Restart network interface: `sudo systemctl restart networking`
5. Disable firewall temporarily for testing: `sudo systemctl stop ufw`

### Serial Port Issues (ESP300, MaiTai, Newport)

**Problem**: Serial port not found or in use

**Diagnostics**:
```bash
# 1. List serial ports
ls -la /dev/tty*
ls -la /dev/ttyUSB*  # USB adapters
ls -la /dev/ttyACM*  # ACM devices

# 2. Check USB devices
lsusb
# Should show connected adapters

# 3. Get device details
udevadm info /dev/ttyUSB0

# 4. Check if port is in use
lsof /dev/ttyUSB0
# If output, another process is using it

# 5. Check permissions
ls -la /dev/ttyUSB0
# Should have crw-rw---- or similar (readable by group dialout)

# 6. Test basic communication
echo "id" > /dev/ttyUSB0
cat /dev/ttyUSB0
# Should see response from device
```

**Solutions**:

1. **Fix permissions**:
   ```bash
   sudo usermod -a -G dialout rustdaq
   sudo systemctl restart rust-daq
   ```

2. **Use persistent naming** (udev rules):
   ```bash
   # Create rule
   sudo tee /etc/udev/rules.d/99-rust-daq.rules > /dev/null <<'EOF'
   SUBSYSTEMS=="usb", ATTRS{idVendor}=="0403", ATTRS{idProduct}=="6001", SYMLINK+="ttyESP300"
   EOF

   # Reload
   sudo udevadm control --reload-rules
   sudo udevadm trigger
   ```

3. **Find correct port**:
   ```bash
   for port in /dev/ttyUSB*; do
     echo "Testing $port..."
     echo "id" > "$port" 2>/dev/null
   done
   ```

4. **Check device drivers** (if using USB-serial adapter):
   ```bash
   # For FTDI (most common):
   sudo apt-get install ftdi-eeprom

   # For Prolific:
   sudo apt-get install pl2303
   ```

### PVCAM Camera Issues

**Problem**: Camera not detected

**Diagnostics**:
```bash
# 1. Check if PVCAM SDK is installed
ls -la /usr/local/lib/libpvcam.so

# 2. List available cameras (PVCAM SDK tool)
pvcam_list_cameras
# Should show camera name like "PrimeBSI"

# 3. Check USB connection (if USB camera)
lsusb | grep -i photometrics
# Should show Photometrics device

# 4. Verify environment
echo $PVCAM_SDK
# Should point to SDK installation

# 5. Test SDK directly (C++ code or provided tools)
# Ensure camera responds to PVCAM commands
```

**Solutions**:

1. **Verify SDK installation**:
   ```bash
   # Install from Photometrics
   # https://www.photometrics.com/support/software

   # Verify installation
   ls -la /usr/local/lib/libpvcam*
   ```

2. **Check camera power**:
   - Camera should have LED indicator (green = powered)
   - Try power-cycling: unplug/replug

3. **Update configuration with correct camera name**:
   ```toml
   [[instruments]]
   id = "pvcam_main"
   [instruments.config]
   camera_name = "PrimeBSI"  # Verify this exact name
   ```

4. **Update device firmware** (if available)

---

## Actor Lifecycle Issues

### Actor Crash During Operation

**Symptoms**:
```
ERROR: Actor 'scpi_meter' panicked: index out of bounds
WARN: Actor 'scpi_meter' stopped unexpectedly
INFO: Restarting actor 'scpi_meter' (attempt 1/3)
```

**Solution**:

1. **Check logs for panic details**:
   ```bash
   sudo journalctl -u rust-daq | grep -A 5 "panicked"
   ```

2. **Increase logging for full backtrace**:
   ```bash
   export RUST_BACKTRACE=full
   sudo systemctl set-environment RUST_BACKTRACE=full
   sudo systemctl restart rust-daq
   ```

3. **Common causes**:
   - Invalid hardware response → verify hardware working
   - Out of memory → increase system RAM or reduce mailbox capacity
   - Race condition → report with full logs

4. **Workaround**:
   - Disable problematic actor temporarily
   - Restart service
   - Investigate and fix

### Graceful Shutdown Hangs

**Symptoms**:
```
sudo systemctl stop rust-daq
# Waits 30 seconds then force-kills
# Logs show actor didn't shutdown cleanly
```

**Solution**:

1. **Increase shutdown timeout**:
   ```bash
   # In systemd service:
   [Service]
   TimeoutStopSec=60  # Increase from default 30s
   ```

2. **Check for hanging hardware operations**:
   ```bash
   # During shutdown, which actor is stuck?
   sudo journalctl -u rust-daq -f
   # Look for last successful actor shutdown
   ```

3. **Verify actor shutdown code**:
   - Each actor should implement proper shutdown
   - Hardware should release resources
   - Timeouts should prevent infinite hangs

4. **Manual cleanup if service won't stop**:
   ```bash
   sudo pkill -9 rust-daq  # Force kill (last resort)
   sudo pkill -9 -f /opt/rust-daq/bin/rust-daq-v4
   ```

---

## Performance Issues

### High CPU Usage

**Symptoms**:
```
top
# rust-daq using 80-100% CPU
# Expected: 5-20% for typical workload
```

**Solutions**:

1. **Check for busy-wait loops**:
   ```bash
   # Enable trace logging
   export RUST_LOG=trace
   sudo systemctl restart rust-daq

   # Look for repeated log entries in same millisecond
   # Indicates busy-wait (spinning)
   ```

2. **Reduce mailbox capacity** (if buffer is too large):
   ```toml
   [actors]
   default_mailbox_capacity = 50  # Reduce from 100
   ```

3. **Reduce logging level** (debug/trace uses CPU):
   ```toml
   [application]
   log_level = "info"  # Change from debug
   ```

4. **Check for excessive hardware polling**:
   - Some actors may be polling hardware frequently
   - Reduce polling rate if available

### High Memory Usage

**Symptoms**:
```
top
# VIRT/RES column shows >1GB
# Expected: 100-500MB for typical workload
```

**Solutions**:

1. **Check for memory leaks**:
   ```bash
   # Monitor over time
   watch -n 10 'ps aux | grep rust-daq | grep -v grep'

   # If RSS grows steadily: memory leak
   # If stable: normal buffering
   ```

2. **Reduce buffer capacities**:
   ```toml
   [actors]
   default_mailbox_capacity = 50  # Smaller queues

   [storage]
   compression_level = 9  # More compression = less buffering
   ```

3. **Check storage backend**:
   ```toml
   [storage]
   default_backend = "arrow"  # More memory-efficient than HDF5
   ```

4. **Enable auto-flush** to free memory:
   ```toml
   [storage]
   auto_flush_interval_secs = 10  # Flush frequently
   ```

### Slow Data Acquisition

**Symptoms**:
```
# Measurement rate < expected
# Example: getting 100 Hz instead of 1000 Hz
```

**Solutions**:

1. **Check system load**:
   ```bash
   top
   # If other processes consuming CPU, reduce them
   ```

2. **Check hardware bottleneck**:
   ```bash
   # Is the slow part hardware or software?
   # Test hardware directly (bypass rust-daq)

   # PVCAM: test with Photometrics tools
   # SCPI: test with telnet
   # Serial: test with minicom
   ```

3. **Optimize storage**:
   ```toml
   [storage]
   default_backend = "arrow"  # Faster than HDF5
   compression_level = 0  # No compression for speed
   auto_flush_interval_secs = 120  # Batch flushes
   ```

4. **Check network for SCPI instruments**:
   ```bash
   ping -c 100 192.168.1.100 | grep avg
   # Latency should be <10ms
   # If higher: network congestion
   ```

---

## Data Integrity Issues

### Verify Data File Integrity

```bash
# For HDF5 files:
h5dump -H /var/lib/rust-daq/data/data_001.h5
h5stat /var/lib/rust-daq/data/data_001.h5

# For Arrow files:
# Use Python:
python3 << 'EOF'
import pyarrow.ipc as ipc
with open("/var/lib/rust-daq/data/data_001.arrow", "rb") as f:
    reader = ipc.open_stream(f)
    for i in range(min(5, reader.num_record_batches)):
        print(reader.get_batch(i))
EOF
```

### Detect Dropped Data

**If using PVCAM streaming**:
```bash
# Check logs for frame drops
sudo journalctl -u rust-daq | grep -i "dropped"

# Count dropped frames
sudo journalctl -u rust-daq | grep "dropped" | wc -l
```

**Mitigation**:
- Increase mailbox capacity
- Use faster storage
- Reduce other system load
- Lower acquisition frame rate

---

## Diagnostics Procedures

### Complete System Diagnostics

Run this script to diagnose all issues:

```bash
#!/bin/bash
echo "=== Rust DAQ Diagnostics ==="

# System info
echo ""
echo "1. System Information:"
uname -a
free -h
df -h /var/lib/rust-daq

# Service status
echo ""
echo "2. Service Status:"
sudo systemctl status rust-daq
sudo systemctl is-active rust-daq

# Configuration
echo ""
echo "3. Configuration Validation:"
/opt/rust-daq/bin/rust-daq-v4 --validate-config /opt/rust-daq/config/config.v4.toml

# Network (for SCPI instruments)
echo ""
echo "4. Network Connectivity:"
ping -c 3 192.168.1.100 || echo "Network instrument unreachable"

# VISA
echo ""
echo "5. VISA Status:"
which visainfo && visainfo || echo "VISA not installed"

# Serial Ports
echo ""
echo "6. Serial Ports:"
ls -la /dev/tty* | grep -E "USB|ACM" || echo "No serial ports found"

# Permissions
echo ""
echo "7. User Permissions:"
id rustdaq
groups rustdaq

# Recent logs
echo ""
echo "8. Recent Errors:"
sudo journalctl -u rust-daq | grep -i error | tail -10

# Processes
echo ""
echo "9. Running Processes:"
ps aux | grep rust-daq | grep -v grep

# Port usage
echo ""
echo "10. Network Ports:"
sudo netstat -lntp | grep rust-daq || echo "Service not listening"

echo ""
echo "=== Diagnostics Complete ==="
```

### Collecting Logs for Support

When reporting issues, collect:

```bash
# Create diagnostics package
mkdir -p /tmp/rust-daq-diagnostics
cd /tmp/rust-daq-diagnostics

# Logs (last 1000 lines)
sudo journalctl -u rust-daq -n 1000 > service-logs.txt

# System info
uname -a > system-info.txt
free -h >> system-info.txt
df -h >> system-info.txt

# Configuration (sanitized)
sudo cat /opt/rust-daq/config/config.v4.toml > config.toml
# Edit: remove any sensitive IP addresses or credentials

# Hardware info
lsusb > hardware-usb.txt
lsof /dev/ttyUSB* > hardware-ports.txt 2>/dev/null || true
visainfo > visa-resources.txt 2>/dev/null || true

# Package
tar -czf rust-daq-diagnostics.tar.gz *
echo "Diagnostics saved to: /tmp/rust-daq-diagnostics/rust-daq-diagnostics.tar.gz"
```

---

**Version**: 1.0
**Last Updated**: 2025-11-17
**Maintained By**: Brian Squires
