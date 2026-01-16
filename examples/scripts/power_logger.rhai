// power_logger.rhai - Example script-based module
//
// A simple power logging module that demonstrates the script module interface.
// This module reads power values and logs them with timestamps.
//
// Usage:
//   let loader = ScriptPluginLoader::new();
//   loader.add_search_path("./examples/scripts");
//   loader.discover().await?;
//   let module = loader.create_module("power_logger").await?;

// Module metadata - required by the script module interface
fn module_type_info() {
    #{
        type_id: "power_logger",
        display_name: "Power Logger",
        description: "Logs power readings with timestamps. Demonstrates the script module interface.",
        version: "1.0.0",
        parameters: [
            #{
                param_id: "log_interval_ms",
                display_name: "Log Interval",
                description: "Time between log entries in milliseconds",
                param_type: "integer",
                default_value: "1000",
                min_value: "100",
                max_value: "60000",
                units: "ms",
                required: false
            },
            #{
                param_id: "log_format",
                display_name: "Log Format",
                description: "Format for log output",
                param_type: "enum",
                default_value: "simple",
                enum_values: ["simple", "verbose", "csv"],
                required: false
            }
        ],
        required_roles: [
            #{
                role_id: "power_source",
                display_name: "Power Source",
                description: "Device providing power readings",
                required_capability: "readable",
                allows_multiple: false
            }
        ],
        optional_roles: [],
        event_types: ["log_started", "log_stopped", "power_reading"],
        data_types: ["power_log"]
    }
}

// Configuration storage
let config = #{
    log_interval_ms: 1000,
    log_format: "simple"
};

// Module state
let is_running = false;
let log_count = 0;

// Configure the module with parameters
fn configure(params) {
    let warnings = [];

    if "log_interval_ms" in params {
        let interval = parse_int(params["log_interval_ms"]);
        if interval >= 100 && interval <= 60000 {
            config.log_interval_ms = interval;
        } else {
            warnings.push("log_interval_ms clamped to valid range [100, 60000]");
            config.log_interval_ms = if interval < 100 { 100 } else { 60000 };
        }
    }

    if "log_format" in params {
        let format = params["log_format"];
        if format == "simple" || format == "verbose" || format == "csv" {
            config.log_format = format;
        } else {
            warnings.push(`Invalid log_format '${format}', using 'simple'`);
        }
    }

    warnings
}

// Get current configuration
fn get_config() {
    #{
        log_interval_ms: `${config.log_interval_ms}`,
        log_format: config.log_format
    }
}

// Stage - prepare resources
fn stage(ctx) {
    print(`[${ctx.module_id}] Staging power logger...`);
    log_count = 0;
}

// Start module execution
fn start(ctx) {
    print(`[${ctx.module_id}] Starting power logger with interval ${config.log_interval_ms}ms`);
    is_running = true;

    // In a real implementation, this would loop and read from hardware
    // For this example, we just log the start
    let timestamp = timestamp();

    if config.log_format == "csv" {
        print("timestamp,module_id,power_mW");
    }

    print(format_log_entry(ctx.module_id, 0.0, timestamp));
    log_count += 1;
}

// Format a log entry based on configured format
fn format_log_entry(module_id, power, timestamp) {
    if config.log_format == "simple" {
        `Power: ${power} mW`
    } else if config.log_format == "verbose" {
        `[${timestamp}] Module ${module_id}: Power reading = ${power} mW`
    } else if config.log_format == "csv" {
        `${timestamp},${module_id},${power}`
    } else {
        `Power: ${power} mW`
    }
}

// Pause module
fn pause() {
    print("Power logger paused");
    is_running = false;
}

// Resume module
fn resume() {
    print("Power logger resumed");
    is_running = true;
}

// Stop module
fn stop() {
    print(`Power logger stopped. Total logs: ${log_count}`);
    is_running = false;
}

// Unstage - cleanup
fn unstage(ctx) {
    print(`[${ctx.module_id}] Unstaging power logger...`);
}
