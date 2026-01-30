# Pre-commit Hooks Guide

Pre-commit hooks automatically run code quality checks before each commit, catching issues early and maintaining consistent code quality.

## Quick Start

```bash
# Install hooks (one-time setup)
bash scripts/install-hooks.sh

# That's it! Hooks now run automatically on every commit
```

## Installation Options

### Full Hooks (Default)

Runs comprehensive checks including formatting, linting, and tests:

```bash
bash scripts/install-hooks.sh
```

**Checks performed:**
- Code formatting (`cargo fmt`)
- Linting (`cargo clippy`)
- Unit tests (fast tests only)
- File issues (trailing whitespace, large files, merge conflicts)
- Secret detection (private keys, credentials)
- TOML validation

**Commit time:** ~30-60 seconds for changed files

### Quick Hooks (Fast Development)

For rapid iteration, use formatting-only hooks:

```bash
bash scripts/install-hooks.sh quick
```

**Checks performed:**
- Code formatting (`cargo fmt`) only

**Commit time:** ~5-10 seconds

### Switching Between Configurations

```bash
# Switch to full hooks
bash scripts/install-hooks.sh

# Switch to quick hooks
bash scripts/install-hooks.sh quick
```

## Manual Hook Execution

Run hooks without committing:

```bash
# Run on all files
pre-commit run --all-files

# Run on staged files only (what git commit would check)
pre-commit run

# Run specific hook
pre-commit run cargo-fmt
pre-commit run cargo-clippy
```

## Skipping Hooks

### Emergency Bypass (Use Sparingly)

```bash
# Skip all hooks for this commit
git commit --no-verify -m "Emergency fix"
```

**When to use:**
- Emergency production fixes that can't wait
- Reverting a broken commit
- Working on unrelated documentation changes

**⚠️ Warning:** Skipped hooks don't run linting or tests. Use only when necessary.

### Skipping Specific Files

Add to your commit:

```bash
# Skip hooks for specific paths
SKIP=cargo-clippy git commit -m "WIP: work in progress"
```

## Hook Configuration

### Full Hooks Configuration

Located in `.pre-commit-config.yaml`:

```yaml
repos:
  # File checks
  - repo: https://github.com/pre-commit/pre-commit-hooks
    hooks:
      - trailing-whitespace
      - end-of-file-fixer
      - check-yaml
      - check-toml
      - check-added-large-files (max 1MB)
      - check-merge-conflict
      - detect-private-key

  # Rust formatting
  - repo: local
    hooks:
      - cargo fmt --all

  # Rust linting
  - repo: local
    hooks:
      - cargo clippy (excludes ui)

  # Fast unit tests
  - repo: local
    hooks:
      - cargo test --lib --bins
```

### Quick Hooks Configuration

Located in `.pre-commit-quick.yaml`:

```yaml
repos:
  # Essential file checks only
  - check-merge-conflict
  - detect-private-key
  
  # Formatting only
  - cargo fmt --all
```

## Troubleshooting

### Issue: "pre-commit: command not found"

**Solution:** Install pre-commit:

```bash
pip install pre-commit
# or
brew install pre-commit  # macOS
```

### Issue: "Hook failed" or tests fail

**Solution:** Run the check manually to see details:

```bash
# See what failed
cargo clippy --workspace --all-targets --features full --exclude ui

# Fix formatting issues
cargo fmt --all

# Run tests that failed
cargo test --lib --bins --workspace --exclude ui
```

### Issue: Hooks are slow

**Solutions:**
1. **Switch to quick hooks** for development:
   ```bash
   bash scripts/install-hooks.sh quick
   ```

2. **Commit smaller changes** more frequently

3. **Skip hooks temporarily** for WIP commits:
   ```bash
   git commit --no-verify -m "WIP"
   ```

### Issue: Hooks run on every file

By default, hooks only run on **changed files**. To run on all files:

```bash
pre-commit run --all-files
```

## CI Integration

Pre-commit hooks also run in CI to catch issues that might slip through:

```yaml
# .github/workflows/ci.yml
- name: Run pre-commit checks
  run: pre-commit run --all-files
```

**Difference from local hooks:**
- CI runs on all files (not just changed)
- CI may use stricter settings
- Some hooks (like cargo-clippy) are skipped in CI if they run separately

## Customization

### Adding Your Own Hooks

Edit `.pre-commit-config.yaml`:

```yaml
- repo: local
  hooks:
    - id: my-custom-check
      name: My Custom Check
      entry: bash -c 'echo "Running custom check"'
      language: system
      types: [rust]
      pass_filenames: false
```

### Excluding Specific Checks

To disable a hook, comment it out in `.pre-commit-config.yaml`:

```yaml
# - id: cargo-test  # Disabled for faster commits
```

### Per-Hook Configuration

Some hooks support additional configuration:

```yaml
- id: check-added-large-files
  args: ['--maxkb=2000']  # Increase size limit to 2MB
```

## Best Practices

1. **Install hooks early**: Set up pre-commit hooks when you first clone the repo
2. **Run manually before pushing**: `pre-commit run --all-files` before opening PRs
3. **Keep hooks fast**: Use quick hooks during development, full hooks before PR
4. **Don't skip hooks routinely**: If you find yourself skipping often, adjust the configuration
5. **Update regularly**: Run `pre-commit autoupdate` to get latest hook versions

## Uninstalling

```bash
# Remove hooks
pre-commit uninstall

# Hooks are no longer active
```

## See Also

- [CONTRIBUTING.md](../../CONTRIBUTING.md) - Development workflow
- [AGENTS.md](../../AGENTS.md) - Build and test commands
- [pre-commit documentation](https://pre-commit.com/) - Official documentation
