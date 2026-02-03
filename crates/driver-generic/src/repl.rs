//! Interactive REPL for driver development and debugging.
//!
//! This module provides an interactive shell for testing declarative serial drivers
//! without running the full DAQ system.

use anyhow::{anyhow, Context, Result};
use colored::Colorize;
use plugin_api::config::InstrumentConfig;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;

use crate::driver::GenericSerialDriver;

/// REPL state and command dispatcher.
pub struct Repl {
    /// Loaded device configuration (V1 TOML schema)
    config: InstrumentConfig,
    /// Optional serial driver (None in mock mode or before connect)
    driver: Arc<Mutex<Option<GenericSerialDriver>>>,
    /// Serial port path (if specified)
    port_path: Option<String>,
    /// Mock mode flag
    mock_mode: bool,
    /// Config file path (for reload)
    config_path: std::path::PathBuf,
}

impl Repl {
    /// Create a new REPL instance.
    pub async fn new(config_path: &Path, port: Option<String>, mock_mode: bool) -> Result<Self> {
        // Load and parse config as typed InstrumentConfig (async I/O)
        let config_str = fs::read_to_string(config_path)
            .await
            .with_context(|| format!("Failed to read config: {}", config_path.display()))?;

        let mut config: InstrumentConfig = toml::from_str(&config_str)
            .with_context(|| format!("Failed to parse TOML: {}", config_path.display()))?;
        config.expand_shorthand();

        Ok(Self {
            config,
            driver: Arc::new(Mutex::new(None)),
            port_path: port,
            mock_mode,
            config_path: config_path.to_path_buf(),
        })
    }

    /// Run the interactive REPL loop.
    pub async fn run(&mut self) -> Result<()> {
        println!(
            "{}",
            "daq-driver-dev - Interactive Driver Development REPL".bold()
        );
        println!("{}", "=".repeat(50));
        println!();

        // Initial validation
        self.cmd_validate().await?;
        println!();

        // Show connection status
        if self.mock_mode {
            println!("{}", "Mode: MOCK (no hardware)".yellow());
        } else if let Some(ref port) = self.port_path {
            println!("Port: {}", port);
            println!("{}", "Status: Disconnected".yellow());
            println!("{}", "Hint: Use 'connect' to open serial port".dimmed());
        } else {
            println!(
                "{}",
                "No port specified (use --port or 'connect <port>')".yellow()
            );
        }

        println!();
        println!(
            "{}",
            "Type 'help' for available commands, 'exit' to quit.".dimmed()
        );
        println!();

        // Create readline editor with history
        let mut rl = DefaultEditor::new()?;

        loop {
            let readline = rl.readline(">> ");
            match readline {
                Ok(line) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }

                    // Add to history
                    let _ = rl.add_history_entry(line);

                    // Parse and execute command
                    match self.execute_command(line).await {
                        Ok(true) => {}      // Continue
                        Ok(false) => break, // Exit requested
                        Err(e) => println!("{} {}", "Error:".red().bold(), e),
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("^C");
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    println!("exit");
                    break;
                }
                Err(err) => {
                    println!("{} {}", "Error:".red().bold(), err);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Execute a REPL command. Returns Ok(true) to continue, Ok(false) to exit.
    async fn execute_command(&mut self, line: &str) -> Result<bool> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(true);
        }

        match parts[0] {
            "help" => self.cmd_help()?,
            "validate" => self.cmd_validate().await?,
            "connect" => self.cmd_connect(parts.get(1).copied()).await?,
            "disconnect" => self.cmd_disconnect().await?,
            "raw" => self.cmd_raw(&parts[1..]).await?,
            "get" => self.cmd_get(parts.get(1).copied()).await?,
            "set" => {
                if parts.len() < 3 {
                    return Err(anyhow!("Usage: set <param> <value>"));
                } else {
                    self.cmd_set(parts[1], parts[2]).await?
                }
            }
            "list" => self.cmd_list(parts.get(1).copied())?,
            "info" => self.cmd_info(parts.get(1).copied())?,
            "reload" => self.cmd_reload().await?,
            "exit" | "quit" => {
                println!("Goodbye!");
                return Ok(false); // Signal clean exit
            }
            _ => {
                return Err(anyhow!(
                    "Unknown command: '{}'. Type 'help' for available commands.",
                    parts[0]
                ))
            }
        }

        Ok(true)
    }

    /// Show help message.
    fn cmd_help(&self) -> Result<()> {
        println!("{}", "Available Commands:".bold());
        println!();
        println!(
            "  {}              - Validate config schema and check for errors",
            "validate".cyan()
        );
        println!(
            "  {}             - Connect to serial port (or specify port)",
            "connect [<port>]".cyan()
        );
        println!(
            "  {}          - Disconnect from serial port",
            "disconnect".cyan()
        );
        println!(
            "  {}          - Send raw command to device",
            "raw <command>".cyan()
        );
        println!(
            "  {}           - Execute a get command (Readable, position, etc.)",
            "get <param>".cyan()
        );
        println!(
            "  {}   - Execute a set command (WavelengthTunable, etc.)",
            "set <param> <value>".cyan()
        );
        println!(
            "  {}          - List available items (commands, parameters, traits)",
            "list [<type>]".cyan()
        );
        println!(
            "  {}           - Show detailed info about a command",
            "info <name>".cyan()
        );
        println!(
            "  {}              - Reload configuration from disk",
            "reload".cyan()
        );
        println!(
            "  {}                - Show this help message",
            "help".cyan()
        );
        println!("  {}                - Exit the REPL", "exit".cyan());
        println!();
        println!(
            "{}",
            "List types: commands, parameters, capabilities, traits".dimmed()
        );
        Ok(())
    }

    /// Validate the configuration.
    async fn cmd_validate(&self) -> Result<()> {
        println!("{}", "Validating configuration...".bold());
        println!();

        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Check required fields (already validated by serde deserialization)
        if self.config.device.name.is_empty() {
            errors.push("Device name is empty".to_string());
        }

        if self.config.device.protocol.is_empty() {
            errors.push("Device protocol is empty".to_string());
        }

        if self.config.commands.is_empty() {
            warnings.push("No commands defined".to_string());
        }

        if let Err(err) = self.config.validate() {
            errors.push(err);
        }

        // Validate trait mappings reference valid commands
        let commands = &self.config.commands;

        if let Some(ref movable) = self.config.trait_mapping.movable {
            for (method_name, mapping) in movable {
                if let Some(ref cmd_name) = mapping.command {
                    if !commands.contains_key(cmd_name) {
                        errors.push(format!(
                            "Movable.{} references unknown command '{}'",
                            method_name, cmd_name
                        ));
                    }
                }
                if let Some(ref cmd_name) = mapping.poll_command {
                    if !commands.contains_key(cmd_name) {
                        errors.push(format!(
                            "Movable.{} poll_command references unknown command '{}'",
                            method_name, cmd_name
                        ));
                    }
                }
            }
        }

        for (trait_name, methods) in &self.config.trait_mapping.traits {
            for (method_name, mapping) in methods {
                if let Some(ref cmd_name) = mapping.command {
                    if !commands.contains_key(cmd_name) {
                        errors.push(format!(
                            "{}.{} references unknown command '{}'",
                            trait_name, method_name, cmd_name
                        ));
                    }
                }
                if let Some(ref cmd_name) = mapping.poll_command {
                    if !commands.contains_key(cmd_name) {
                        errors.push(format!(
                            "{}.{} poll_command references unknown command '{}'",
                            trait_name, method_name, cmd_name
                        ));
                    }
                }
            }
        }

        // Print results
        if errors.is_empty() && warnings.is_empty() {
            println!("{} TOML syntax valid", "✓".green().bold());
            println!("{} Required fields present", "✓".green().bold());
            println!(
                "{} Capability mappings reference valid commands",
                "✓".green().bold()
            );
            println!();
            println!("{}", "Configuration appears valid!".green().bold());
        } else {
            for error in &errors {
                println!("{} {}", "✗".red().bold(), error);
            }
            for warning in &warnings {
                println!("{} {}", "!".yellow().bold(), warning);
            }

            if !errors.is_empty() {
                return Err(anyhow!("Validation failed with {} error(s)", errors.len()));
            }
        }

        Ok(())
    }

    /// Connect to the serial port.
    async fn cmd_connect(&mut self, port_arg: Option<&str>) -> Result<()> {
        if self.mock_mode {
            println!("{}", "Mock mode enabled - no actual connection".yellow());
            // In mock mode, we don't actually connect
            return Ok(());
        }

        // Determine port path
        let port_path = port_arg
            .or(self.port_path.as_deref())
            .ok_or_else(|| anyhow!("No port specified. Use: connect <port>"))?;

        // Get baud rate from config
        let baud_rate = self.config.connection.baud_rate;

        println!("Connecting to {} @ {} baud...", port_path, baud_rate);

        // Open serial port using common::serial helper
        use common::serial::{open_serial_async, wrap_shared_unbuffered};
        let port = open_serial_async(port_path, baud_rate, &self.config.device.name).await?;

        // Wrap in SharedPortUnbuffered (Arc<Mutex<DynSerial>>)
        let shared_port = wrap_shared_unbuffered(port);

        // Determine device address (RS-485 multidrop)
        let address = self
            .config
            .connection
            .bus
            .as_ref()
            .map(|bus| bus.default_address.clone())
            .unwrap_or_else(|| "0".to_string());

        // Create GenericSerialDriver with the current config
        let driver = GenericSerialDriver::new(self.config.clone(), shared_port, &address)?;

        // Store driver
        *self.driver.lock().await = Some(driver);

        println!("{} Connected to {}", "✓".green().bold(), port_path);

        // Update port_path if it was specified as an argument
        if port_arg.is_some() {
            self.port_path = Some(port_path.to_string());
        }

        Ok(())
    }

    /// Disconnect from the serial port.
    async fn cmd_disconnect(&mut self) -> Result<()> {
        let mut driver = self.driver.lock().await;
        if driver.is_none() {
            println!("{}", "Not connected".yellow());
            return Ok(());
        }

        *driver = None;
        println!("{} Disconnected", "✓".green().bold());
        Ok(())
    }

    /// Send a raw command to the device.
    async fn cmd_raw(&self, args: &[&str]) -> Result<()> {
        if args.is_empty() {
            return Err(anyhow!("Usage: raw <command>"));
        }

        let command = args.join(" ");

        let driver = {
            let guard = self.driver.lock().await;
            guard
                .as_ref()
                .cloned()
                .ok_or_else(|| anyhow!("Not connected. Use 'connect' first."))?
        };

        println!("{} {}", "Sending:".dimmed(), command);

        let response = driver.transaction(&command).await?;
        if response.is_empty() {
            println!("{}", "No response".dimmed());
        } else {
            println!("{} {}", "Response:".dimmed(), response);
        }

        Ok(())
    }

    /// Execute a get command (read parameter).
    async fn cmd_get(&self, param: Option<&str>) -> Result<()> {
        let param = param.ok_or_else(|| anyhow!("Usage: get <param>"))?;

        let driver = {
            let guard = self.driver.lock().await;
            guard
                .as_ref()
                .cloned()
                .ok_or_else(|| anyhow!("Not connected. Use 'connect' first."))?
        };

        if let Some(param_def) = self.config.parameters.get(param) {
            if let Some(cmd) = param_def.get_command() {
                let response = driver.transaction(cmd).await?;
                if response.is_empty() {
                    println!("{}", "No response".dimmed());
                } else {
                    println!("{} {}", "Response:".dimmed(), response);
                }
                if let Ok(value) = response.parse::<f64>() {
                    driver.set_parameter(param, value).await;
                }
            } else {
                println!(
                    "{} Parameter '{}' has no get command in config.",
                    "!".yellow(),
                    param
                );
            }
        } else {
            println!(
                "{} Unknown parameter '{}'. Use 'list parameters' to see available.",
                "!".yellow(),
                param
            );
        }

        Ok(())
    }

    /// Execute a set command (write parameter).
    async fn cmd_set(&self, param: &str, value: &str) -> Result<()> {
        let driver = {
            let guard = self.driver.lock().await;
            guard
                .as_ref()
                .cloned()
                .ok_or_else(|| anyhow!("Not connected. Use 'connect' first."))?
        };

        if let Some(param_def) = self.config.parameters.get(param) {
            if let Some(template) = param_def.set_command() {
                let command = format_set_command(template, value);
                println!("{} {}", "Sending:".dimmed(), command);
                driver.send_command(&command).await?;
                if let Ok(parsed) = value.parse::<f64>() {
                    driver.set_parameter(param, parsed).await;
                }
            } else {
                println!(
                    "{} Parameter '{}' has no set command in config.",
                    "!".yellow(),
                    param
                );
            }
        } else {
            println!(
                "{} Unknown parameter '{}'. Use 'list parameters' to see available.",
                "!".yellow(),
                param
            );
        }
        Ok(())
    }

    /// List available items (commands, parameters, capabilities, traits).
    fn cmd_list(&self, list_type: Option<&str>) -> Result<()> {
        match list_type {
            Some("commands") | Some("cmd") => {
                println!("{}", "Available Commands:".bold());
                println!();
                for (name, cmd) in &self.config.commands {
                    println!("  {}", name.cyan());
                    println!("    Template: {}", cmd.template.dimmed());
                    println!("    Expects Response: {}", cmd.expects_response);
                    if let Some(ref response) = cmd.response {
                        println!("    Response: {}", response);
                    }
                    println!();
                }
            }
            Some("parameters") | Some("params") => {
                println!("{}", "Defined Parameters:".bold());
                println!();
                if self.config.parameters.is_empty() {
                    println!("  {}", "No parameters defined".dimmed());
                } else {
                    for (name, param) in &self.config.parameters {
                        println!("  {}", name.cyan());
                        match param {
                            plugin_api::config::ParameterDef::Simple(cmd) => {
                                println!("    Mode: read-only");
                                println!("    Command: {}", cmd);
                            }
                            plugin_api::config::ParameterDef::GetSet { get, set } => {
                                println!("    Mode: read-write");
                                println!("    Get: {}", get);
                                println!("    Set: {}", set);
                            }
                            plugin_api::config::ParameterDef::Full(config) => {
                                println!("    Type: {:?}", config.r#type);
                                println!("    Default: {}", config.default);
                                if let Some((min, max)) = &config.range {
                                    println!("    Range: {} to {}", min, max);
                                }
                                if let Some(ref unit) = config.unit {
                                    println!("    Unit: {}", unit);
                                }
                                if let Some(ref desc) = config.description {
                                    println!("    Description: {}", desc);
                                }
                            }
                        }
                        println!();
                    }
                }
            }
            Some("capabilities") | Some("caps") => {
                println!("{}", "Device Capabilities:".bold());
                println!();
                if self.config.device.capabilities.is_empty() {
                    println!("  {}", "No capabilities defined".dimmed());
                } else {
                    for cap in &self.config.device.capabilities {
                        println!("  {}", cap.cyan());
                    }
                }
                println!();
                println!("{}", "Use 'list traits' to see trait mappings.".dimmed());
            }
            Some("traits") => {
                println!("{}", "Trait Mappings:".bold());
                println!();
                if let Some(ref movable) = self.config.trait_mapping.movable {
                    println!("  {}", "Movable".yellow().bold());
                    for (method, mapping) in movable {
                        if let Some(ref cmd) = mapping.command {
                            println!("    {} → {}", method.cyan(), cmd);
                        }
                        if let Some(ref poll_cmd) = mapping.poll_command {
                            println!("    {}.poll → {}", method.cyan(), poll_cmd);
                        }
                    }
                    println!();
                }
                for (trait_name, methods) in &self.config.trait_mapping.traits {
                    println!("  {}", trait_name.yellow().bold());
                    for (method, mapping) in methods {
                        if let Some(ref cmd) = mapping.command {
                            println!("    {} → {}", method.cyan(), cmd);
                        }
                        if let Some(ref poll_cmd) = mapping.poll_command {
                            println!("    {}.poll → {}", method.cyan(), poll_cmd);
                        }
                    }
                    println!();
                }
            }
            None => {
                println!("{}", "Summary:".bold());
                println!();
                println!("  Name:       {}", self.config.device.name);
                println!("  Protocol:   {}", self.config.device.protocol);
                println!("  Commands:   {}", self.config.commands.len());
                println!("  Parameters: {}", self.config.parameters.len());
                println!("  Responses:  {}", self.config.responses.len());
                println!();
                println!("{}", "Use 'list <type>' for details:".dimmed());
                println!("  {} - Command templates", "commands".cyan());
                println!("  {} - Parameter definitions", "parameters".cyan());
                println!("  {} - Device capabilities", "capabilities".cyan());
                println!("  {} - Trait mappings", "traits".cyan());
            }
            Some(unknown) => {
                return Err(anyhow!(
                    "Unknown list type '{}'. Valid types: commands, parameters, capabilities, traits",
                    unknown
                ));
            }
        }

        Ok(())
    }

    /// Show detailed info about a command.
    fn cmd_info(&self, name: Option<&str>) -> Result<()> {
        let name = name.ok_or_else(|| anyhow!("Usage: info <command-name>"))?;

        if let Some(cmd) = self.config.commands.get(name) {
            println!("{} {}", "Command:".bold(), name.cyan());
            println!();
            println!("  Template:      {}", cmd.template);
            println!("  Expects Response: {}", cmd.expects_response);
            if let Some(ref response) = cmd.response {
                println!("  Response:     {}", response);
            }
            println!();
            println!("{}", "Example:".dimmed());
            println!("  Send: {}", cmd.template);
            if cmd.expects_response {
                println!("  Expect: <response>");
            } else {
                println!("  Expect: No response (write command)");
            }

            Ok(())
        } else {
            Err(anyhow!(
                "Command '{}' not found. Use 'list commands' to see available.",
                name
            ))
        }
    }

    /// Reload configuration from disk.
    async fn cmd_reload(&mut self) -> Result<()> {
        println!(
            "Reloading configuration from {}...",
            self.config_path.display()
        );

        let config_str = fs::read_to_string(&self.config_path)
            .await
            .with_context(|| format!("Failed to read config: {}", self.config_path.display()))?;

        let mut config: InstrumentConfig = toml::from_str(&config_str)
            .with_context(|| format!("Failed to parse TOML: {}", self.config_path.display()))?;

        config.expand_shorthand();
        self.config = config;

        println!("{} Configuration reloaded", "✓".green().bold());

        // If connected, warn that reconnect may be needed
        if self.driver.lock().await.is_some() {
            println!(
                "{} Connection still active. Use 'disconnect' and 'connect' to apply changes.",
                "!".yellow()
            );
        }

        Ok(())
    }
}

fn format_set_command(template: &str, value: &str) -> String {
    if template.contains("{}") {
        template.replace("{}", value)
    } else if template.contains("{value}") {
        template.replace("{value}", value)
    } else {
        format!("{} {}", template, value)
    }
}
