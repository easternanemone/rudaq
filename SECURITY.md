# Security Policy

This document describes security considerations for deploying and operating rust-daq.

## Plugin Security Model

**WARNING: Plugins are trusted code.**

The rust-daq plugin system uses `abi_stable` for ABI compatibility but provides **no isolation or sandboxing**. Plugins loaded by rust-daq run in the same process space as the daemon with full access to:

- **All daemon memory** - Plugins can read and write any memory the daemon can access
- **All file system access** - Plugins inherit the daemon's file system permissions
- **Network access** - Plugins can make arbitrary network connections
- **Hardware devices** - Plugins have full access to connected scientific instruments

### Security Implications

Because plugins run in-process:

1. **A malicious plugin can crash the daemon** - Segfaults, panics, or calls to `exit()` terminate the entire process
2. **A malicious plugin can corrupt daemon memory** - No memory isolation exists between plugin and host
3. **A malicious plugin can exfiltrate data** - Full network access enables data theft
4. **A buggy plugin affects the entire system** - Memory leaks, deadlocks, or infinite loops impact all operations

### Why This Design?

This trust model is an intentional trade-off for scientific instrumentation:

- **Performance** - Zero-copy data sharing between plugins and host is critical for high-speed DAQ
- **Hardware access** - Scientific instruments require low-latency, direct hardware control
- **Simplicity** - In-process plugins avoid complex IPC serialization

This is acceptable in controlled lab environments where plugin authors are known and trusted.

### Recommendations

#### For System Administrators

1. **Only install plugins from trusted sources** - Verify the plugin author and source before deployment
2. **Review plugin source code** - Audit plugin code before building and deploying
3. **Restrict plugin directory permissions** - Use appropriate file permissions:
   ```bash
   # Example: Root-owned plugin directory
   sudo mkdir -p /usr/lib/daq/plugins
   sudo chown root:root /usr/lib/daq/plugins
   sudo chmod 755 /usr/lib/daq/plugins
   ```
4. **Monitor daemon logs** - Watch for unexpected plugin load messages
5. **Run daemon with minimal privileges** - Use a dedicated service account, not root
6. **Use `--no-plugins` in high-security environments** - Disable plugin loading entirely when not needed

#### For Plugin Developers

1. **Follow secure coding practices** - Validate all inputs, handle errors gracefully
2. **Avoid panics** - Use `Result` types instead of `unwrap()` or `expect()`
3. **Document security implications** - Be explicit about what resources your plugin accesses
4. **Use dependency auditing** - Run `cargo audit` on plugin dependencies
5. **Minimize dependencies** - Each dependency is additional attack surface

### Plugin Loading Behavior

When the daemon starts, it:

1. Scans configured plugin directories for dynamic libraries (`.so`, `.dylib`, `.dll`)
2. Verifies ABI compatibility using `abi_stable`
3. Loads compatible plugins into the daemon process
4. Logs each loaded plugin with its version and path

Example log output:
```
INFO  Loading plugin: my-plugin v1.0.0 from /usr/lib/daq/plugins/libmy_plugin.so
```

### Disabling Plugins

To run the daemon without loading any plugins:

```bash
rust-daq daemon --port 50051 --no-plugins
```

This is recommended for:
- High-security deployments
- Debugging plugin-related issues
- Minimal attack surface requirements

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
- [docs/architecture/](docs/architecture/) - Architecture Decision Records
- Plugin API: `crates/daq-plugin-api/`
