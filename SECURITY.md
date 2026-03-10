# Security Policy

This document describes security considerations for deploying and operating rust-daq.

## Plugin / Driver Extension Model

rust-daq currently supports multiple extension paths for device support:

- Compiled `DriverFactory` implementations — native SDK drivers (PVCAM, Andor, Comedi, etc.) are compiled into the daemon.
- `driver-universal` / manifest-driver configs — text-protocol devices (serial/TCP/SCPI) are described declaratively and loaded at runtime.
- Manifest-driver plugin discovery — the `hardware` crate also contains runtime plugin discovery and optional hot-reload support for manifest-driver plugins.

Operationally, this means deployments should distinguish between:

- trusted, compiled-in drivers
- trusted declarative manifests loaded at runtime
- any runtime plugin search paths or hot-reload directories, which should be treated as privileged inputs

Script plugins (Rhai, Python) are available when the `scripting` feature is enabled and run in the same process.

## Network Security

### gRPC Configuration

The daemon binds to network interfaces as configured in `config/config.v4.toml`:

```toml
[grpc]
bind_address = "0.0.0.0"  # All interfaces in the default lab-oriented config
auth_enabled=***
# auth_token="***"
allowed_origins = ["*"]
```

#### Production Recommendations

1. Restrict bind address — use `127.0.0.1` for loopback-only access:
   ```toml
   bind_address = "127.0.0.1"
   ```

2. Enable authentication — set `auth_enabled=***` and configure an auth token or JWT secret for production deployments.

3. Use firewall rules — restrict access to the gRPC port (default 50051).

4. Deploy behind a reverse proxy — use TLS termination for encrypted connections.

5. Restrict CORS origins — replace `allowed_origins = ["*"]` with explicit trusted origins for browser deployments.

## Reporting Security Issues

If you discover a security vulnerability in rust-daq, please report it responsibly:

1. Do not open a public GitHub issue for security vulnerabilities.
2. Contact the maintainers directly via email.
3. Include detailed steps to reproduce the issue.
4. Allow reasonable time for a fix before public disclosure.

## Related Documentation

- `CLAUDE.md` — development guidelines and architecture overview
- `docs/adr/` — architecture decision records
- `docs/how-to/hardware-drivers.md` — driver guide
- `docs/how-to/web-gui.md` — browser deployment and CORS considerations
