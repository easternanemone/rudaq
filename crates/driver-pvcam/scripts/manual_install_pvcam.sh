#!/bin/bash
set -e

# Source Locations (Extracted)
SRC_DRIVER="/home/maitai/pvcam_install/extracted_driver"
SRC_SDK="/home/maitai/pvcam_install/extracted_sdk"

if [ ! -d "$SRC_DRIVER" ] || [ ! -d "$SRC_SDK" ]; then
    echo "Error: Extracted directories not found. Please ensure extraction step completed."
    exit 1
fi

echo "=== PVCAM Manual Install ==="

# 1. Create Directories
echo "Creating /opt/pvcam structure..."
sudo mkdir -p /opt/pvcam/library/x86_64
sudo mkdir -p /opt/pvcam/sdk/include
sudo mkdir -p /opt/pvcam/drivers/user-mode
sudo mkdir -p /opt/pvcam/etc
sudo mkdir -p /etc/udev/rules.d

# 2. Copy Driver Libraries
echo "Installing Core Libraries..."
sudo cp -a "$SRC_DRIVER/opt/pvcam/library/x86_64/"* /opt/pvcam/library/x86_64/

# 3. Copy UMD Drivers (USB Support)
echo "Installing User-Mode Drivers..."
sudo cp -a "$SRC_DRIVER/opt/pvcam/drivers/user-mode/"* /opt/pvcam/drivers/user-mode/

# 4. Copy Udev Rules
echo "Installing Udev Rules..."
sudo cp "$SRC_DRIVER/opt/pvcam/lib/udev/rules.d/"*.rules /etc/udev/rules.d/

# 5. Copy SDK Headers
echo "Installing SDK Headers..."
sudo cp -a "$SRC_SDK/opt/pvcam/sdk/include/"* /opt/pvcam/sdk/include/

# 6. Configure Environment
echo "Configuring Environment..."
# Create /etc/profile.d/pvcam.sh
sudo tee /etc/profile.d/pvcam.sh > /dev/null <<EOF
#!/bin/bash
export PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode
export PVCAM_VERSION=3.10.2.5
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:\$LD_LIBRARY_PATH
EOF
sudo chmod +x /etc/profile.d/pvcam.sh

# 7. Reload Hardware Rules
echo "Reloading Udev Rules..."
sudo udevadm control --reload-rules
sudo udevadm trigger

echo "=== Install Complete ==="
echo "Please re-login or source /etc/profile.d/pvcam.sh to update environment."
