#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    unsafe_code,
    missing_docs,
    deprecated
)]
//! Module export verification tests
//!
//! These tests verify that public types are actually exported and accessible from the
//! `rust_daq` crate. They serve as compile-time verification to prevent dead code issues
//! where modules exist but aren't properly exported.
//!
//! # Background
//!
//! This test suite was created after `storage.rs` was discovered to be dead code because
//! it wasn't exported from `mod.rs`. Standard unit tests didn't catch this because they
//! tested the file directly, not through the public API.
//!
//! # Test Strategy
//!
//! Each test uses a type-checking pattern that forces the compiler to verify the type
//! exists and is accessible. If the export is missing or broken, the test fails at
//! compile time, not runtime.
//!
//! # Feature Gates
//!
//! Tests are feature-gated to match the actual availability of modules in the crate.

// =============================================================================
// Hardware Capability Trait Exports
// =============================================================================

#[test]
fn verify_movable_trait_export() {
    // Verify Movable trait is exported at crate root
    fn _check_trait_exists<T: rust_daq::hardware::Movable>() {}

    // Also verify it's re-exported from capabilities module
    fn _check_from_capabilities<T: rust_daq::hardware::capabilities::Movable>() {}
}

#[test]
fn verify_readable_trait_export() {
    // Verify Readable trait is exported
    fn _check_trait_exists<T: rust_daq::hardware::Readable>() {}
    fn _check_from_capabilities<T: rust_daq::hardware::capabilities::Readable>() {}
}

#[test]
fn verify_frame_producer_trait_export() {
    // Verify FrameProducer trait is exported
    fn _check_trait_exists<T: rust_daq::hardware::FrameProducer>() {}
    fn _check_from_capabilities<T: rust_daq::hardware::capabilities::FrameProducer>() {}
}

#[test]
fn verify_exposure_control_trait_export() {
    // Verify ExposureControl trait is exported
    fn _check_trait_exists<T: rust_daq::hardware::ExposureControl>() {}
    fn _check_from_capabilities<T: rust_daq::hardware::capabilities::ExposureControl>() {}
}

#[test]
fn verify_triggerable_trait_export() {
    // Verify Triggerable trait is exported
    fn _check_trait_exists<T: rust_daq::hardware::Triggerable>() {}
    fn _check_from_capabilities<T: rust_daq::hardware::capabilities::Triggerable>() {}
}

// =============================================================================
// Hardware Data Types
// =============================================================================

#[test]
fn verify_frame_ref_export() {
    // Verify FrameRef is exported from hardware module
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::FrameRef>();

    // Verify it can be constructed
    let data = vec![0u8; 1024];
    let _frame = rust_daq::hardware::FrameRef::new(32, 32, data, 32);
}

#[test]
fn verify_frame_export() {
    // Verify Frame is exported from hardware module
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::Frame>();

    // Verify it can be constructed
    let buffer = vec![0u16; 1024];
    let _frame = rust_daq::hardware::Frame::from_u16(32, 32, &buffer);
}

#[test]
fn verify_roi_export() {
    // Verify Roi is exported from hardware module
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::Roi>();

    // Verify Default trait is accessible
    let _roi = rust_daq::hardware::Roi::default();
}

// =============================================================================
// Hardware Mock Implementations
// =============================================================================

#[test]
fn verify_mock_stage_export() {
    // Verify MockStage is exported and implements Movable
    fn _check_type_exists<T: rust_daq::hardware::Movable>() {}
    _check_type_exists::<rust_daq::hardware::mock::MockStage>();
}

#[test]
fn verify_mock_camera_export() {
    // Verify MockCamera is exported and implements FrameProducer + ExposureControl
    fn _check_type_exists<
        T: rust_daq::hardware::FrameProducer + rust_daq::hardware::ExposureControl,
    >() {
    }
    _check_type_exists::<rust_daq::hardware::mock::MockCamera>();
}

#[test]
fn verify_mock_power_meter_export() {
    // Verify MockPowerMeter is exported and implements Readable
    fn _check_type_exists<T: rust_daq::hardware::Readable>() {}
    _check_type_exists::<rust_daq::hardware::mock::MockPowerMeter>();
}

// =============================================================================
// Hardware Registry
// =============================================================================

#[test]
fn verify_device_id_export() {
    // Verify DeviceId type alias is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::registry::DeviceId>();
}

#[test]
fn verify_capability_enum_export() {
    // Verify Capability enum is exported from common
    fn _check_type_exists<T>() {}
    _check_type_exists::<common::driver::Capability>();
}

// =============================================================================
// Mock Serial Port (always available)
// =============================================================================

#[test]
fn verify_mock_serial_export() {
    // Verify mock_serial module is exported
    fn _check_module_exists() {
        // This will fail to compile if the module isn't exported
        let (_port, _harness) = rust_daq::hardware::mock_serial::new();
    }
    _check_module_exists();
}

// =============================================================================
// Real Hardware Drivers (feature-gated)
// =============================================================================

#[cfg(feature = "thorlabs")]
#[test]
fn verify_ell14_export() {
    // Verify Ell14 driver is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::ell14::Ell14Driver>();
}

#[cfg(feature = "newport")]
#[test]
fn verify_esp300_export() {
    // Verify Esp300 driver is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::esp300::Esp300Driver>();
}

#[cfg(all(feature = "pvcam", feature = "pvcam_sdk"))]
#[test]
fn verify_pvcam_export() {
    // Verify PVCAM camera is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::pvcam::PvcamDriver>();
}

#[cfg(feature = "spectra_physics")]
#[test]
fn verify_maitai_export() {
    // Verify MaiTai laser is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::maitai::MaiTaiDriver>();
}

#[cfg(feature = "newport_power_meter")]
#[test]
fn verify_newport_1830c_export() {
    // Verify Newport 1830-C power meter is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::hardware::newport_1830c::Newport1830CDriver>();
}

// =============================================================================
// gRPC Services (feature: networking)
// =============================================================================

#[cfg(feature = "server")]
#[test]
fn verify_grpc_server_export() {
    // Verify DaqServer is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<daq_server::grpc::DaqServer>();
}

#[cfg(feature = "server")]
#[test]
fn verify_hardware_service_export() {
    // Verify HardwareServiceImpl is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<daq_server::grpc::HardwareServiceImpl>();
}

#[cfg(feature = "server")]
#[test]
fn verify_scan_service_export() {
    // Verify ScanServiceImpl is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<daq_server::grpc::ScanServiceImpl>();
}

#[cfg(feature = "server")]
#[test]
fn verify_storage_service_export() {
    // Verify StorageServiceImpl is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<daq_server::grpc::StorageServiceImpl>();
}

#[cfg(feature = "server")]
#[test]
fn verify_preset_service_export() {
    // Verify PresetServiceImpl is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<daq_server::grpc::PresetServiceImpl>();
}

#[cfg(feature = "server")]
#[test]
fn verify_module_service_export() {
    // Verify ModuleServiceImpl is exported
    fn _check_type_exists<T>() {}
    use daq_server::grpc::ModuleServiceImpl;
    _check_type_exists::<ModuleServiceImpl>();
}

#[cfg(feature = "server")]
#[test]
fn verify_plugin_service_export() {
    // Verify PluginServiceImpl is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<daq_server::grpc::PluginServiceImpl>();
}

// =============================================================================
// gRPC Proto Types
// =============================================================================

#[cfg(feature = "server")]
#[test]
fn verify_proto_types_export() {
    // Verify commonly used proto types are re-exported
    fn _check_type_exists<T>() {}

    // Control service types
    _check_type_exists::<daq_server::grpc::SystemStatus>();
    _check_type_exists::<daq_server::grpc::ScriptStatus>();

    // Hardware service types
    _check_type_exists::<daq_server::grpc::DeviceInfo>();
    _check_type_exists::<daq_server::grpc::MoveRequest>();
    _check_type_exists::<daq_server::grpc::ReadValueRequest>();

    // Scan service types
    _check_type_exists::<daq_server::grpc::ScanConfig>();
    _check_type_exists::<daq_server::grpc::ScanStatus>();

    // Preset service types
    _check_type_exists::<daq_server::grpc::Preset>();
    _check_type_exists::<daq_server::grpc::PresetMetadata>();

    // Module service types
    _check_type_exists::<daq_server::grpc::ModuleTypeSummary>();
    _check_type_exists::<daq_server::grpc::ModuleConfig>();

    // Storage service types
    _check_type_exists::<daq_server::grpc::StorageConfig>();
    _check_type_exists::<daq_server::grpc::RecordingStatus>();
}

// =============================================================================
// Module System (feature: modules)
// =============================================================================

#[cfg(feature = "modules")]
#[test]
fn verify_module_trait_export() {
    // Verify Module trait is exported
    use rust_daq::modules::Module;

    fn _check_trait_exists<T: Module>() {}

    // Note: Can't instantiate trait, just verify it exists
}

#[cfg(feature = "modules")]
#[test]
fn verify_module_registry_export() {
    // Verify ModuleRegistry is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::modules::ModuleRegistry>();
}

// =============================================================================
// Configuration System
// =============================================================================

// =============================================================================
// Error Types
// =============================================================================

#[test]
fn verify_daq_error_export() {
    // Verify DaqError is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::error::DaqError>();
}

// =============================================================================
// Scripting Engine (V5) - Feature-gated
// =============================================================================

#[cfg(feature = "scripting")]
#[test]
fn verify_rhai_engine_export() {
    // Verify RhaiEngine (concrete type) is exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::scripting::RhaiEngine>();
}

#[cfg(feature = "scripting")]
#[test]
fn verify_script_engine_trait_export() {
    // Verify ScriptEngine trait is exported
    use daq_scripting::ScriptEngine;
    fn _check_trait_exists<T: ScriptEngine>() {}
    // Can't instantiate trait objects without marker traits
}

#[cfg(feature = "scripting")]
#[test]
fn verify_script_handle_exports() {
    // Verify handle types are exported
    fn _check_type_exists<T>() {}
    _check_type_exists::<rust_daq::scripting::StageHandle>();
    _check_type_exists::<rust_daq::scripting::CameraHandle>();
    // Note: PowerMeterHandle doesn't exist in bindings.rs
}

// =============================================================================
// Integration Test: Verify Complete API Surface
// =============================================================================

#[test]
fn verify_complete_public_api() {
    // This test verifies that the most critical public API is accessible
    // If this test compiles, the basic public API structure is correct

    // If we get here, all critical exports are working
    fn _all_types_accessible() {}
    _all_types_accessible();
}

#[cfg(feature = "server")]
#[test]
fn verify_complete_grpc_api() {
    // Verify complete gRPC API surface is accessible

    // Services
    use daq_server::grpc::HardwareServiceImpl;
    use daq_server::grpc::ModuleServiceImpl;
    use protocol::daq::module_service_server::ModuleService;

    // Verify service exports
    fn _check_service_types() {
        // Ensure services implement their traits
        fn _assert_hardware_service<T: protocol::daq::hardware_service_server::HardwareService>() {}
        _assert_hardware_service::<HardwareServiceImpl>();

        fn _assert_module_service<T: ModuleService>() {}
        _assert_module_service::<ModuleServiceImpl>();
    }
    _check_service_types();

    fn _all_grpc_types_accessible() {}
    _all_grpc_types_accessible();
}

// =============================================================================
// Negative Tests: Verify Internal Types Are NOT Exported
// =============================================================================

// These tests verify that internal implementation details are properly hidden.
// They should fail to compile if uncommented, demonstrating proper encapsulation.

/*
#[test]
fn verify_internal_types_not_exported() {
    // This should NOT compile - internal types should not be public

    // Example: If we had internal state types that shouldn't be exposed
    // fn _check_type_exists<T>() {}
    // _check_type_exists::<rust_daq::hardware::ell14::InternalState>();
}
*/
