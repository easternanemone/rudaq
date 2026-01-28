# Error Tracking Guide

This document describes how to set up and use error tracking in rust-daq applications.

## Overview

rust-daq uses [Sentry](https://sentry.io) for error tracking and performance monitoring. Error tracking is **opt-in** and requires:

1. The `error_tracking` feature flag enabled at build time
2. The `SENTRY_DSN` environment variable set at runtime

If either is missing, error tracking is silently disabled with no impact on application performance.

## Quick Start

### 1. Build with Error Tracking

```bash
# Daemon
cargo build -p daq-bin --features error_tracking

# GUI
cargo build -p daq-egui --features error_tracking
```

### 2. Configure Sentry DSN

```bash
export SENTRY_DSN="https://your-key@sentry.io/your-project"
```

### 3. Run the Application

```bash
# Errors will now be sent to Sentry
./target/debug/rust-daq-daemon daemon --port 50051
```

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SENTRY_DSN` | Yes | - | Sentry Data Source Name (from Sentry project settings) |
| `SENTRY_ENVIRONMENT` | No | `development` | Environment name (e.g., `production`, `staging`) |
| `SENTRY_RELEASE` | No | Package version | Release version for tracking regressions |

## Integration Details

### Error Tracking Module

The `common::error_tracking` module provides:

```rust
use common::error_tracking::{self, MessageLevel};

// Initialize (returns a guard that must stay alive)
let _guard = error_tracking::init("my-app", "1.0.0");

// Capture an error
error_tracking::capture_error(&some_error);

// Capture a message
error_tracking::capture_message("Something happened", MessageLevel::Warning);

// Add breadcrumbs for debugging
error_tracking::add_breadcrumb("user_action", "Clicked start button");

// Set user context
error_tracking::set_user(Some("user123"), Some("user@example.com"), None);

// Add custom context
error_tracking::set_context("hardware", serde_json::json!({
    "camera": "Prime BSI",
    "serial_port": "/dev/ttyUSB0"
}));
```

### Sample Rates

The default configuration:
- **Error sample rate**: 100% (all errors captured)
- **Traces sample rate**: 10% (performance monitoring)

To customize:

```rust
use common::error_tracking::{init_with_config, ErrorTrackingConfig};

let _guard = init_with_config(ErrorTrackingConfig {
    app_name: "daq-daemon".to_string(),
    version: "1.0.0".to_string(),
    environment: "production".to_string(),
    sample_rate: 1.0,          // Capture all errors
    traces_sample_rate: 0.2,   // 20% of transactions
});
```

## What Gets Captured

### Automatic Captures
- Panics (with full backtraces)
- Errors logged at ERROR level via tracing
- Application context (OS, Rust version, etc.)

### Manual Captures
- Custom errors via `capture_error()`
- Messages via `capture_message()`
- Breadcrumbs for debugging context

## Privacy Considerations

**By default, no PII is captured.** To add user context, explicitly call:

```rust
error_tracking::set_user(Some("user_id"), Some("email"), Some("username"));
```

Be careful not to include sensitive data in:
- Breadcrumb messages
- Custom context values
- Error messages

## Sentry Project Setup

### Creating a Sentry Project

1. Go to [sentry.io](https://sentry.io) and create an account
2. Create a new project, select "Rust" as the platform
3. Copy the DSN from Project Settings > Client Keys

### Recommended Sentry Settings

- **Release tracking**: Enable to track error rates across versions
- **Source maps**: Not applicable for Rust (native binaries)
- **Performance monitoring**: Enable for transaction tracking
- **Issue grouping**: Use default fingerprinting rules

## Troubleshooting

### Errors Not Appearing in Sentry

1. **Check DSN is set**: `echo $SENTRY_DSN`
2. **Check feature flag**: Ensure built with `--features error_tracking`
3. **Check logs**: Look for "Initializing error tracking" message
4. **Check network**: Sentry requires outbound HTTPS to `sentry.io`

### Performance Impact

With default settings, error tracking has minimal performance impact:
- Initialization: ~10ms
- Error capture: ~1-5ms (async)
- Memory: ~2MB additional

For performance-critical paths, consider using breadcrumbs instead of full error captures.

## CI/CD Integration

### GitHub Actions Secret

Add `SENTRY_DSN` as a repository secret, then use in workflows:

```yaml
- name: Run with error tracking
  env:
    SENTRY_DSN: ${{ secrets.SENTRY_DSN }}
  run: ./my-app
```

### Release Tracking

The `SENTRY_RELEASE` is automatically set to the package version. For custom releases:

```yaml
env:
  SENTRY_RELEASE: "rust-daq@${{ github.sha }}"
```

## See Also

- [Sentry Rust SDK Documentation](https://docs.sentry.io/platforms/rust/)
- [Error Recovery](../architecture/adr-connection-reliability.md) - Application-level error handling
- [AGENTS.md](../../AGENTS.md) - Build and test commands
