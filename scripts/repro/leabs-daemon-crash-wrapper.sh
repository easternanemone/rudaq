#!/usr/bin/env bash
# Run rust-daq-daemon under a crash-capture wrapper on leabs-dev.
# Intended to be launched on the remote host (directly or via SSH + nohup).
#
# The wrapper:
# - captures startup environment/provenance
# - enables core dumps where allowed (ulimit -c unlimited)
# - waits for daemon exit (including segfaults)
# - records exit code, signal inference, journal excerpts, and coredump metadata
# - writes all artifacts to a timestamped session directory
#
# Example (run on leabs-dev inside repo):
#   source "$HOME/.cargo/env"
#   source config/hosts/leabs-dev.env
#   nohup bash scripts/repro/leabs-daemon-crash-wrapper.sh \
#     --capture-root /tmp/rust-daq-daemon-crash \
#     --label istar_stream_repro \
#     -- ./target/release/rust-daq-daemon daemon --port 50051 \
#        --hardware-config config/leabs_hardware.toml --runtime-mode universal \
#     >/tmp/rust-daq-daemon-wrapper.out 2>&1 < /dev/null &

set -euo pipefail

CAPTURE_ROOT="/tmp/rust-daq-daemon-crash"
LABEL="daemon"
JOURNAL_LINES=400

usage() {
  cat <<'EOF'
Usage: leabs-daemon-crash-wrapper.sh [options] -- <daemon command...>

Options:
  --capture-root <dir>   Root directory for crash sessions (default: /tmp/rust-daq-daemon-crash)
  --label <name>         Session label prefix (default: daemon)
  --journal-lines <n>    Number of journal lines to store on exit (default: 400)
  -h, --help             Show help

All arguments after `--` are executed as the wrapped daemon process.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --capture-root)
      CAPTURE_ROOT="$2"
      shift 2
      ;;
    --label)
      LABEL="$2"
      shift 2
      ;;
    --journal-lines)
      JOURNAL_LINES="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ $# -eq 0 ]]; then
  echo "No wrapped command provided (use -- <command...>)" >&2
  usage >&2
  exit 2
fi

mkdir -p "$CAPTURE_ROOT"
ts="$(date +%Y%m%dT%H%M%S%z)"
session_dir="${CAPTURE_ROOT}/${LABEL}_${ts}_$$"
mkdir -p "$session_dir"

printf '%s\n' "$session_dir" > "${CAPTURE_ROOT}/latest_session_path.txt"

cmd_file="${session_dir}/command.txt"
log_file="${session_dir}/daemon.stdout_stderr.log"
exit_meta="${session_dir}/exit_meta.env"

{
  printf 'started_at_iso=%s\n' "$(date --iso-8601=seconds)"
  printf 'hostname=%s\n' "$(hostname)"
  printf 'cwd=%s\n' "$(pwd)"
  printf 'user=%s\n' "$(id -un)"
  printf 'uid=%s\n' "$(id -u)"
  printf 'gid=%s\n' "$(id -g)"
  printf 'wrapper_pid=%s\n' "$$"
} > "${session_dir}/session_meta.env"

printf '%q ' "$@" > "$cmd_file"
printf '\n' >> "$cmd_file"

{
  echo "# uname"
  uname -a || true
  echo
  echo "# core_pattern"
  cat /proc/sys/kernel/core_pattern 2>/dev/null || true
  echo
  echo "# ulimit -a (before)"
  ulimit -a || true
} > "${session_dir}/system_snapshot.txt"

# Try to allow core dump generation (will silently no-op if restricted by system policy).
ulimit -c unlimited || true
{
  echo
  echo "# ulimit -a (after ulimit -c unlimited)"
  ulimit -a || true
} >> "${session_dir}/system_snapshot.txt"

start_epoch="$(date +%s)"
start_iso="$(date --iso-8601=seconds)"
printf 'start_epoch=%s\nstart_iso=%s\n' "$start_epoch" "$start_iso" >> "${session_dir}/session_meta.env"

export RUST_BACKTRACE="${RUST_BACKTRACE:-full}"
export RUST_LOG="${RUST_LOG:-info}"
export RUSTFLAGS="${RUSTFLAGS:-}"

echo "[wrapper] session_dir=$session_dir"
echo "[wrapper] starting: $(cat "$cmd_file")"

set +e
"$@" >"$log_file" 2>&1
cmd_ec=$?
set -e

end_epoch="$(date +%s)"
end_iso="$(date --iso-8601=seconds)"
signal_name=""
signal_num=""
if [[ $cmd_ec -ge 128 ]]; then
  signal_num=$((cmd_ec - 128))
  signal_name="$(kill -l "$signal_num" 2>/dev/null || true)"
fi

{
  printf 'wrapped_exit_code=%s\n' "$cmd_ec"
  printf 'end_epoch=%s\n' "$end_epoch"
  printf 'end_iso=%s\n' "$end_iso"
  printf 'duration_sec=%s\n' "$((end_epoch - start_epoch))"
  printf 'signal_num=%s\n' "$signal_num"
  printf 'signal_name=%s\n' "$signal_name"
} > "$exit_meta"

{
  echo "# daemon process exit summary"
  cat "$exit_meta"
  echo
  echo "# recent journal (last ${JOURNAL_LINES} lines)"
  journalctl --since "@${start_epoch}" --no-pager 2>&1 | tail -n "$JOURNAL_LINES" || true
} > "${session_dir}/journal_since_start.log"

{
  echo "# coredumpctl list"
  coredumpctl list --no-pager 2>&1 || true
  echo
  echo "# coredumpctl info (best effort by binary name)"
  coredumpctl info rust-daq-daemon --no-pager 2>&1 || true
} > "${session_dir}/coredumpctl.txt"

{
  echo "# possible core files under repo and /tmp"
  find . /tmp -maxdepth 2 -type f \( -name 'core' -o -name 'core.*' \) -printf '%TY-%Tm-%Td %TT %p %s bytes\n' 2>&1 || true
} > "${session_dir}/core_files.txt"

echo "[wrapper] wrapped process exited (code=${cmd_ec}${signal_name:+, signal=${signal_name}})"
echo "[wrapper] artifacts: ${session_dir}"

exit "$cmd_ec"
