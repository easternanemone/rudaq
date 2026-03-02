# Andor iSTAR Daemon Crash Capture and Reproduction

This runbook captures evidence when `rust-daq-daemon` exits unexpectedly during iSTAR streaming
and standardizes reproducible stress sequences on `leabs-dev`.

Use this when the GUI reports transport failures like:

- `Failed to start hardware stream: status: Unknown, message: "transport error", ...`

That message can be a *follow-on symptom* after the daemon has already crashed.

## Tools Added

- `scripts/leabs-daemon-crash-wrapper.sh`
  - runs on `leabs-dev`
  - wraps `rust-daq-daemon`
  - captures exit code, journal excerpts, coredump metadata, core-file scan
- `scripts/repro-istar-stream-crash.sh`
  - runs locally
  - starts the wrapped daemon on `leabs-dev` (optional)
  - ensures local SSH tunnel to `127.0.0.1:50051`
  - drives `StartStream` + `StreamFrames` via `grpcurl`
  - polls daemon health and collects remote artifacts into a local run directory

## Sequence A: Continuous Stress Soak (single long stream)

This exercises a sustained `StreamFrames` session.

```bash
cd "$(git rev-parse --show-toplevel)"

bash scripts/repro-istar-stream-crash.sh \
  --soak-seconds 1800 \
  --quality full \
  --max-fps 30 \
  --exposure-ms 10
```

What this does:

1. Starts a wrapped daemon on `leabs-dev` (unless `--no-restart-daemon` is used)
2. Starts `istar_camera` hardware streaming
3. Opens a long-lived `StreamFrames` subscriber (`grpcurl`)
4. Polls `ControlService/GetDaemonInfo` while the stream is active
5. Copies wrapper crash artifacts back into the local run directory

## Sequence B: Restart-Loop Stress (mimics GUI reconnect churn)

This exercises repeated `StartStream` + short `StreamFrames` + `StopStream` cycles against the
same daemon process (closer to GUI reconnect behavior after timeouts/errors).

Step 1: Start wrapped daemon once and validate tunnel (short run, leaves daemon running)

```bash
cd "$(git rev-parse --show-toplevel)"

bash scripts/repro-istar-stream-crash.sh \
  --soak-seconds 5 \
  --quality fast \
  --max-fps 2 \
  --exposure-ms 10
```

Step 2: Repeat short stream cycles without restarting the daemon

```bash
cd "$(git rev-parse --show-toplevel)"

i=0
while [ $i -lt 40 ]; do
  i=$((i+1))
  echo "=== ITERATION $i ==="
  bash scripts/repro-istar-stream-crash.sh \
    --no-restart-daemon \
    --soak-seconds 15 \
    --quality fast \
    --max-fps 10 \
    --exposure-ms 10 || break
done
```

Each iteration creates a separate local run directory with:

- `summary.txt`
- `health_poll.log`
- `start_stream.json` / `stream_frames.stderr.txt`
- `remote_journal.tail.txt`
- `remote_wrapper_session/` (copied wrapper artifacts from `leabs-dev`)

## Artifact Locations

Local runs are stored under:

- `tmp/istar-crash-repro/`

Remote wrapped-daemon sessions are stored under:

- `/tmp/rust-daq-daemon-crash/`
- latest session path file:
  - `/tmp/rust-daq-daemon-crash/latest_session_path.txt`

## Interpreting Results

### If the daemon crashes

Expected signals:

- local run `summary.txt` shows `Health check failed while stream process still running`
- local run exits non-zero
- remote wrapper session `exit_meta.env` shows non-zero exit or signal-derived exit
- remote wrapper `journal_since_start.log` may contain kernel trap / segmentation fault / GPF

### If the daemon stays healthy

You still get a valid baseline run with:

- exact stream parameters used
- daemon health polling timeline
- remote wrapper session evidence showing no crash during the interval

This is still useful because it narrows the trigger conditions.

## Notes for Root-Cause Work

- The wrapper captures evidence but does not "fix" the crash.
- If the crash reproduces, compare:
  - stream quality (`FULL` vs `FAST`)
  - `max_fps`
  - duration-to-failure
  - whether failures correlate with reconnect/start-stop churn vs one continuous stream
- When filing with Andor, include:
  - kernel trap line (`journalctl`)
  - wrapper `exit_meta.env`
  - stream parameters and exposure settings used in the run
