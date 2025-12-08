#!/bin/bash
# Quick hardware check script
# Attempts to identify which device is on which port

echo "=== Quick Hardware Check ==="
echo ""

for port in /dev/ttyUSB{0..5}; do
    if [ -e "$port" ]; then
        echo "Testing $port..."

        # Try to read from port with short timeout
        timeout 2 cat "$port" 2>/dev/null &
        PID=$!

        # Give it a moment
        sleep 0.5

        # Kill if still running
        kill -9 $PID 2>/dev/null

        echo "  Port exists and is accessible"
        echo ""
    fi
done

echo "=== USB Device Info ==="
lsusb | grep -E "(FTDI|Silicon|National)"

echo ""
echo "=== Serial Port Mappings ==="
for port in /dev/ttyUSB{0..5}; do
    if [ -e "$port" ]; then
        udevadm info --name="$port" 2>/dev/null | grep -E "(ID_VENDOR|ID_MODEL|ID_SERIAL)" || echo "  $port: No udev info"
    fi
done
