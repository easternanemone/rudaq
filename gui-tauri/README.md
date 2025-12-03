# rust-daq GUI (Tauri + React) – Legacy / Optional

> **Status:** The primary GUI for rust-daq is now the native `egui` / `eframe` desktop client (`rust_daq_gui_egui`).  
> This Tauri + React project remains available for users who prefer a web‑style UI or want to experiment with Tauri, but it is no longer required for normal operation and may lag behind the egui client in features.

## Features

- **Live Device Monitoring**: Real-time display of device status and values
- **Manual Control**: Direct control of hardware devices through an intuitive interface
- **Multi-device Support**: Control multiple devices simultaneously
- **Dark Theme**: Professional dark theme optimized for scientific instruments
- **Cross-platform**: Runs on macOS, Linux, and Windows

## Architecture

### Backend (Rust/Tauri)
- **gRPC Client**: Connects to rust-daq daemon on localhost:50051
- **Tauri Commands**: Exposes hardware operations to the frontend
- **Async Communication**: Non-blocking hardware control

### Frontend (React + TypeScript)
- **React Query**: State management and caching
- **Tailwind CSS**: Modern, responsive styling
- **Lucide Icons**: Clean, consistent iconography

## Prerequisites

1. **rust-daq daemon** running on localhost:50051
   ```bash
   # From rust-daq root directory
   cargo run --features networking -- daemon --port 50051
   ```

2. **Node.js** (v18 or later)
3. **Rust** (latest stable)

## Installation

```bash
# Navigate to gui-tauri directory
cd gui-tauri

# Install npm dependencies
npm install
```

## Development

```bash
# Run in development mode (hot-reload enabled)
npm run tauri dev
```

This will:
1. Start the Vite dev server on localhost:1420
2. Launch the Tauri window with live reload

## Build

```bash
# Create production build
npm run tauri build
```

This generates:
- **macOS**: `.app` bundle in `src-tauri/target/release/bundle/macos/`
- **Linux**: `.AppImage` or `.deb` in `src-tauri/target/release/bundle/`
- **Windows**: `.msi` installer in `src-tauri/target/release/bundle/msi/`

## Usage

### 1. Connect to Daemon

Click "Connect to Daemon" and enter the daemon address (default: `localhost:50051`).

### 2. Select a Device

The left panel shows all registered devices with live status updates. Click a device to select it.

### 3. Control the Device

The right panel shows controls based on device capabilities:

#### Movable Devices (Stages, Rotation Mounts)
- Enter target position
- Move (async) or Move & Wait (blocks until settled)
- Emergency stop button

#### Readable Devices (Power Meters, Sensors)
- Read current value
- Display with units

#### Cameras (Frame Producers)
- Set exposure time
- View current exposure setting

#### Lasers (MaiTai, etc.)
- Shutter control (open/close)
- Wavelength tuning
- Emission control (on/off)

## Available Tauri Commands

### Connection
- `connect_to_daemon(address: string)` - Connect to gRPC daemon

### Device Discovery
- `list_devices()` - Get all registered devices
- `get_device_state(device_id: string)` - Get current device state

### Motion Control
- `move_absolute(device_id, position, wait_for_completion)`
- `stop_motion(device_id)`

### Scalar Readout
- `read_value(device_id)` - Read sensor value

### Exposure Control
- `set_exposure(device_id, exposure_ms)`
- `get_exposure(device_id)`

### Laser Control
- `set_shutter(device_id, open: bool)`
- `set_wavelength(device_id, wavelength_nm)`
- `set_emission(device_id, enabled: bool)`

### Parameters
- `list_parameters(device_id)` - Get settable parameters
- `get_parameter(device_id, parameter_name)`
- `set_parameter(device_id, parameter_name, value)`

## Project Structure

```
gui-tauri/
├── src/                    # React source code
│   ├── components/         # React components
│   │   ├── ConnectionStatus.tsx
│   │   ├── DeviceStatusPanel.tsx
│   │   └── ManualControlPanel.tsx
│   ├── App.tsx            # Main app component
│   ├── main.tsx           # React entry point
│   └── styles.css         # Global styles
├── src-tauri/             # Rust backend
│   ├── src/
│   │   └── main.rs        # Tauri commands and gRPC client
│   ├── Cargo.toml
│   └── tauri.conf.json    # Tauri configuration
├── package.json
├── tsconfig.json
├── tailwind.config.js
└── vite.config.ts
```

## Troubleshooting

### Cannot connect to daemon
- Ensure daemon is running: `cargo run --features networking -- daemon --port 50051`
- Check firewall settings
- Verify correct address (default: `localhost:50051`)

### Build errors
- Run `npm install` to ensure all dependencies are installed
- Clear build cache: `rm -rf node_modules dist src-tauri/target`
- Reinstall: `npm install && npm run tauri build`

### Device not appearing
- Ensure device is registered with the daemon
- Check daemon logs for errors
- Verify hardware connection (serial port, USB, etc.)

## Development Notes

### Adding New Device Types

1. Add new capability check in `DeviceInfo` type (App.tsx)
2. Create control component in `ManualControlPanel.tsx`
3. Add corresponding Tauri command in `src-tauri/src/main.rs`
4. Implement gRPC call to daemon

### State Management

- **React Query** handles all server state (devices, device states)
- Queries auto-refresh at intervals (1-5 seconds depending on criticality)
- Mutations invalidate relevant queries on success

### Styling

- Uses Tailwind CSS with custom dark theme
- Main colors: slate (background), primary (blue accents), semantic (green/red/yellow)
- Responsive layout with fixed left panel, flexible right panel

## Future Enhancements

- [ ] WebSocket support for lower-latency state updates
- [ ] Frame streaming visualization (for cameras)
- [ ] Parameter change streaming (real-time graph of values)
- [ ] Scan progress visualization
- [ ] Preset management UI
- [ ] Multi-device synchronization controls

## License

Same as rust-daq project
