#!/usr/bin/env bash
set -euo pipefail

# Helper: run PVCAM SDK example binaries on the maitai host with correct env.
# Usage: scripts/pvcam_sdk_examples.sh [ExampleBinary] [args...]
# Env: PVCAM_HOST (default maitai@100.117.5.12), TIMEOUT_SECONDS (default 12)

HOST="${PVCAM_HOST:-maitai@100.117.5.12}"
DEFAULT_EXAMPLE="LiveImage"

usage() {
  cat <<'EOF'
Usage: pvcam_sdk_examples.sh [ExampleBinary] [args...]

Runs a PVCAM SDK example on the remote host with the correct env vars.
Env:
  PVCAM_HOST        SSH target (default: maitai@100.117.5.12)
  TIMEOUT_SECONDS   Timeout for the example (default: 12)
Examples:
  pvcam_sdk_examples.sh LiveImage
  pvcam_sdk_examples.sh LiveImage_SmartStreaming
  TIMEOUT_SECONDS=20 pvcam_sdk_examples.sh FastStreamingToDisk
EOF
}

EXAMPLE="${1:-$DEFAULT_EXAMPLE}"
if [[ "$EXAMPLE" == "-h" || "$EXAMPLE" == "--help" ]]; then
  usage
  exit 0
fi
shift || true

# Allow only known example names to avoid accidental injection.
case "$EXAMPLE" in
  LiveImage|LiveImage_SmartStreaming|LiveImage_triggering|LiveImage_SoftwareTrigger|LiveImage_ChangeExposure|LiveImage_MultiCam|FastStreamingToDisk|ImageSequence|ImageSequence_Snap|ImageSequence_MultiCam|Centroids|ExtendedBinningFactors|ExtendedEnumerations|FanSpeedAndTemperature|FrameSumming_32bpp|MultipleRegions|PostProcessingParameters|PostProcessingProgrammatically|ProgrammableScanMode)
    ;;
  *)
    echo "Unsupported example: $EXAMPLE" >&2
    exit 1
    ;;
esac

TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-12}"

REMOTE_ARGS=""
for arg in "$@"; do
  REMOTE_ARGS+=" $(printf '%q' "$arg")"
done

REMOTE_CMD="source /etc/profile.d/pvcam.sh && source /etc/profile.d/pvcam-sdk.sh && export PVCAM_SDK_DIR=/opt/pvcam/sdk && export LIBRARY_PATH=/opt/pvcam/library/x86_64:\$LIBRARY_PATH && export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:/opt/pvcam/drivers/user-mode:\$LD_LIBRARY_PATH && cd /opt/pvcam/sdk/examples/code_samples/bin/linux-x86_64/release && timeout ${TIMEOUT_SECONDS} ./$(printf '%q' "$EXAMPLE")${REMOTE_ARGS}"

ssh "$HOST" "bash -lc $(printf '%q' "$REMOTE_CMD")"
