# Security Policy

This document describes security considerations for deploying and operating rust-daq.

## Plugin / Driver Extension Model

rust-daq uses two compile-time extension mechanisms for adding device support:

- **`DriverFactory` trait** - Native SDK drivers (PVCAM, Andor, Comedi, etc.) implement this trait and are compiled into the binary.
- **`driver-universal` TOML manifests** - Text-protocol devices (serial/TCP/SCPI) are defined as TOML configuration files in `config/devices/`, loaded at runtime by the manifest driver system.

Neither mechanism loads arbitrary native code at runtime. All driver code is compiled into the daemon binary or interpreted from declarative TOML manifests.

Script plugins (Rhai, Python) are available when the `scripting` feature is enabled and run in the same process.

## Network Security

### gRPC Configuration

The daemon binds to network interfaces as configured in `config/config.v4.toml`:

```toml
[grpc]
bind_address = "0.0.0.0"  # All interfaces (default)
auth_enabled = false
allowed_origins = ["http://localhost:3000", "http://127.0.0.1:3000"]
```

#### Production Recommendations

1. **Restrict bind address** - Use `127.0.0.1` for loopback-only access:
   ```toml
   bind_address = "127.0.0.1"
   ```

2. **Enable authentication** - Set `auth_enabled = true` for production deployments

3. **Use firewall rules** - Restrict access to the gRPC port (default 50051)

4. **Deploy behind a reverse proxy** - Use TLS termination for encrypted connections

## Reporting Security Issues

If you discover a security vulnerability in rust-daq, please report it responsibly:

1. **Do not** open a public GitHub issue for security vulnerabilities
2. Contact the maintainers directly via email
3. Include detailed steps to reproduce the issue
4. Allow reasonable time for a fix before public disclosure

## Related Documentation

- [CLAUDE.md](CLAUDE.md) - Development guidelines and architecture overview
- [docs/adr/](docs/adr/) - Architecture Decision Records
- Driver guide: `docs/how-to/hardware-drivers.md`
