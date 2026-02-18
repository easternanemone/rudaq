# ADR: GUI Connection Reliability (bd-j3xz)

**Status:** Accepted
**Date:** 2024-12-24
**Epic:** bd-j3xz (GUI Connection Stability)

## Context

The rust-daq GUI connects to a headless daemon via gRPC for instrument control and data acquisition. In production deployments, the GUI often runs on a different machine than the daemon (e.g., lab workstation connecting to instrument controller).

**Problems with the original implementation:**
1. Network instability caused silent connection drops with no recovery
2. Daemon restarts required manual GUI reconnection
3. "Zombie" connections (TCP alive but daemon unresponsive) went undetected
4. Connection errors showed raw gRPC messages, confusing users
5. Panels crashed or showed cryptic errors when disconnected

## Decision

Implement a multi-layered connection reliability system with:

### 1. Explicit State Machine (`ConnectionState`)

```
Disconnected ──connect()──> Connecting ──success──> Connected
     ↑                          │                       │
     │                          │ failure               │ health_check_failed
     │                          ↓                       ↓
     └──cancel()────────── Reconnecting <──────────────┘
                               │
                               │ max_retries_exceeded
                               ↓
                             Error
```

**States:**
- `Disconnected` - Initial state, no connection attempt
- `Connecting` - Active connection attempt in progress
- `Connected` - Healthy connection established
- `Reconnecting` - Auto-recovery in progress with backoff
- `Error` - Non-retriable failure (e.g., invalid URL)

### 2. Exponential Backoff with Jitter

Prevents thundering herd and server overload during recovery:

```rust
delay = min(base_delay * 2^attempt, max_delay) + random_jitter
```

**Default configuration:**
- Base delay: 1 second
- Max delay: 30 seconds
- Max attempts: 10
- Jitter: 0-1 second

### 3. Multi-Layer Health Monitoring

| Layer | Mechanism | Detects |
|-------|-----------|---------|
| Transport | gRPC HTTP/2 keepalives (30s interval) | Dead TCP connections |
| Application | Periodic `get_daemon_info` calls (30s) | Zombie connections, daemon hangs |

**Health check flow:**
1. Every 30 seconds, call `get_daemon_info` RPC
2. Track consecutive failures (threshold: 2)
3. On threshold breach, trigger auto-reconnect
4. Reset counters on successful connection

### 4. gRPC Channel Tuning

```rust
ChannelConfig {
    connect_timeout: 10s,      // Initial connection
    request_timeout: 30s,      // Per-RPC timeout
    keepalive_interval: 30s,   // HTTP/2 PING frames
    keepalive_timeout: 10s,    // PING response deadline
    keepalive_while_idle: true // Ping even without traffic
}
```

### 5. User-Friendly Error Mapping

Raw gRPC errors are translated to actionable messages:

| gRPC Error | User Message |
|------------|--------------|
| `connection refused` | "Daemon not running. Start with: cargo run --bin rust-daq-daemon -- daemon" |
| `dns error` | "Cannot resolve hostname. Check the address." |
| `timed out` | "Connection timed out. Check network connectivity." |
| `transport error` | "Network connection lost. Will retry automatically." |

### 6. Graceful Offline Degradation

When disconnected, panels show context-aware help instead of errors:

```rust
if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
    return; // Shows helpful offline UI
}
```

**Offline notice includes:**
- Context-specific icon and message
- Quick Start instructions for daemon
- "What works offline?" expandable section

## Consequences

### Positive
- GUI survives daemon restarts without user intervention
- Clear visual feedback: color-coded status bar (green/yellow/red/gray)
- Users get actionable error messages, not gRPC internals
- Reduced support burden from "connection lost" issues
- Panels remain responsive when offline (no freezing)

### Negative
- Slight increase in background network traffic (health checks)
- More complex connection code (~700 lines in reconnect.rs)
- Health check adds small latency to failure detection (up to 30s)

### Neutral
- Connection state persisted to local storage for reconnection on restart
- Address resolution follows precedence: UI override > storage > env var > default

## Implementation Files

| File | Purpose |
|------|---------|
| `reconnect.rs` | ConnectionState, ConnectionManager, ReconnectConfig, HealthConfig |
| `client.rs` | DaqClient, ChannelConfig, health_check() |
| `connection.rs` | DaemonAddress, URL normalization, persistence |
| `widgets/offline_notice.rs` | OfflineNotice widget, OfflineContext enum |
| `app.rs` | Integration with egui event loop |

## Testing

Unit tests cover:
- State transitions (26 tests in reconnect.rs)
- URL normalization and validation
- Error classification (retriable vs non-retriable)
- Friendly error message mapping

Integration testing requires manual verification with daemon start/stop cycles.

## References

- [gRPC Keepalive Best Practices](https://grpc.io/docs/guides/keepalive/)
- [Exponential Backoff And Jitter](https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/)
- [Circuit Breaker Pattern](https://martinfowler.com/bliki/CircuitBreaker.html)
