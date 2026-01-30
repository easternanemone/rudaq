#!/bin/bash
set -e

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== PVCAM Arch Linux Installer ===${NC}"

# Check for root
if [[ $EUID -eq 0 ]]; then
   echo -e "${RED}This script calls sudo internally. Please do not run as root directly.${NC}"
   exit 1
fi

# 1. Install Dependencies
echo -e "${YELLOW}Step 1: Installing Dependencies...${NC}"

# Detect Kernel for Headers
KERNEL_RELEASE=$(uname -r)
echo "Detected running kernel: $KERNEL_RELEASE"

HEADERS_PKG="linux-headers"
if [[ "$KERNEL_RELEASE" == *"zen"* ]]; then
    HEADERS_PKG="linux-zen-headers"
elif [[ "$KERNEL_RELEASE" == *"lts"* ]]; then
    HEADERS_PKG="linux-lts-headers"
elif [[ "$KERNEL_RELEASE" == *"hardened"* ]]; then
    HEADERS_PKG="linux-hardened-headers"
fi

echo -e "Installing: ${GREEN}dkms libusb libtiff base-devel $HEADERS_PKG${NC}"
echo "You may be asked for your sudo password."
sudo pacman -Syu --noconfirm --needed dkms libusb libtiff base-devel $HEADERS_PKG

# 2. Locate Installers
INSTALL_DIR="$HOME/pvcam_install"
if [ ! -d "$INSTALL_DIR" ]; then
    echo -e "${RED}Error: Install directory $INSTALL_DIR not found.${NC}"
    echo "Please ensure the installer archive was extracted correctly."
    exit 1
fi

# 3. Run Driver Installer
echo -e "\n${YELLOW}Step 2: Installing PVCAM Driver...${NC}"
cd "$INSTALL_DIR/pvcam" || exit 1
DRIVER_RUN=$(find . -maxdepth 1 -name "pvcam_*.run" | head -n 1)

if [ -z "$DRIVER_RUN" ]; then
    echo -e "${RED}Error: Driver installer (pvcam_*.run) not found in $PWD${NC}"
    exit 1
fi

echo "Found driver installer: $DRIVER_RUN"
chmod +x "$DRIVER_RUN"
echo "Running installer (Unattended)..."
sudo ./"$DRIVER_RUN" -q -- -q --accept-license

# 4. Run SDK Installer
echo -e "\n${YELLOW}Step 3: Installing PVCAM SDK...${NC}"
cd "$INSTALL_DIR/pvcam-sdk" || exit 1
SDK_RUN=$(find . -maxdepth 1 -name "pvcam-sdk_*.run" | head -n 1)

if [ -z "$SDK_RUN" ]; then
    echo -e "${RED}Error: SDK installer (pvcam-sdk_*.run) not found in $PWD${NC}"
    exit 1
fi

echo "Found SDK installer: $SDK_RUN"
chmod +x "$SDK_RUN"
echo "Running installer (Unattended)..."
sudo ./"$SDK_RUN" -q -- -q --accept-license

echo -e "\n${GREEN}=== Installation Complete! ===${NC}"
echo -e "${YELLOW}Please REBOOT your machine now to load the new drivers.${NC}"
echo "Command: sudo reboot"
