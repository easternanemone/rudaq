# Rerun Blueprints

Generated layout files for Rerun visualization.

## Default path

The server auto-loads `crates/daq-server/blueprints/daq_default.rbl` unless overridden with `RERUN_BLUEPRINT`.

```
export RERUN_BLUEPRINT=crates/daq-server/blueprints/daq_default.rbl
# or disable blueprint loading
export RERUN_BLUEPRINT=none
```

## Regenerating

Run the helper script (creates a local venv and installs rerun-sdk):

```
scripts/regenerate_blueprints.sh
```

Outputs:
- `daq_default.rbl`
- `daq_camera_only.rbl`
- `daq_timeseries_only.rbl`
- `daq_acquisition.rbl`

Blueprint application ID is `rust-daq`; keep it in sync with `APP_ID` in `crates/daq-server/src/rerun_sink.rs`.
