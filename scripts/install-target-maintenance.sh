#!/usr/bin/env bash
# Install/uninstall periodic local target cleanup.
#
# macOS: installs a launchd user agent
# Linux: installs a user crontab entry

set -euo pipefail

MODE="full"
THRESHOLD_GB=30
SCHEDULE="weekly"
UNINSTALL=false

usage() {
  cat <<'EOF'
Usage: scripts/install-target-maintenance.sh [options]

Options:
  --mode <full|partial>    Cleanup mode passed to target-maintenance.sh (default: full)
  --threshold-gb <N>       Cleanup threshold in GiB (default: 30)
  --schedule <weekly|monthly>
                           Run weekly (Sun 03:17) or monthly (day 1 at 03:17)
  --uninstall              Remove scheduled maintenance
  -h, --help               Show this help

Examples:
  bash scripts/install-target-maintenance.sh
  bash scripts/install-target-maintenance.sh --mode partial --threshold-gb 20
  bash scripts/install-target-maintenance.sh --uninstall
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      MODE="${2:-}"
      shift 2
      ;;
    --threshold-gb)
      THRESHOLD_GB="${2:-}"
      shift 2
      ;;
    --schedule)
      SCHEDULE="${2:-}"
      shift 2
      ;;
    --uninstall)
      UNINSTALL=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ "$MODE" != "full" && "$MODE" != "partial" ]]; then
  echo "Invalid mode: $MODE (expected full or partial)" >&2
  exit 2
fi

if [[ "$SCHEDULE" != "weekly" && "$SCHEDULE" != "monthly" ]]; then
  echo "Invalid schedule: $SCHEDULE (expected weekly or monthly)" >&2
  exit 2
fi

if ! [[ "$THRESHOLD_GB" =~ ^[0-9]+$ ]]; then
  echo "Invalid --threshold-gb value: $THRESHOLD_GB" >&2
  exit 2
fi

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
SCRIPT_PATH="$REPO_ROOT/scripts/target-maintenance.sh"
LOG_DIR="$REPO_ROOT/.git/maintenance-logs"
LOG_FILE="$LOG_DIR/target-maintenance.log"

if [[ ! -x "$SCRIPT_PATH" ]]; then
  echo "Expected executable script at: $SCRIPT_PATH" >&2
  echo "Run: chmod +x scripts/target-maintenance.sh" >&2
  exit 1
fi

mkdir -p "$LOG_DIR"

install_macos() {
  local label="com.rustdaq.target-maintenance"
  local plist="$HOME/Library/LaunchAgents/${label}.plist"
  local schedule_block

  if [[ "$SCHEDULE" == "weekly" ]]; then
    schedule_block='
    <dict>
      <key>Weekday</key>
      <integer>0</integer>
      <key>Hour</key>
      <integer>3</integer>
      <key>Minute</key>
      <integer>17</integer>
    </dict>'
  else
    schedule_block='
    <dict>
      <key>Day</key>
      <integer>1</integer>
      <key>Hour</key>
      <integer>3</integer>
      <key>Minute</key>
      <integer>17</integer>
    </dict>'
  fi

  if [[ "$UNINSTALL" == "true" ]]; then
    launchctl bootout "gui/${UID}/${label}" >/dev/null 2>&1 || true
    rm -f "$plist"
    echo "Removed launchd target maintenance job."
    return
  fi

  mkdir -p "$HOME/Library/LaunchAgents"

  cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${SCRIPT_PATH}</string>
    <string>--mode</string>
    <string>${MODE}</string>
    <string>--threshold-gb</string>
    <string>${THRESHOLD_GB}</string>
  </array>
  <key>WorkingDirectory</key>
  <string>${REPO_ROOT}</string>
  <key>StandardOutPath</key>
  <string>${LOG_FILE}</string>
  <key>StandardErrorPath</key>
  <string>${LOG_FILE}</string>
  <key>StartCalendarInterval</key>${schedule_block}
</dict>
</plist>
EOF

  launchctl bootout "gui/${UID}/${label}" >/dev/null 2>&1 || true
  if ! launchctl bootstrap "gui/${UID}" "$plist" >/dev/null 2>&1; then
    launchctl load -w "$plist"
  fi

  echo "Installed launchd target maintenance job: ${label}"
  echo "Log file: ${LOG_FILE}"
}

install_linux() {
  local marker="# rust-daq-target-maintenance"
  local schedule_expr
  local cron_cmd
  local new_line
  local existing
  local filtered

  if ! command -v crontab >/dev/null 2>&1; then
    echo "crontab not found; cannot install schedule automatically." >&2
    exit 1
  fi

  if [[ "$SCHEDULE" == "weekly" ]]; then
    schedule_expr="17 3 * * 0"
  else
    schedule_expr="17 3 1 * *"
  fi

  cron_cmd="cd '$REPO_ROOT' && '$SCRIPT_PATH' --mode '$MODE' --threshold-gb '$THRESHOLD_GB' >> '$LOG_FILE' 2>&1"
  new_line="${schedule_expr} ${cron_cmd} ${marker}"

  existing="$(crontab -l 2>/dev/null || true)"
  filtered="$(printf '%s\n' "$existing" | grep -v "$marker" || true)"

  if [[ "$UNINSTALL" == "true" ]]; then
    printf '%s\n' "$filtered" | sed '/^[[:space:]]*$/d' | crontab -
    echo "Removed cron target maintenance job."
    return
  fi

  {
    printf '%s\n' "$filtered"
    printf '%s\n' "$new_line"
  } | sed '/^[[:space:]]*$/d' | crontab -

  echo "Installed cron target maintenance job."
  echo "Schedule: $SCHEDULE (${schedule_expr})"
  echo "Log file: ${LOG_FILE}"
}

case "$(uname -s)" in
  Darwin)
    install_macos
    ;;
  Linux)
    install_linux
    ;;
  *)
    echo "Unsupported OS for automatic scheduler install: $(uname -s)" >&2
    echo "Run scripts/target-maintenance.sh manually or wire it into your local scheduler."
    exit 1
    ;;
esac
