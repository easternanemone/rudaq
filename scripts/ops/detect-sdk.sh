#!/usr/bin/env bash
# detect-sdk.sh — Probe for installed hardware SDKs and output cargo features
#
# Detects PVCAM, Andor SDK3, and Comedi libraries on the current host.
# Outputs sourceable shell variables for use by deploy scripts and CI.
#
# Usage:
#   eval "$(bash scripts/ops/detect-sdk.sh)"       # Source detected vars
#   bash scripts/ops/detect-sdk.sh                  # Print vars + summary
#   source <(bash scripts/ops/detect-sdk.sh)        # Alternative source form
#
# Output (stdout, safe to eval/source):
#   DETECTED_FEATURES="pvcam_hardware,comedi_hardware"
#   PVCAM_SDK_DIR=/opt/pvcam/sdk
#   ANDOR_SDK3_DIR=/usr/local
#   COMEDI_DETECTED=true
#
# Summary goes to stderr so it doesn't pollute eval.

set -euo pipefail

# ============================================================================
# Internal detection state
# ============================================================================
_features=()
_pvcam_sdk_dir=""
_andor_sdk3_dir=""
_comedi_detected=false

# Summary helper (stderr only)
_summary() {
    echo "$@" >&2
}

# ============================================================================
# PVCAM Detection
# ============================================================================
# Probe order (matches env-check.sh and maitai.env):
#   1. PVCAM SDK directory at /opt/pvcam/sdk (contains pvcam.h)
#   2. libpvcam.so in /opt/pvcam/library/x86_64 or standard ld paths
#   3. pvcam.h header
detect_pvcam() {
    local sdk_dir="${PVCAM_SDK_DIR:-/opt/pvcam/sdk}"
    local lib_dir="${PVCAM_LIB_DIR:-/opt/pvcam/library/x86_64}"
    local found_sdk=false
    local found_lib=false
    local found_header=false

    # Check SDK directory
    if [[ -d "$sdk_dir" ]]; then
        found_sdk=true
    fi

    # Check for libpvcam.so
    if [[ -f "${lib_dir}/libpvcam.so" ]]; then
        found_lib=true
    elif ldconfig -p 2>/dev/null | grep -q 'libpvcam\.so'; then
        found_lib=true
    fi

    # Check for pvcam.h header
    if [[ -f "${sdk_dir}/include/pvcam.h" ]] || \
       [[ -f "${sdk_dir}/pvcam.h" ]] || \
       [[ -f "/opt/pvcam/sdk/include/pvcam.h" ]]; then
        found_header=true
    fi

    if $found_sdk && $found_lib; then
        _pvcam_sdk_dir="$sdk_dir"
        _features+=("pvcam_hardware")
        _summary "  [FOUND]  PVCAM: sdk=${sdk_dir}, lib=$($found_lib && echo yes || echo no), header=$($found_header && echo yes || echo no)"
    elif $found_sdk; then
        _pvcam_sdk_dir="$sdk_dir"
        _summary "  [PARTIAL] PVCAM: sdk=${sdk_dir} found but libpvcam.so missing"
    else
        _summary "  [ABSENT] PVCAM: no SDK directory at ${sdk_dir}"
    fi
}

# ============================================================================
# Andor SDK3 Detection
# ============================================================================
# Probe order (matches leabs-dev.env):
#   1. atcore.h in $ANDOR_SDK3_DIR/include or /usr/local/include
#   2. libatcore.so in $ANDOR_SDK3_DIR/lib or /usr/local/lib or ld paths
detect_andor() {
    local sdk_dir="${ANDOR_SDK3_DIR:-/usr/local}"
    local found_header=false
    local found_lib=false

    # Check for atcore.h
    if [[ -f "${sdk_dir}/include/atcore.h" ]]; then
        found_header=true
    elif [[ -f "/usr/local/include/atcore.h" ]]; then
        found_header=true
        sdk_dir="/usr/local"
    fi

    # Check for libatcore.so
    if [[ -f "${sdk_dir}/lib/libatcore.so" ]]; then
        found_lib=true
    elif [[ -f "/usr/local/lib/libatcore.so" ]]; then
        found_lib=true
        sdk_dir="/usr/local"
    elif ldconfig -p 2>/dev/null | grep -q 'libatcore\.so'; then
        found_lib=true
    fi

    if $found_header && $found_lib; then
        _andor_sdk3_dir="$sdk_dir"
        _features+=("andor_hardware")
        _summary "  [FOUND]  Andor SDK3: dir=${sdk_dir}, header=yes, lib=yes"
    elif $found_lib; then
        _andor_sdk3_dir="$sdk_dir"
        _summary "  [PARTIAL] Andor SDK3: libatcore.so found but atcore.h missing"
    elif $found_header; then
        _summary "  [PARTIAL] Andor SDK3: atcore.h found but libatcore.so missing"
    else
        _summary "  [ABSENT] Andor SDK3: no headers or libraries found"
    fi
}

# ============================================================================
# Comedi Detection
# ============================================================================
# Probe order:
#   1. comedilib.h in /usr/include or /usr/local/include
#   2. libcomedi.so in standard ld paths
detect_comedi() {
    local found_header=false
    local found_lib=false

    # Check for comedilib.h
    if [[ -f "/usr/include/comedilib.h" ]] || \
       [[ -f "/usr/local/include/comedilib.h" ]]; then
        found_header=true
    fi

    # Check for libcomedi.so
    if [[ -f "/usr/lib/libcomedi.so" ]] || \
       [[ -f "/usr/lib/x86_64-linux-gnu/libcomedi.so" ]] || \
       [[ -f "/usr/local/lib/libcomedi.so" ]]; then
        found_lib=true
    elif ldconfig -p 2>/dev/null | grep -q 'libcomedi\.so'; then
        found_lib=true
    fi

    if $found_header && $found_lib; then
        _comedi_detected=true
        _features+=("comedi_hardware")
        _summary "  [FOUND]  Comedi: header=yes, lib=yes"
    elif $found_lib; then
        _summary "  [PARTIAL] Comedi: libcomedi.so found but comedilib.h missing"
    elif $found_header; then
        _summary "  [PARTIAL] Comedi: comedilib.h found but libcomedi.so missing"
    else
        _summary "  [ABSENT] Comedi: not installed"
    fi
}

# ============================================================================
# Main
# ============================================================================
_summary "detect-sdk: Probing for hardware SDKs on $(hostname)..."

detect_pvcam
detect_andor
detect_comedi

# Build comma-separated feature list
DETECTED_FEATURES=""
if [[ ${#_features[@]} -gt 0 ]]; then
    DETECTED_FEATURES=$(IFS=,; echo "${_features[*]}")
fi

_summary ""
_summary "  Detected features: ${DETECTED_FEATURES:-<none>}"
_summary ""

# Output sourceable variables to stdout
echo "DETECTED_FEATURES=\"${DETECTED_FEATURES}\""

if [[ -n "$_pvcam_sdk_dir" ]]; then
    echo "PVCAM_SDK_DIR=\"${_pvcam_sdk_dir}\""
fi

if [[ -n "$_andor_sdk3_dir" ]]; then
    echo "ANDOR_SDK3_DIR=\"${_andor_sdk3_dir}\""
fi

echo "COMEDI_DETECTED=${_comedi_detected}"
