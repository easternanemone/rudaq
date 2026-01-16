# daq-egui

The egui-based GUI for rust-daq, providing a desktop control panel for the headless daemon.

**egui Version:** 0.33 | **Rerun Version:** 0.27.3

## Binaries

- `rust-daq-gui` (default): Main control panel for daemon interaction
- `daq-rerun` (feature `rerun_viewer`): Integrated control panel with embedded Rerun viewer for live camera streaming

## Quick Start

```bash
# Start the daemon (in another terminal)
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml

# Start the GUI
cargo run -p daq-egui --bin rust-daq-gui --features standalone
```

The GUI will automatically connect to `http://127.0.0.1:50051`.

## User Interface
The application features a modern, dockable interface powered by `egui_dock`.

### Layout
- **Navigation Panel (Left):** Access all tools and modules.
- **Main Content (Center):** The active tool (e.g., Signal Plotter, Scripts).
- **Logs (Bottom):** Real-time application logs and system status.

### Customizing the Workspace
- **Drag and Drop:** Click and drag tab headers to rearrange panels.
- **Split Views:** Drag a tab to the edge of a pane to split the view (e.g., view Scripts and Plotter side-by-side).
- **Persistence:** Your layout is automatically saved and restored when you restart the application.

### Navigation Panel
The Navigation panel acts as the command center:
- **Focus Tab:** Clicking a button (e.g., "Signal Plotter") focuses that tab if it's already open.
- **Open Tab:** If a tab isn't visible, clicking its button in Navigation will add it to the workspace.

## Connection Management

The GUI implements robust connection handling with auto-reconnect, health monitoring, and graceful offline degradation. See [ADR: Connection Reliability](../../docs/architecture/adr-connection-reliability.md) for architectural details.

### Connection States

| State | Color | Description |
|-------|-------|-------------|
| Disconnected | Gray | No connection, click Connect to start |
| Connecting | Yellow | Connection attempt in progress |
| Connected | Green | Healthy connection to daemon |
| Reconnecting | Yellow | Auto-recovery after connection loss |
| Error | Red | Non-recoverable error (e.g., invalid URL) |

### Configuration

**Environment Variables:**

| Variable | Default | Description |
|----------|---------|-------------|
| `DAQ_DAEMON_URL` | `http://127.0.0.1:50051` | Daemon gRPC address |

**Address Resolution Order:**
1. User input in connection bar (highest priority)
2. Saved address from previous session
3. `DAQ_DAEMON_URL` environment variable
4. Default: `http://127.0.0.1:50051`

### Auto-Reconnect

When connection is lost, the GUI automatically attempts reconnection with exponential backoff:

- **Base delay:** 1 second
- **Max delay:** 30 seconds (capped)
- **Max attempts:** 10

The status bar shows "Reconnecting in Xs..." with a Cancel button.

### Health Monitoring

The GUI performs periodic health checks to detect "zombie" connections:

- **Interval:** Every 30 seconds
- **Method:** `get_daemon_info` RPC call
- **Failure threshold:** 2 consecutive failures triggers reconnect

### Error Messages

Common connection errors and their meaning:

| Message | Cause | Action |
|---------|-------|--------|
| "Daemon not running" | No process listening on port | Start the daemon |
| "Cannot resolve hostname" | DNS lookup failed | Check address spelling |
| "Connection timed out" | Network unreachable or firewall | Check network/firewall |
| "Network connection lost" | Transport error, will auto-retry | Wait for reconnect |

## Developer Guide

### Key Modules

| Module | Purpose |
|--------|---------|
| `client.rs` | `DaqClient` gRPC wrapper, `ChannelConfig` |
| `connection.rs` | `DaemonAddress`, URL normalization, persistence |
| `reconnect.rs` | `ConnectionManager`, `ConnectionState`, auto-reconnect logic |
| `app.rs` | Main application, egui integration |
| `icons.rs` | Phosphor icon constants organized by category |
| `layout.rs` | Standardized spacing, colors, and frame helpers |
| `theme.rs` | Light/Dark/System theme switching with persistence |
| `widgets/` | Reusable UI components (see below) |

### ConnectionManager

The `ConnectionManager` handles all connection lifecycle:

```rust
use crate::reconnect::{ConnectionManager, ConnectionState};

let mut connection = ConnectionManager::new();

// Start connection
connection.connect(address, &runtime);

// Check state
if connection.state().is_connected() {
    // Use the client
}

// Poll for results in UI loop
if let Some((client, version)) = connection.poll(&runtime, &address) {
    // New connection established
}

// Disconnect
connection.disconnect();
```

### Offline Mode Widget

Use `offline_notice` in panels that require daemon connection:

```rust
use crate::widgets::{offline_notice, OfflineContext};

pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>) {
    ui.heading("My Panel");

    // Show offline notice if disconnected, return early
    if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
        return;
    }

    // Normal panel content (only rendered when connected)
    // ...
}
```

**Available contexts:**
- `OfflineContext::Devices` - Device/instrument control
- `OfflineContext::Experiments` - Experiment/scan execution
- `OfflineContext::Scripts` - Script execution
- `OfflineContext::Storage` - Data storage operations
- `OfflineContext::Modules` - Module management

### Adding New gRPC Methods

1. Add method to `DaqClient` in `client.rs`:

```rust
pub async fn my_method(&mut self, param: &str) -> Result<MyResponse> {
    let request = MyRequest { param: param.to_string() };
    let response = self.control.my_method(request).await?;
    Ok(response.into_inner())
}
```

2. Handle connection errors with `friendly_error_message`:

```rust
use crate::reconnect::friendly_error_message;

match client.my_method("test").await {
    Ok(response) => { /* success */ },
    Err(e) => {
        let msg = friendly_error_message(&e.to_string());
        self.error = Some(msg);
    }
}
```

## daq-rerun: Integrated Rerun Viewer

The `daq-rerun` binary provides a single integrated GUI combining DAQ controls with an embedded Rerun viewer for live camera visualization. This uses the "Mullet Architecture": gRPC for control plane, Rerun for data plane.

### Remote Hardware Streaming

To stream camera data from a remote daemon (e.g., maitai):

```bash
# On remote machine: start daemon with camera
ssh maitai@100.117.5.12
source /etc/profile.d/pvcam.sh
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
cd ~/rust-daq
cargo run --bin rust-daq-daemon --features pvcam_sdk -- daemon --port 50051 --lab-hardware

# On local machine: run integrated GUI
RERUN_URL=rerun+http://100.117.5.12:9876/proxy \
DAQ_DAEMON_URL=http://100.117.5.12:50051 \
cargo run --bin daq-rerun --features rerun_viewer
```

### When SSH Tunneling is Needed

**Tailscale users (recommended):** No tunneling required. Both machines on the same Tailnet can connect directly via Tailscale IP (e.g., `100.x.x.x`). The daemon binds to `0.0.0.0` by default, making it accessible on all network interfaces.

**Non-Tailscale users:** SSH tunneling is required when:
- The remote machine is behind a firewall blocking port 50051
- You're connecting over public internet without VPN
- Security policy requires encrypted transport

```bash
# Create SSH tunnel (only if not using Tailscale)
ssh -L 50051:localhost:50051 -L 9876:localhost:9876 user@remote-host

# Then connect GUI to localhost
DAQ_DAEMON_URL=http://127.0.0.1:50051 cargo run --bin daq-rerun --features rerun_viewer
```

**Automatic tunneling:** Future versions may support automatic SSH tunnel management. Track progress in beads issue tracker.

### Architecture

- **Control Plane (gRPC :50051):** Device commands, script execution, status
- **Data Plane (Rerun :9876):** Camera frames, measurements, visualizations

The daemon logs camera frames directly to its Rerun server, which the embedded viewer connects to. This avoids the bandwidth issues of streaming 8MB frames over gRPC.

## PVCAM Live View

With `pvcam`, `pvcam_sdk`, `arrow`, and `pvcam_arrow` enabled, the left nav shows a **"PVCAM Live to Rerun"** toggle.

**Required environment:**
```bash
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_LIB_DIR=/opt/pvcam/library/x86_64
export PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
```

**Run with Rerun viewer:**
```bash
# Terminal 1: Start Rerun viewer
cargo run -p daq-egui --bin daq-rerun --features rerun_viewer

# Terminal 2: Start GUI with PVCAM support
cargo run -p daq-egui --bin rust-daq-gui \
  --features "pvcam,pvcam_sdk,arrow,pvcam_arrow"
```

Frames stream to Rerun at `127.0.0.1:9876` under path `/pvcam/image`.

## Testing

```bash
# Run all daq-egui tests
cargo test -p daq-egui --features standalone

# Run with verbose output
cargo test -p daq-egui --features standalone -- --nocapture
```

## Troubleshooting

**GUI shows "Connecting..." indefinitely:**
- Check if daemon is running: `ps aux | grep rust-daq-daemon`
- Verify port: `netstat -an | grep 50051`
- Check firewall rules if connecting remotely

**"Transport error" after working connection:**
- Daemon may have crashed - check daemon logs
- Network interruption - GUI will auto-reconnect
- If persistent, restart both daemon and GUI

**Panels show "Not Connected" even when status bar is green:**
- Rare race condition - click Refresh in the panel
- If persistent, disconnect and reconnect via status bar
