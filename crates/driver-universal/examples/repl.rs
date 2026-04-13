//! Interactive REPL for universal driver development and debugging (bd-6vh6).
//!
//! Load a schema v3 TOML manifest, optionally connect to real hardware, and
//! interactively send commands, query parameters, and inspect metadata —
//! without running the full DAQ daemon.
//!
//! # Usage
//!
//! ```bash
//! # Validate a manifest (no hardware)
//! cargo run -p driver-universal --example repl -- config/devices/ell14.toml
//!
//! # Connect to a serial device
//! cargo run -p driver-universal --example repl -- config/devices/ell14.toml --port /dev/ttyUSB0
//!
//! # Connect to a TCP device
//! cargo run -p driver-universal --example repl -- config/devices/ipg_laser.toml --host 10.0.0.15 --tcp-port 10001
//!
//! # Mock mode (validates manifest, no transport)
//! cargo run -p driver-universal --example repl -- config/devices/ell14.toml --mock
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::Mutex;

use driver_universal::config::parse::parse_manifest;
use driver_universal::config::raw::RawManifest;
use driver_universal::config::validated::{ConnectionConfig, DeviceManifest};
use driver_universal::driver::UniversalDriver;
use driver_universal::transport::{MockTransport, Transport};

// =============================================================================
// CLI argument parsing (manual — avoids adding clap dependency)
// =============================================================================

struct Args {
    manifest_path: PathBuf,
    port: Option<String>,
    host: Option<String>,
    tcp_port: Option<u16>,
    mock: bool,
    address: String,
}

fn parse_args() -> Result<Args> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        eprintln!("Usage: repl <manifest.toml> [OPTIONS]");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --port <path>      Serial port (e.g., /dev/ttyUSB0)");
        eprintln!("  --host <addr>      TCP host override (e.g., 10.0.0.15)");
        eprintln!("  --tcp-port <port>  TCP port override (e.g., 10001)");
        eprintln!("  --address <addr>   Device bus address for RS-485 (default: 0)");
        eprintln!("  --mock             Mock mode (no hardware connection)");
        std::process::exit(i32::from(args.len() < 2));
    }

    let manifest_path = PathBuf::from(&args[1]);
    let mut port = None;
    let mut host = None;
    let mut tcp_port = None;
    let mut mock = false;
    let mut address = "0".to_string();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                port = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow!("--port requires a value"))?
                        .clone(),
                );
            }
            "--host" => {
                i += 1;
                host = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow!("--host requires a value"))?
                        .clone(),
                );
            }
            "--tcp-port" => {
                i += 1;
                tcp_port = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow!("--tcp-port requires a value"))?
                        .parse()?,
                );
            }
            "--address" => {
                i += 1;
                address.clone_from(
                    args.get(i)
                        .ok_or_else(|| anyhow!("--address requires a value"))?,
                );
            }
            "--mock" => mock = true,
            other => return Err(anyhow!("Unknown argument: {other}")),
        }
        i += 1;
    }

    Ok(Args {
        manifest_path,
        port,
        host,
        tcp_port,
        mock,
        address,
    })
}

// =============================================================================
// REPL state
// =============================================================================

struct Repl {
    manifest: Arc<DeviceManifest>,
    manifest_path: PathBuf,
    driver: Option<UniversalDriver>,
    transport: Option<Arc<Mutex<Box<dyn Transport>>>>,
    address: String,
    mock: bool,
    /// CLI overrides for connection
    port_override: Option<String>,
    host_override: Option<String>,
    tcp_port_override: Option<u16>,
}

impl Repl {
    async fn load_manifest(path: &Path) -> Result<DeviceManifest> {
        let content = tokio::fs::read_to_string(path).await?;
        let raw: RawManifest = toml::from_str(&content)?;
        parse_manifest(raw).map_err(|errors| {
            let msgs: Vec<String> = errors.iter().map(ToString::to_string).collect();
            anyhow!("Manifest validation errors:\n  {}", msgs.join("\n  "))
        })
    }

    async fn new(args: &Args) -> Result<Self> {
        let manifest = Self::load_manifest(&args.manifest_path).await?;
        Ok(Self {
            manifest: Arc::new(manifest),
            manifest_path: args.manifest_path.clone(),
            driver: None,
            transport: None,
            address: args.address.clone(),
            mock: args.mock,
            port_override: args.port.clone(),
            host_override: args.host.clone(),
            tcp_port_override: args.tcp_port,
        })
    }

    async fn run(&mut self) -> Result<()> {
        println!("daq-driver-dev — Universal Driver REPL (bd-6vh6)");
        println!("{}", "=".repeat(50));
        println!();

        self.cmd_validate();
        println!();

        if self.mock {
            println!("  Mode: MOCK (no hardware)");
        } else {
            println!("  Status: Disconnected");
            println!("  Hint: use 'connect' to open transport");
        }
        println!();
        println!("  Type 'help' for commands, 'exit' to quit.");
        println!();

        let stdin = std::io::stdin();
        let mut line_buf = String::new();

        loop {
            line_buf.clear();
            eprint!(">> ");
            use std::io::Write;
            std::io::stderr().flush().ok();

            match stdin.read_line(&mut line_buf) {
                Ok(0) => {
                    println!("exit");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Read error: {e}");
                    break;
                }
            }

            let line = line_buf.trim();
            if line.is_empty() {
                continue;
            }

            match self.execute(line).await {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => eprintln!("  Error: {e}"),
            }
        }

        Ok(())
    }

    async fn execute(&mut self, line: &str) -> Result<bool> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(true);
        }

        match parts[0] {
            "help" | "h" | "?" => self.cmd_help(),
            "validate" => self.cmd_validate(),
            "connect" => self.cmd_connect(&parts[1..]).await?,
            "disconnect" => self.cmd_disconnect(),
            "raw" => self.cmd_raw(&parts[1..]).await?,
            "query" => self.cmd_query(&parts[1..]).await?,
            "list" => self.cmd_list(parts.get(1).copied()),
            "info" => self.cmd_info(parts.get(1).copied()),
            "params" => self.cmd_params(),
            "reload" => self.cmd_reload().await?,
            "exit" | "quit" | "q" => {
                println!("Goodbye!");
                return Ok(false);
            }
            other => eprintln!("  Unknown command: '{other}'. Type 'help' for commands."),
        }

        Ok(true)
    }

    // =========================================================================
    // Commands
    // =========================================================================

    fn cmd_help(&self) {
        println!("Commands:");
        println!("  validate                 Validate manifest schema");
        println!("  connect [--port P]       Connect to device (serial/TCP from manifest)");
        println!("  disconnect               Close transport connection");
        println!("  raw <text>               Send raw string, print response");
        println!("  query <command> [k=v ..] Execute a named manifest command");
        println!("  list [commands|params|responses|caps|conversions]");
        println!("                           List manifest items");
        println!("  info <command>           Show detailed command info");
        println!("  params                   Show parameter metadata");
        println!("  reload                   Hot-reload manifest from disk");
        println!("  exit                     Quit");
    }

    fn cmd_validate(&self) {
        let m = &self.manifest;
        let d = &m.device;
        println!("  Manifest: {}", self.manifest_path.display());
        println!("  Name:     {}", d.name);
        if let Some(ref mfr) = d.manufacturer {
            println!("  Mfr:      {mfr}");
        }
        println!(
            "  Commands: {} | Responses: {} | Conversions: {}",
            m.commands.len(),
            m.responses.len(),
            m.conversions.len()
        );
        println!(
            "  Params:   {} | Init seq: {} steps",
            m.parameter_metadata.len(),
            m.init_sequence.len()
        );

        let conn = match &m.connection {
            ConnectionConfig::Serial {
                baud_rate,
                terminator,
                ..
            } => {
                let term = terminator.as_deref().unwrap_or("none");
                format!("serial @{} (term: {term:?})", baud_rate.value())
            }
            ConnectionConfig::Tcp { host, port, .. } => format!("tcp {host}:{port}"),
            ConnectionConfig::Udp { host, port, .. } => format!("udp {host}:{port}"),
            ConnectionConfig::Usbtmc { .. } => "usbtmc".to_string(),
        };
        println!("  Transport: {conn}");

        let cap_names = &d.capability_names;
        if !cap_names.is_empty() {
            println!("  Capabilities: {}", cap_names.join(", "));
        }

        println!("  OK — manifest is valid");
    }

    async fn cmd_connect(&mut self, args: &[&str]) -> Result<()> {
        if self.transport.is_some() {
            println!("  Already connected. Use 'disconnect' first.");
            return Ok(());
        }

        // Parse optional --port override from inline args
        let mut port_override = self.port_override.clone();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--port" && i + 1 < args.len() {
                port_override = Some(args[i + 1].to_string());
                i += 2;
            } else {
                i += 1;
            }
        }

        let transport: Box<dyn Transport> = if self.mock {
            println!("  Connected (mock transport)");
            Box::new(MockTransport::new(vec![]))
        } else {
            match &self.manifest.connection {
                ConnectionConfig::Serial {
                    baud_rate,
                    terminator,
                    serial_config,
                    ..
                } => {
                    let port_path = port_override
                        .as_deref()
                        .ok_or_else(|| anyhow!("Serial manifest requires --port <path>"))?;
                    let baud = baud_rate.value();
                    println!("  Opening serial {port_path} @ {baud} baud...");
                    let t = driver_universal::transport::SerialTransport::open(
                        port_path,
                        baud,
                        terminator.as_deref(),
                        serial_config,
                    )
                    .await?;
                    println!("  Connected (serial)");
                    Box::new(t)
                }
                ConnectionConfig::Tcp {
                    host,
                    port,
                    timeout,
                    terminator,
                } => {
                    let h = self.host_override.as_deref().unwrap_or(host.as_str());
                    let p = self.tcp_port_override.unwrap_or(*port);
                    println!("  Connecting to {h}:{p}...");
                    let t = driver_universal::transport::TcpTransport::connect(
                        h,
                        p,
                        timeout.as_duration(),
                        terminator.as_deref(),
                    )
                    .await?;
                    println!("  Connected (TCP)");
                    Box::new(t)
                }
                ConnectionConfig::Udp { .. } => {
                    return Err(anyhow!("UDP transport not yet implemented"));
                }
                ConnectionConfig::Usbtmc { .. } => {
                    return Err(anyhow!("USBTMC transport not yet implemented in REPL"));
                }
            }
        };

        let shared: Arc<Mutex<Box<dyn Transport>>> = Arc::new(Mutex::new(transport));
        let driver = UniversalDriver::new_shared(
            Arc::clone(&self.manifest),
            Arc::clone(&shared),
            &self.address,
        );

        if !self.manifest.init_sequence.is_empty() {
            println!(
                "  Running init sequence ({} steps)...",
                self.manifest.init_sequence.len()
            );
            driver.run_init_sequence().await?;
            println!("  Init sequence complete");
        }

        self.transport = Some(shared);
        self.driver = Some(driver);
        Ok(())
    }

    fn cmd_disconnect(&mut self) {
        if self.transport.is_none() {
            println!("  Not connected.");
            return;
        }
        self.driver = None;
        self.transport = None;
        println!("  Disconnected.");
    }

    async fn cmd_raw(&self, args: &[&str]) -> Result<()> {
        let transport = self
            .transport
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Use 'connect' first."))?;

        if args.is_empty() {
            return Err(anyhow!("Usage: raw <command text>"));
        }
        let command = args.join(" ");
        let timeout = self.manifest.connection.timeout().as_duration();

        let transport = transport.lock().await;
        let response = transport.query(command.as_bytes(), timeout).await?;
        println!("  TX: {command}");
        println!("  RX: {response}");
        Ok(())
    }

    async fn cmd_query(&self, args: &[&str]) -> Result<()> {
        let transport = self
            .transport
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Use 'connect' first."))?;

        if args.is_empty() {
            return Err(anyhow!("Usage: query <command_name> [param=value ...]"));
        }
        let cmd_name = args[0];
        let cmd_config = self
            .manifest
            .commands
            .get(cmd_name)
            .ok_or_else(|| anyhow!("Unknown command: '{cmd_name}'. Use 'list commands'."))?;

        // Build template parameters from remaining args
        let mut params: HashMap<String, serde_json::Value> = HashMap::new();
        params.insert("address".to_string(), serde_json::json!(self.address));

        for arg in &args[1..] {
            if let Some((k, v)) = arg.split_once('=') {
                if let Ok(n) = v.parse::<f64>() {
                    params.insert(k.to_string(), serde_json::json!(n));
                } else {
                    params.insert(k.to_string(), serde_json::json!(v));
                }
            }
        }

        let command_str =
            driver_universal::template::render_command(&cmd_config.template, &params)?;
        let timeout = self.manifest.connection.timeout().as_duration();

        println!("  TX: {command_str}");

        if cmd_config.expects_response {
            let transport = transport.lock().await;
            let response = transport.query(command_str.as_bytes(), timeout).await?;
            println!("  RX: {response}");

            // Try to parse response
            if let Some(ref resp_ref) = cmd_config.response
                && let Some(resp_config) = self.manifest.responses.get(&resp_ref.0)
            {
                match driver_universal::response::parse_with_parser(&response, resp_config) {
                    Ok(parsed) => println!("  Parsed: {parsed}"),
                    Err(e) => println!("  Parse failed: {e}"),
                }
            }
        } else {
            let transport = transport.lock().await;
            transport.send(command_str.as_bytes()).await?;
            println!("  (no response expected)");
        }

        Ok(())
    }

    fn cmd_list(&self, what: Option<&str>) {
        match what {
            Some("commands") | Some("cmd") | Some("c") => {
                print_sorted("Commands", self.manifest.commands.keys(), |name| {
                    let suffix = if self.manifest.commands[name].expects_response {
                        " [expects response]"
                    } else {
                        ""
                    };
                    format!("{name}{suffix}")
                });
            }
            Some("params") | Some("p") => self.cmd_params(),
            Some("responses") | Some("resp") | Some("r") => {
                print_sorted("Responses", self.manifest.responses.keys(), |n| {
                    n.to_string()
                });
            }
            Some("caps") | Some("capabilities") => {
                let caps = &self.manifest.device.capability_names;
                println!("  Capabilities ({}):", caps.len());
                for cap in caps {
                    println!("    {cap}");
                }
            }
            Some("conversions") | Some("conv") => {
                print_sorted("Conversions", self.manifest.conversions.keys(), |n| {
                    n.to_string()
                });
            }
            _ => {
                println!(
                    "  commands: {}  params: {}  responses: {}  caps: {}  conversions: {}",
                    self.manifest.commands.len(),
                    self.manifest.parameter_metadata.len(),
                    self.manifest.responses.len(),
                    self.manifest.device.capability_names.len(),
                    self.manifest.conversions.len(),
                );
                println!("  Use: list commands|params|responses|caps|conversions");
            }
        }
    }

    fn cmd_info(&self, name: Option<&str>) {
        let Some(name) = name else {
            println!("  Usage: info <command_name>");
            return;
        };

        if let Some(cmd) = self.manifest.commands.get(name) {
            println!("  Command: {name}");
            println!("  Template: {}", cmd.template.source);
            if let Some(ref desc) = cmd.description {
                println!("  Description: {desc}");
            }
            if let Some(ref resp_ref) = cmd.response {
                println!("  Response parser: {}", resp_ref.0);
            }
            if cmd.expects_response {
                println!("  Expects response: yes");
            }
            if !cmd.parameters.is_empty() {
                println!("  Parameters:");
                for (name, dtype) in &cmd.parameters {
                    println!("    {name}: {dtype}");
                }
            }
        } else {
            println!("  Unknown command: '{name}'. Use 'list commands'.");
        }
    }

    fn cmd_params(&self) {
        if self.manifest.parameter_metadata.is_empty() {
            println!("  No parameters defined.");
            return;
        }
        println!("  Parameters ({}):", self.manifest.parameter_metadata.len());
        for meta in &self.manifest.parameter_metadata {
            let rw = if meta.read_only { "RO" } else { "RW" };
            let unit = meta.unit.as_deref().unwrap_or("");
            let range = match (meta.min_value, meta.max_value) {
                (Some(lo), Some(hi)) => format!(" [{lo}..{hi}]"),
                (Some(lo), None) => format!(" [{lo}..]"),
                (None, Some(hi)) => format!(" [..{hi}]"),
                (None, None) => String::new(),
            };
            println!(
                "    {} ({}) {}{} {}",
                meta.name, meta.dtype, rw, range, unit
            );
        }
    }

    async fn cmd_reload(&mut self) -> Result<()> {
        println!("  Reloading {}...", self.manifest_path.display());
        let new_manifest = Self::load_manifest(&self.manifest_path).await?;
        let new_arc = Arc::new(new_manifest);
        self.manifest = new_arc.clone();

        // Reconnect driver with new manifest if transport is active
        if let Some(ref transport) = self.transport {
            self.driver = Some(UniversalDriver::new_shared(
                new_arc,
                Arc::clone(transport),
                &self.address,
            ));
            println!("  Reloaded and reconnected driver with new manifest.");
        } else {
            println!("  Reloaded (not connected).");
        }

        self.cmd_validate();
        Ok(())
    }
}

/// Print a sorted list of items with a title and per-item formatting.
fn print_sorted<'a>(
    title: &str,
    keys: impl Iterator<Item = &'a String>,
    fmt: impl Fn(&str) -> String,
) {
    let mut names: Vec<&str> = keys.map(String::as_str).collect();
    names.sort_unstable();
    println!("  {title} ({}):", names.len());
    for name in names {
        println!("    {}", fmt(name));
    }
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let mut repl = Repl::new(&args).await?;

    if args.port.is_some() || args.host.is_some() || args.mock {
        repl.cmd_connect(&[]).await?;
    }

    repl.run().await
}
