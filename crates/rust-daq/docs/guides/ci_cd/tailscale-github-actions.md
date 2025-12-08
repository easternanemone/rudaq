# Tailscale Integration for GitHub Actions

This document describes how to enable hardware-in-the-loop testing in GitHub Actions using Tailscale.

## Overview

The GitHub Actions CI/CD pipeline includes a `hardware-tests` job that connects to the Tailnet to enable testing against physical laboratory instruments. This allows automated testing of instrument drivers against real hardware.

## Setup Instructions

### 1. Create Tailscale OAuth Credentials

1. Log in to the [Tailscale Admin Console](https://login.tailscale.com/admin/settings/oauth)
2. Navigate to **Settings** → **OAuth clients**
3. Click **Generate OAuth client**
4. Configure the OAuth client:
   - **Description**: GitHub Actions CI/CD
   - **Tags**: `tag:ci` (create this tag if it doesn't exist)
   - **Scopes**:
     - `devices:read`
     - `devices:write` (optional, for managing nodes)
5. Save the OAuth client and securely copy the **Client ID** and **Client Secret**

### 2. Configure GitHub Secrets

Add the following secrets to your GitHub repository:

1. Navigate to **Settings** → **Secrets and variables** → **Actions**
2. Add the following repository secrets:
   - `TS_OAUTH_CLIENT_ID`: The OAuth Client ID from step 1
   - `TS_OAUTH_SECRET`: The OAuth Client Secret from step 1

### 3. Configure Tailscale ACLs (Optional but Recommended)

Add ACL rules to restrict CI runner access to only necessary hardware:

```json
{
  "tagOwners": {
    "tag:ci": ["autogroup:admin"]
  },
  "acls": [
    {
      "action": "accept",
      "src": ["tag:ci"],
      "dst": ["tag:lab-equipment:*"]
    }
  ]
}
```

This allows CI runners (tagged `tag:ci`) to access devices tagged `tag:lab-equipment`.

## Workflow Behavior

The `hardware-tests` job:

- **Runs only on**: `main` branch pushes (not on PRs to conserve resources)
- **Dependencies**: Requires `lint` job to pass first
- **Runs in parallel** with other test jobs

### Workflow Steps

1. **Connect to Tailscale**: Establishes VPN connection to Tailnet
2. **Test connectivity**: Runs `tailscale status` to verify connection
3. **Ping hardware** (when configured): Tests reachability of lab equipment
4. **Run hardware tests** (future): Executes integration tests with `--ignored` flag

## Adding Hardware Tests

To add actual hardware integration tests:

1. Create tests with `#[ignore]` attribute:
   ```rust
   #[test]
   #[ignore] // Requires physical hardware
   fn test_esp300_motion_control() {
       // Test code that connects to real ESP300 controller
   }
   ```

2. Update the workflow's ping command with your hardware IPs:
   ```yaml
   - name: Test Tailnet connectivity
     run: |
       echo "Testing connectivity to lab equipment..."
       ping -c 3 100.x.y.z  # Replace with actual Tailnet IP
   ```

3. Enable hardware test execution:
   ```yaml
   - name: Run hardware integration tests
     run: cargo test --test hardware_integration -- --ignored
   ```

## Security Considerations

- OAuth credentials are stored as GitHub encrypted secrets
- CI runners use ephemeral Tailscale nodes (automatically cleaned up)
- ACLs restrict CI access to only tagged lab equipment
- Tests run only on `main` branch to prevent unauthorized access from PR forks

## Troubleshooting

### Connection Fails

Check that:
1. OAuth credentials are correctly configured in GitHub secrets
2. Tailscale ACLs allow `tag:ci` to access destination devices
3. Target hardware is online and connected to Tailnet

### Tests Timeout

- Increase timeouts in test code
- Verify network latency between GitHub Actions runner and hardware
- Check if instruments require warmup time

### Logs

View Tailscale connection logs in GitHub Actions:
```yaml
- name: Debug Tailscale
  run: |
    tailscale status
    tailscale ping <device-ip>
```

## Future Enhancements

- [ ] Add matrix of hardware configurations
- [ ] Implement test fixtures for common instruments
- [ ] Add performance benchmarking against real hardware
- [ ] Enable selective hardware tests via workflow dispatch
- [ ] Add hardware availability checking before test execution

## References

- [Tailscale GitHub Action](https://tailscale.com/kb/1276/tailscale-github-action)
- [GitHub Actions Secrets](https://docs.github.com/en/actions/security-guides/encrypted-secrets)
- [Tailscale ACL Documentation](https://tailscale.com/kb/1018/acls)
