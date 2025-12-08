#!/bin/bash
# Hardware identification script
# Systematically tests each port to identify connected devices

PORTS=("/dev/ttyUSB0" "/dev/ttyUSB1" "/dev/ttyUSB2" "/dev/ttyUSB3" "/dev/ttyUSB4" "/dev/ttyUSB5")

echo "=================================================="
echo "Hardware Identification Script"
echo "=================================================="
echo ""

for port in "${PORTS[@]}"; do
    if [ ! -e "$port" ]; then
        continue
    fi

    echo "Testing $port..."
    echo "=================="

    # Test ESP300 (Newport Motion Controller)
    echo "  [ESP300] Trying ESP300 identification..."
    ESP300_PORT="$port" timeout 10 cargo test --test hardware_esp300_validation \
        --features hardware_tests,instrument_newport --release \
        test_esp300_communication_basic -- --exact --nocapture 2>&1 | grep -E "(test result|passed|FAILED)" | head -5

    # Test Newport 1830-C (Power Meter)
    echo "  [1830-C] Trying Newport 1830-C identification..."
    NEWPORT_PORT="$port" timeout 10 cargo test --test hardware_newport1830c_validation \
        --features hardware_tests,instrument_newport_power_meter --release \
        test_newport_communication_basic -- --exact --nocapture 2>&1 | grep -E "(test result|passed|FAILED)" | head -5

    echo ""
done

echo "=================================================="
echo "Identification Complete"
echo "=================================================="
