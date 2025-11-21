# Example Scripts for script_runner

This directory contains example scripts for testing the `script_runner` CLI tool.

## Available Scripts

### 1. simple_math.rhai
Basic arithmetic operations demonstration.

```bash
cargo run --bin script_runner -- examples/scripts/simple_math.rhai
```

### 2. loops.rhai
Loop demonstration calculating sum of first 100 numbers.

```bash
cargo run --bin script_runner -- examples/scripts/loops.rhai
```

### 3. globals_demo.rhai
Demonstrates using global variables passed from command line.

```bash
cargo run --bin script_runner -- --global max_iterations=50 examples/scripts/globals_demo.rhai
```

### 4. validation_test.rhai
Script for testing syntax validation without execution.

```bash
cargo run --bin script_runner -- --validate examples/scripts/validation_test.rhai
```

## General Usage

```bash
# Basic execution
cargo run --bin script_runner -- <script_file>

# With verbose logging
cargo run --bin script_runner -- --verbose <script_file>

# Validate syntax only
cargo run --bin script_runner -- --validate <script_file>

# Set global variables
cargo run --bin script_runner -- --global key=value <script_file>

# Specify engine (currently only Rhai)
cargo run --bin script_runner -- --engine rhai <script_file>

# Custom operation limit
cargo run --bin script_runner -- --max-operations 50000 <script_file>
```

## Features

- Multiple scripting backend support (currently Rhai)
- Script validation without execution
- Global variable injection from command line
- Detailed error reporting with line numbers
- Configurable logging levels
- Operation safety limits to prevent infinite loops
