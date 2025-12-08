// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::Channel;

// Import proto types from rust_daq
use rust_daq::grpc::proto::{
    hardware_service_client::HardwareServiceClient, ArmRequest, DeviceStateRequest,
    DeviceStateSubscribeRequest, GetExposureRequest, GetParameterRequest, GetShutterRequest,
    GetWavelengthRequest, ListDevicesRequest, ListParametersRequest, MoveRequest,
    ReadValueRequest, SetExposureRequest, SetParameterRequest, SetShutterRequest,
    SetWavelengthRequest, StopMotionRequest, WaitSettledRequest,
    SetEmissionRequest, GetEmissionRequest,
};

// Shared gRPC client state
struct AppState {
    client: Option<HardwareServiceClient<Channel>>,
}

impl AppState {
    fn new() -> Self {
        Self { client: None }
    }
}

// Serializable device info for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceInfo {
    id: String,
    name: String,
    driver_type: String,
    is_movable: bool,
    is_readable: bool,
    is_triggerable: bool,
    is_frame_producer: bool,
    is_exposure_controllable: bool,
    is_shutter_controllable: bool,
    is_wavelength_tunable: bool,
    is_emission_controllable: bool,
    position_units: Option<String>,
    min_position: Option<f64>,
    max_position: Option<f64>,
    reading_units: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceState {
    device_id: String,
    online: bool,
    position: Option<f64>,
    last_reading: Option<f64>,
    armed: Option<bool>,
    streaming: Option<bool>,
    exposure_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ParameterDescriptor {
    device_id: String,
    name: String,
    description: String,
    dtype: String,
    units: String,
    readable: bool,
    writable: bool,
    min_value: Option<f64>,
    max_value: Option<f64>,
    enum_values: Vec<String>,
}

// Tauri commands

#[tauri::command]
async fn connect_to_daemon(
    address: String,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let endpoint = format!("http://{}", address);

    let client = HardwareServiceClient::connect(endpoint.clone())
        .await
        .map_err(|e| format!("Failed to connect: {}", e))?;

    let mut app_state = state.write().await;
    app_state.client = Some(client);

    Ok(format!("Connected to {}", endpoint))
}

#[tauri::command]
async fn list_devices(
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<DeviceInfo>, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(ListDevicesRequest {
        capability_filter: None,
    });

    let response = client
        .list_devices(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let devices: Vec<DeviceInfo> = response
        .into_inner()
        .devices
        .into_iter()
        .map(|d| {
            let metadata = d.metadata.unwrap_or_default();
            DeviceInfo {
                id: d.id,
                name: d.name,
                driver_type: d.driver_type,
                is_movable: d.is_movable,
                is_readable: d.is_readable,
                is_triggerable: d.is_triggerable,
                is_frame_producer: d.is_frame_producer,
                is_exposure_controllable: d.is_exposure_controllable,
                is_shutter_controllable: d.is_shutter_controllable,
                is_wavelength_tunable: d.is_wavelength_tunable,
                is_emission_controllable: d.is_emission_controllable,
                position_units: metadata.position_units,
                min_position: metadata.min_position,
                max_position: metadata.max_position,
                reading_units: metadata.reading_units,
            }
        })
        .collect();

    Ok(devices)
}

#[tauri::command]
async fn get_device_state(
    device_id: String,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<DeviceState, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(DeviceStateRequest { device_id: device_id.clone() });

    let response = client
        .get_device_state(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let state = response.into_inner();
    Ok(DeviceState {
        device_id,
        online: state.online,
        position: state.position,
        last_reading: state.last_reading,
        armed: state.armed,
        streaming: state.streaming,
        exposure_ms: state.exposure_ms,
    })
}

#[tauri::command]
async fn move_absolute(
    device_id: String,
    position: f64,
    wait_for_completion: bool,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(MoveRequest {
        device_id: device_id.clone(),
        value: position,
        wait_for_completion: Some(wait_for_completion),
        timeout_ms: Some(30000), // 30 second timeout
    });

    let response = client
        .move_absolute(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let resp = response.into_inner();
    if resp.success {
        Ok(format!(
            "Moved {} to position {:.3}",
            device_id, resp.final_position
        ))
    } else {
        Err(resp.error_message)
    }
}

#[tauri::command]
async fn stop_motion(
    device_id: String,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(StopMotionRequest {
        device_id: device_id.clone(),
    });

    let response = client
        .stop_motion(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let resp = response.into_inner();
    if resp.success {
        Ok(format!(
            "Stopped {} at position {:.3}",
            device_id, resp.stopped_position
        ))
    } else {
        Err("Failed to stop motion".to_string())
    }
}

#[tauri::command]
async fn read_value(
    device_id: String,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<(f64, String), String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(ReadValueRequest {
        device_id: device_id.clone(),
    });

    let response = client
        .read_value(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let resp = response.into_inner();
    if resp.success {
        Ok((resp.value, resp.units))
    } else {
        Err(resp.error_message)
    }
}

#[tauri::command]
async fn set_exposure(
    device_id: String,
    exposure_ms: f64,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<f64, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(SetExposureRequest {
        device_id,
        exposure_ms,
    });

    let response = client
        .set_exposure(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let resp = response.into_inner();
    if resp.success {
        Ok(resp.actual_exposure_ms)
    } else {
        Err(resp.error_message)
    }
}

#[tauri::command]
async fn get_exposure(
    device_id: String,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<f64, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(GetExposureRequest { device_id });

    let response = client
        .get_exposure(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    Ok(response.into_inner().exposure_ms)
}

#[tauri::command]
async fn set_shutter(
    device_id: String,
    open: bool,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<bool, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(SetShutterRequest {
        device_id,
        open,
    });

    let response = client
        .set_shutter(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let resp = response.into_inner();
    if resp.success {
        Ok(resp.is_open)
    } else {
        Err(resp.error_message)
    }
}

#[tauri::command]
async fn set_wavelength(
    device_id: String,
    wavelength_nm: f64,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<f64, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(SetWavelengthRequest {
        device_id,
        wavelength_nm,
    });

    let response = client
        .set_wavelength(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let resp = response.into_inner();
    if resp.success {
        Ok(resp.actual_wavelength_nm)
    } else {
        Err(resp.error_message)
    }
}

#[tauri::command]
async fn set_emission(
    device_id: String,
    enabled: bool,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<bool, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(SetEmissionRequest {
        device_id,
        enabled,
    });

    let response = client
        .set_emission(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let resp = response.into_inner();
    if resp.success {
        Ok(resp.is_enabled)
    } else {
        Err(resp.error_message)
    }
}

#[tauri::command]
async fn list_parameters(
    device_id: String,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ParameterDescriptor>, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(ListParametersRequest {
        device_id: device_id.clone(),
    });

    let response = client
        .list_parameters(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let params: Vec<ParameterDescriptor> = response
        .into_inner()
        .parameters
        .into_iter()
        .map(|p| ParameterDescriptor {
            device_id: p.device_id,
            name: p.name,
            description: p.description,
            dtype: p.dtype,
            units: p.units,
            readable: p.readable,
            writable: p.writable,
            min_value: p.min_value,
            max_value: p.max_value,
            enum_values: p.enum_values,
        })
        .collect();

    Ok(params)
}

#[tauri::command]
async fn get_parameter(
    device_id: String,
    parameter_name: String,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(GetParameterRequest {
        device_id,
        parameter_name,
    });

    let response = client
        .get_parameter(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    Ok(response.into_inner().value)
}

#[tauri::command]
async fn set_parameter(
    device_id: String,
    parameter_name: String,
    value: String,
    state: tauri::State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let mut app_state = state.write().await;
    let client = app_state
        .client
        .as_mut()
        .ok_or("Not connected to daemon")?;

    let request = tonic::Request::new(SetParameterRequest {
        device_id,
        parameter_name,
        value,
    });

    let response = client
        .set_parameter(request)
        .await
        .map_err(|e| format!("gRPC error: {}", e))?;

    let resp = response.into_inner();
    if resp.success {
        Ok(resp.actual_value)
    } else {
        Err(resp.error_message)
    }
}

fn main() {
    let app_state = Arc::new(RwLock::new(AppState::new()));

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            connect_to_daemon,
            list_devices,
            get_device_state,
            move_absolute,
            stop_motion,
            read_value,
            set_exposure,
            get_exposure,
            set_shutter,
            set_wavelength,
            set_emission,
            list_parameters,
            get_parameter,
            set_parameter,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
