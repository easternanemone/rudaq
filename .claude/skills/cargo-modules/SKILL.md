---
name: cargo-modules
description: Visualize and analyze Rust crate structure using cargo-modules for understanding module hierarchy, dependencies, and detecting orphaned files
tags: [rust, crate-structure, modules, dependencies, analysis]
---

# cargo-modules: Rust Crate Structure Analysis

Use `cargo modules` to visualize and analyze internal crate structure in this Rust workspace. This tool helps understand module hierarchies, internal dependencies, and detect orphaned files.

## When to Use This Skill

- **Before refactoring**: Understand current module structure and dependencies
- **Exploring codebase**: Get high-level overview of module organization
- **Detecting issues**: Find orphaned source files not linked in the module tree
- **Understanding dependencies**: Visualize how modules depend on each other
- **Debugging module errors**: Identify visibility issues and circular dependencies

## Core Commands

### 1. Visualize Module Structure

Show hierarchical tree of modules, types, functions, and traits:

```bash
# Basic structure for main crate
cargo modules structure --package rust_daq

# Filter views for clarity
cargo modules structure --package rust_daq --no-fns          # Hide functions
cargo modules structure --package rust_daq --no-types        # Hide types
cargo modules structure --package rust_daq --no-traits       # Hide traits

# Focus on specific module
cargo modules structure --package rust_daq --focus-on config

# Limit depth for large crates
cargo modules structure --package rust_daq --max-depth 3

# Sort by visibility or kind instead of name
cargo modules structure --package rust_daq --sort-by visibility
```

### 2. Analyze Internal Dependencies

Generate DOT graph showing how modules depend on each other:

```bash
# Basic dependency graph (outputs DOT format)
cargo modules dependencies --package rust_daq

# Filter for cleaner graph
cargo modules dependencies --package rust_daq --no-fns --no-types

# Focus on specific module's dependencies
cargo modules dependencies --package rust_daq --focus-on hardware

# Check for circular dependencies
cargo modules dependencies --package rust_daq --acyclic

# Different layout algorithms
cargo modules dependencies --package rust_daq --layout dot      # Hierarchical
cargo modules dependencies --package rust_daq --layout fdp      # Force-directed
cargo modules dependencies --package rust_daq --layout circo    # Circular
```

**Visualizing graphs**: If `xdot` is installed, pipe output directly:
```bash
cargo modules dependencies --package rust_daq | xdot -
```

### 3. Detect Orphaned Files

Find `.rs` files not linked into the module tree:

```bash
# Check for orphaned files
cargo modules orphans --package rust_daq

# Fail build if orphans found (useful for CI)
cargo modules orphans --package rust_daq --deny
```

## Workspace Considerations

This workspace has multiple packages. **Always specify `--package` or `--lib` or `--bin`**:

```bash
# Analyze specific package
cargo modules structure --package daq-core
cargo modules structure --package hardware

# Analyze library only
cargo modules structure --lib

# Analyze specific binary
cargo modules structure --bin server
```

**Available packages**:
- `rust_daq` - Main runtime façade
- `daq-core` - Foundation types
- `hardware` - HAL and drivers
- `driver-pvcam` - PVCAM camera driver
- `server` - gRPC server
- `experiment` - RunEngine and Plans
- `scripting` - Rhai integration
- `storage` - Data persistence
- `ui` - GUI application
- `protocol` - Protobuf definitions
- `bin` - CLI binaries

## Feature-Specific Analysis

Analyze structure with specific features enabled:

```bash
# With all features (requires PVCAM SDK)
cargo modules structure --package rust_daq --all-features

# With specific features
cargo modules structure --package rust_daq --features "networking,instrument_photometrics"

# Without default features
cargo modules structure --package rust_daq --no-default-features

# Test configuration
cargo modules structure --package rust_daq --cfg-test
```

## Common Use Cases

### Understanding a New Module

```bash
# See what's exported from daq-core
cargo modules structure --package daq-core --max-depth 2

# Focus on specific subsystem
cargo modules structure --package hardware --focus-on drivers
```

### Pre-Refactoring Analysis

```bash
# Full structure before changes
cargo modules structure --package storage > structure-before.txt

# Check dependencies
cargo modules dependencies --package storage > deps-before.dot

# After refactoring, compare
cargo modules structure --package storage > structure-after.txt
diff structure-before.txt structure-after.txt
```

### Finding Dead Code

```bash
# Check for orphaned files
cargo modules orphans --package hardware

# Check for circular dependencies
cargo modules dependencies --package daq-core --acyclic
```

### Investigating Visibility Issues

```bash
# Sort by visibility to see pub vs private
cargo modules structure --package server --sort-by visibility

# Filter to only public API
cargo modules dependencies --package daq-core --no-private
```

## Output Interpretation

### Structure Tree Symbols
- `mod` - Module definition
- `struct`, `enum`, `union` - Type definitions
- `trait` - Trait definitions
- `fn`, `async fn`, `const fn` - Functions
- `pub`, `pub(crate)`, `pub(super)`, `pub(self)` - Visibility levels

### Dependency Graph (DOT format)
- **Nodes**: Modules, types, functions, traits
- **Edges**: Dependencies between items
- **Colors**: Indicate different types of dependencies

## Best Practices

1. **Always specify package in workspace**: Avoid "Multiple packages present" errors
2. **Filter for readability**: Use `--no-fns`, `--no-types` to reduce noise
3. **Focus analysis**: Use `--focus-on` and `--max-depth` for large crates
4. **Check orphans regularly**: Catch unlinked files early
5. **Version control**: Save structure snapshots before major refactors
6. **CI integration**: Use `cargo modules orphans --deny` in CI pipelines

## Troubleshooting

**Error: "Multiple packages present in workspace"**
→ Add `--package <name>` or use `--lib` or `--bin <name>`

**Graph too large/complex**
→ Use filters (`--no-fns`, `--no-types`) and `--max-depth`

**Missing dependencies in graph**
→ Check feature flags; some modules only appear with specific features

**"Orphan" file is intentional**
→ Either link it properly or document why it's excluded (e.g., code generation templates)

## Integration with Development Workflow

```bash
# Pre-commit hook: check for orphans
cargo modules orphans --package rust_daq --deny

# Documentation: generate current structure
cargo modules structure --package rust_daq --no-fns > docs/architecture/current-structure.txt

# Code review: compare dependency changes
cargo modules dependencies --package daq-core | diff - deps-baseline.dot
```

## Additional Resources

- GitHub: https://github.com/regexident/cargo-modules
- Installation: `cargo install cargo-modules`
- For graph visualization: Install `xdot` or Graphviz tools
