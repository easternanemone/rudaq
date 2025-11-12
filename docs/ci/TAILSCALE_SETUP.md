# Tailscale Setup for GitHub Actions

This document outlines the process for setting up Tailscale in GitHub Actions to enable access to hardware in the loop (HIL) testing environments.

## GitHub Actions Secrets

The following secrets must be configured in the GitHub repository settings under "Secrets and variables > Actions" for the Tailscale integration to work correctly:

-   `TAILSCALE_AUTHKEY`: The Tailscale authentication key. This is a one-off key that is generated from the Tailscale admin console. It is used to authenticate the GitHub Actions runner to your Tailscale network.
-   `MAITAI_SSH_HOST`: The Tailscale host name of the SSH server to connect to. In our case, this is `maitai-eos`.
-   `MAITAI_SSH_USER`: The username to use when connecting to the SSH server.

## Workflow Configuration

The `.github/workflows/ci.yml` file contains a `hardware-tests` job that uses the `tailscale/github-action` to connect to the Tailscale network. The action is configured with the `TAILSCALE_AUTHKEY` secret.

Once connected, subsequent steps in the job can access resources on the Tailscale network. For example, the workflow includes a step to SSH into the `maitai-eos` host and verify access to serial ports.

## Example Workflow Step

Here is an example of the Tailscale setup step in the workflow:

```yaml
- name: Connect to Tailscale
  uses: tailscale/github-action@v2
  with:
    authkey: ${{ secrets.TAILSCALE_AUTHKEY }}
```

And an example of an SSH command to access a machine on the tailnet:

```yaml
- name: Test SSH access to maitai-eos
  env:
    SSH_HOST: ${{ secrets.MAITAI_SSH_HOST }}
    SSH_USER: ${{ secrets.MAITAI_SSH_USER }}
  run: |
    ssh -o StrictHostKeyChecking=no $SSH_USER@$SSH_HOST 'echo "SSH connection successful"'
```
