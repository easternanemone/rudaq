//! PluginService implementation for data-driven instrument plugins (bd-22si.6.1)
//!
//! Provides gRPC endpoints for discovering plugins, querying their capabilities,
//! and managing plugin instances. This enables remote GUIs to dynamically render
//! controls based on plugin YAML definitions.

#[cfg(feature = "tokio_serial")]
use crate::hardware::plugin::registry::PluginFactory;
#[cfg(feature = "tokio_serial")]
use crate::hardware::plugin::schema::{DriverType, UiElement};
#[cfg(feature = "tokio_serial")]
use crate::hardware::plugin::driver::GenericDriver;
#[cfg(feature = "tokio_serial")]
use crate::hardware::registry::{DeviceConfig, DeviceRegistry, DriverType as RegistryDriverType};

use crate::grpc::proto::{
    plugin_service_server::PluginService, DestroyPluginInstanceRequest,
    DestroyPluginInstanceResponse, GetPluginInfoRequest, GetPluginInstanceStatusRequest,
    ListPluginInstancesRequest, ListPluginInstancesResponse, ListPluginsRequest,
    ListPluginsResponse, PluginActionable, PluginAxis, PluginCapabilities, PluginInfo,
    PluginInstanceStatus, PluginInstanceSummary, PluginLoggable, PluginMovable, PluginProtocol,
    PluginReadable, PluginScriptable, PluginSettable, PluginSwitchable, PluginSummary,
    PluginUiElement, SpawnPluginRequest, SpawnPluginResponse,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

/// Plugin instance tracking information
#[derive(Clone)]
pub struct PluginInstance {
    pub instance_id: String,
    pub plugin_id: String,
    pub plugin_name: String,
    pub address: String,
    pub device_id: String,
    pub connected: bool,
    pub mock_mode: bool,
    pub commands_sent: u64,
    pub commands_failed: u64,
    pub start_time_ns: u64,
    pub last_error: Option<String>,
    pub last_error_time_ns: Option<u64>,
    #[cfg(feature = "tokio_serial")]
    pub driver: Option<Arc<GenericDriver>>,
}

impl std::fmt::Debug for PluginInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginInstance")
            .field("instance_id", &self.instance_id)
            .field("plugin_id", &self.plugin_id)
            .field("connected", &self.connected)
            .field("driver", &"<Option<Arc<GenericDriver>>>")
            .finish()
    }
}

/// Plugin gRPC service implementation
///
/// Provides discovery and management of YAML-defined instrument plugins.
/// Works in conjunction with the plugin factory to list available plugins
/// and spawn driver instances.
pub struct PluginServiceImpl {
    #[cfg(feature = "tokio_serial")]
    factory: Arc<RwLock<PluginFactory>>,
    #[cfg(feature = "tokio_serial")]
    registry: Arc<RwLock<DeviceRegistry>>,
    instances: Arc<RwLock<HashMap<String, PluginInstance>>>,
    next_instance_id: Arc<RwLock<u64>>,
}

impl std::fmt::Debug for PluginServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginServiceImpl")
            .field("instances", &"<Arc<RwLock<HashMap>>>")
            .finish()
    }
}

impl PluginServiceImpl {
    /// Create a new PluginService with the given plugin factory and device registry
    #[cfg(feature = "tokio_serial")]
    pub fn new(
        factory: Arc<RwLock<PluginFactory>>,
        registry: Arc<RwLock<DeviceRegistry>>,
    ) -> Self {
        Self {
            factory,
            registry,
            instances: Arc::new(RwLock::new(HashMap::new())),
            next_instance_id: Arc::new(RwLock::new(1)),
        }
    }

    /// Create a stub PluginService when tokio_serial is not enabled
    #[cfg(not(feature = "tokio_serial"))]
    pub fn new_stub() -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            next_instance_id: Arc::new(RwLock::new(1)),
        }
    }

    /// Generate the next instance ID
    async fn next_instance_id(&self) -> String {
        let mut id = self.next_instance_id.write().await;
        let current = *id;
        *id += 1;
        format!("plugin-instance-{}", current)
    }
}

#[cfg(feature = "tokio_serial")]
fn driver_type_to_string(dt: &DriverType) -> String {
    match dt {
        DriverType::SerialScpi => "serial_scpi".to_string(),
        DriverType::TcpScpi => "tcp_scpi".to_string(),
        DriverType::SerialRaw => "serial_raw".to_string(),
        DriverType::TcpRaw => "tcp_raw".to_string(),
    }
}

#[cfg(feature = "tokio_serial")]
fn ui_element_to_proto(elem: &UiElement) -> PluginUiElement {
    match elem {
        UiElement::Group(g) => PluginUiElement {
            element_type: "group".to_string(),
            label: g.label.clone(),
            target: None,
            source: None,
            action: None,
            children: g.children.iter().map(ui_element_to_proto).collect(),
        },
        UiElement::Slider(s) => PluginUiElement {
            element_type: "slider".to_string(),
            label: s.label.clone().unwrap_or_default(),
            target: Some(s.target.clone()),
            source: None,
            action: None,
            children: vec![],
        },
        UiElement::Readout(r) => PluginUiElement {
            element_type: "readout".to_string(),
            label: r.label.clone().unwrap_or_default(),
            target: None,
            source: Some(r.source.clone()),
            action: None,
            children: vec![],
        },
        UiElement::Toggle(t) => PluginUiElement {
            element_type: "toggle".to_string(),
            label: t.label.clone().unwrap_or_default(),
            target: Some(t.target.clone()),
            source: None,
            action: None,
            children: vec![],
        },
        UiElement::Button(b) => PluginUiElement {
            element_type: "button".to_string(),
            label: b.label.clone(),
            target: None,
            source: None,
            action: Some(b.action.clone()),
            children: vec![],
        },
        UiElement::Dropdown(d) => PluginUiElement {
            element_type: "dropdown".to_string(),
            label: d.label.clone().unwrap_or_default(),
            target: Some(d.target.clone()),
            source: None,
            action: None,
            children: vec![],
        },
    }
}

#[tonic::async_trait]
impl PluginService for PluginServiceImpl {
    async fn list_plugins(
        &self,
        request: Request<ListPluginsRequest>,
    ) -> Result<Response<ListPluginsResponse>, Status> {
        #[cfg(not(feature = "tokio_serial"))]
        {
            let _ = request;
            return Ok(Response::new(ListPluginsResponse { plugins: vec![] }));
        }

        #[cfg(feature = "tokio_serial")]
        {
            let req = request.into_inner();
            let factory = self.factory.read().await;

            let mut plugins = Vec::new();
            for plugin_id in factory.available_plugins() {
                if let Some(config) = factory.get_config(&plugin_id) {
                    let driver_type = driver_type_to_string(&config.metadata.driver_type);

                    // Apply filter if provided
                    if let Some(ref filter) = req.driver_type_filter {
                        if &driver_type != filter {
                            continue;
                        }
                    }

                    let caps = &config.capabilities;
                    plugins.push(PluginSummary {
                        plugin_id: config.metadata.id.clone(),
                        name: config.metadata.name.clone(),
                        version: config.metadata.version.clone(),
                        driver_type,
                        has_readable: !caps.readable.is_empty(),
                        has_movable: caps.movable.is_some(),
                        has_settable: !caps.settable.is_empty(),
                        has_switchable: !caps.switchable.is_empty(),
                        has_actionable: !caps.actionable.is_empty(),
                        has_loggable: !caps.loggable.is_empty(),
                        has_scriptable: !caps.scriptable.is_empty(),
                    });
                }
            }

            Ok(Response::new(ListPluginsResponse { plugins }))
        }
    }

    async fn get_plugin_info(
        &self,
        request: Request<GetPluginInfoRequest>,
    ) -> Result<Response<PluginInfo>, Status> {
        #[cfg(not(feature = "tokio_serial"))]
        {
            let _ = request;
            return Err(Status::unimplemented(
                "Plugin system not available (tokio_serial feature disabled)",
            ));
        }

        #[cfg(feature = "tokio_serial")]
        {
            let plugin_id = request.into_inner().plugin_id;
            let factory = self.factory.read().await;

            let config = factory.get_config(&plugin_id).ok_or_else(|| {
                Status::not_found(format!("Plugin '{}' not found", plugin_id))
            })?;

            let caps = &config.capabilities;

            // Convert capabilities to proto types
            let readable: Vec<PluginReadable> = caps
                .readable
                .iter()
                .map(|r| PluginReadable {
                    name: r.name.clone(),
                    command: r.command.clone(),
                    pattern: r.pattern.clone(),
                    unit: r.unit.clone(),
                })
                .collect();

            let movable = caps.movable.as_ref().map(|m| PluginMovable {
                axes: m
                    .axes
                    .iter()
                    .map(|a| PluginAxis {
                        name: a.name.clone(),
                        unit: a.unit.clone(),
                        min: a.min,
                        max: a.max,
                    })
                    .collect(),
                set_cmd: m.set_cmd.clone(),
                get_cmd: m.get_cmd.clone(),
                get_pattern: m.get_pattern.clone(),
            });

            let settable: Vec<PluginSettable> = caps
                .settable
                .iter()
                .map(|s| PluginSettable {
                    name: s.name.clone(),
                    set_cmd: s.set_cmd.clone(),
                    get_cmd: s.get_cmd.clone(),
                    pattern: s.pattern.clone(),
                    unit: s.unit.clone(),
                    min: s.min,
                    max: s.max,
                    value_type: format!("{:?}", s.value_type).to_lowercase(),
                    options: s.options.clone(),
                })
                .collect();

            let switchable: Vec<PluginSwitchable> = caps
                .switchable
                .iter()
                .map(|s| PluginSwitchable {
                    name: s.name.clone(),
                    on_cmd: s.on_cmd.clone(),
                    off_cmd: s.off_cmd.clone(),
                    status_cmd: s.status_cmd.clone(),
                    pattern: s.pattern.clone(),
                })
                .collect();

            let actionable: Vec<PluginActionable> = caps
                .actionable
                .iter()
                .map(|a| PluginActionable {
                    name: a.name.clone(),
                    cmd: a.cmd.clone(),
                    wait_ms: a.wait_ms as u32,
                })
                .collect();

            let loggable: Vec<PluginLoggable> = caps
                .loggable
                .iter()
                .map(|l| PluginLoggable {
                    name: l.name.clone(),
                    cmd: l.cmd.clone(),
                    pattern: l.pattern.clone(),
                })
                .collect();

            let scriptable: Vec<PluginScriptable> = caps
                .scriptable
                .iter()
                .map(|s| PluginScriptable {
                    name: s.name.clone(),
                    description: s.description.clone(),
                    script: s.script.clone(),
                    timeout_ms: s.timeout_ms as u32,
                })
                .collect();

            let ui_layout: Vec<PluginUiElement> =
                config.ui_layout.iter().map(ui_element_to_proto).collect();

            Ok(Response::new(PluginInfo {
                plugin_id: config.metadata.id.clone(),
                name: config.metadata.name.clone(),
                version: config.metadata.version.clone(),
                driver_type: driver_type_to_string(&config.metadata.driver_type),
                protocol: Some(PluginProtocol {
                    baud_rate: config.protocol.baud_rate,
                    termination: config.protocol.termination.clone(),
                    command_delay_ms: config.protocol.command_delay_ms as u32,
                    timeout_ms: config.protocol.timeout_ms as u32,
                    tcp_host: config.protocol.tcp_host.clone(),
                    tcp_port: config.protocol.tcp_port.map(|p| p as u32),
                }),
                capabilities: Some(PluginCapabilities {
                    readable,
                    movable,
                    settable,
                    switchable,
                    actionable,
                    loggable,
                    scriptable,
                }),
                ui_layout,
            }))
        }
    }

    async fn spawn_plugin(
        &self,
        request: Request<SpawnPluginRequest>,
    ) -> Result<Response<SpawnPluginResponse>, Status> {
        #[cfg(not(feature = "tokio_serial"))]
        {
            let _ = request;
            return Err(Status::unimplemented(
                "Plugin system not available (tokio_serial feature disabled)",
            ));
        }

        #[cfg(feature = "tokio_serial")]
        {
            let req = request.into_inner();
            let factory = self.factory.read().await;

            // Get plugin name for tracking
            let plugin_name = factory
                .plugin_display_name(&req.plugin_id)
                .unwrap_or("Unknown")
                .to_string();

            // Generate IDs upfront
            let instance_id = self.next_instance_id().await;
            let device_id = format!("plugin-{}-{}", req.plugin_id, instance_id);

            // Attempt to spawn the driver
            let spawn_result = if req.mock_mode {
                // Spawn mock driver (no hardware connection)
                factory.spawn_mock(&req.plugin_id).await
            } else {
                // Spawn real driver with hardware connection
                factory.spawn(&req.plugin_id, &req.address).await
            };

            // Handle spawn result
            match spawn_result {
                Ok(driver) => {
                    let driver_arc = Arc::new(driver);

                    // 1. Create DeviceConfig for registration
                    let device_config = DeviceConfig {
                        id: device_id.clone(),
                        name: plugin_name.clone(),
                        driver: RegistryDriverType::Plugin {
                            plugin_id: req.plugin_id.clone(),
                            address: req.address.clone(),
                        },
                    };

                    // 2. Register with DeviceRegistry
                    let mut registry = self.registry.write().await;
                    if let Err(e) = registry
                        .register_plugin_instance(device_config, driver_arc.clone())
                        .await
                    {
                        let error_message =
                            format!("Spawned plugin but failed to register device: {}", e);
                        tracing::error!(
                            "Failed to register plugin instance {}: {}",
                            instance_id,
                            error_message
                        );
                        // Do not store the instance if registration fails, as the driver is now a zombie.
                        // The driver_arc will be dropped, closing the connection.
                        return Ok(Response::new(SpawnPluginResponse {
                            success: false,
                            error_message,
                            instance_id,
                            device_id,
                        }));
                    }
                    drop(registry); // Release registry lock

                    // 3. Store instance locally in PluginService
                    let instance = PluginInstance {
                        instance_id: instance_id.clone(),
                        plugin_id: req.plugin_id.clone(),
                        plugin_name,
                        address: req.address.clone(),
                        device_id: device_id.clone(),
                        connected: true,
                        mock_mode: req.mock_mode,
                        commands_sent: 0,
                        commands_failed: 0,
                        start_time_ns: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos() as u64,
                        last_error: None,
                        last_error_time_ns: None,
                        driver: Some(driver_arc),
                    };

                    self.instances
                        .write()
                        .await
                        .insert(instance_id.clone(), instance);

                    tracing::info!(
                        "Spawned and registered plugin instance: {} (plugin={}, device_id={}, mock={}, address={})",
                        instance_id,
                        req.plugin_id,
                        device_id,
                        req.mock_mode,
                        req.address
                    );

                    Ok(Response::new(SpawnPluginResponse {
                        success: true,
                        error_message: String::new(),
                        instance_id,
                        device_id,
                    }))
                }
                Err(e) => {
                    // Failed to spawn driver - create tracking entry with error
                    let error_message = format!("Failed to spawn plugin: {}", e);
                    
                    let instance = PluginInstance {
                        instance_id: instance_id.clone(),
                        plugin_id: req.plugin_id.clone(),
                        plugin_name,
                        address: req.address.clone(),
                        device_id: device_id.clone(),
                        connected: false,
                        mock_mode: req.mock_mode,
                        commands_sent: 0,
                        commands_failed: 1,
                        start_time_ns: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos() as u64,
                        last_error: Some(error_message.clone()),
                        last_error_time_ns: Some(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_nanos() as u64,
                        ),
                        driver: None,
                    };

                    self.instances
                        .write()
                        .await
                        .insert(instance_id.clone(), instance);

                    tracing::error!(
                        "Failed to spawn plugin instance: {} - {}",
                        instance_id,
                        error_message
                    );

                    Ok(Response::new(SpawnPluginResponse {
                        success: false,
                        error_message,
                        instance_id,
                        device_id,
                    }))
                }
            }
        }
    }

    async fn list_plugin_instances(
        &self,
        request: Request<ListPluginInstancesRequest>,
    ) -> Result<Response<ListPluginInstancesResponse>, Status> {
        let req = request.into_inner();
        let instances = self.instances.read().await;

        let summaries: Vec<PluginInstanceSummary> = instances
            .values()
            .filter(|inst| {
                req.plugin_id
                    .as_ref()
                    .map(|id| &inst.plugin_id == id)
                    .unwrap_or(true)
            })
            .map(|inst| PluginInstanceSummary {
                instance_id: inst.instance_id.clone(),
                plugin_id: inst.plugin_id.clone(),
                plugin_name: inst.plugin_name.clone(),
                address: inst.address.clone(),
                device_id: inst.device_id.clone(),
                connected: inst.connected,
                mock_mode: inst.mock_mode,
            })
            .collect();

        Ok(Response::new(ListPluginInstancesResponse {
            instances: summaries,
        }))
    }

    async fn get_plugin_instance_status(
        &self,
        request: Request<GetPluginInstanceStatusRequest>,
    ) -> Result<Response<PluginInstanceStatus>, Status> {
        let instance_id = request.into_inner().instance_id;
        let instances = self.instances.read().await;

        let inst = instances
            .get(&instance_id)
            .ok_or_else(|| Status::not_found(format!("Instance '{}' not found", instance_id)))?;

        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        Ok(Response::new(PluginInstanceStatus {
            instance_id: inst.instance_id.clone(),
            plugin_id: inst.plugin_id.clone(),
            plugin_name: inst.plugin_name.clone(),
            address: inst.address.clone(),
            device_id: inst.device_id.clone(),
            connected: inst.connected,
            mock_mode: inst.mock_mode,
            last_error: inst.last_error.clone(),
            last_error_time_ns: inst.last_error_time_ns,
            commands_sent: inst.commands_sent,
            commands_failed: inst.commands_failed,
            uptime_ns: now_ns.saturating_sub(inst.start_time_ns),
        }))
    }

    async fn destroy_plugin_instance(
        &self,
        request: Request<DestroyPluginInstanceRequest>,
    ) -> Result<Response<DestroyPluginInstanceResponse>, Status> {
        let req = request.into_inner();
        let mut instances = self.instances.write().await;

        #[cfg_attr(not(feature = "tokio_serial"), allow(unused_variables))]
        if let Some(instance) = instances.remove(&req.instance_id) {
            // Execute on_disconnect sequence if requested
            #[cfg(feature = "tokio_serial")]
            if req.run_disconnect_sequence {
                if let Some(driver) = &instance.driver {
                    let disconnect_sequence = &driver.config.on_disconnect;
                    if !disconnect_sequence.is_empty() {
                        tracing::info!(
                            "Executing on_disconnect sequence for instance '{}' ({} commands)",
                            req.instance_id,
                            disconnect_sequence.len()
                        );
                        
                        match driver.execute_command_sequence(disconnect_sequence).await {
                            Ok(()) => {
                                tracing::info!(
                                    "Successfully executed on_disconnect sequence for instance '{}'",
                                    req.instance_id
                                );
                            }
                            Err(e) => {
                                // Log the error but don't fail the destroy operation
                                tracing::warn!(
                                    "Error executing on_disconnect sequence for instance '{}': {}. \
                                     Continuing with instance destruction.",
                                    req.instance_id,
                                    e
                                );
                            }
                        }
                    } else {
                        tracing::debug!(
                            "No on_disconnect commands defined for instance '{}'",
                            req.instance_id
                        );
                    }
                }
            }

            // Also unregister from the DeviceRegistry
            #[cfg(feature = "tokio_serial")]
            {
                let mut registry = self.registry.write().await;
                if !registry.unregister(&instance.device_id) {
                    tracing::warn!(
                        "Device '{}' for instance '{}' was not found in registry during destroy.",
                        instance.device_id,
                        req.instance_id
                    );
                }
            }

            Ok(Response::new(DestroyPluginInstanceResponse {
                success: true,
                error_message: String::new(),
            }))
        } else {
            Err(Status::not_found(format!(
                "Instance '{}' not found",
                req.instance_id
            )))
        }
    }
}
