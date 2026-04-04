#[cfg(test)]
mod tests {
    use crate::grpc::error_mapping::{
        DRIVER_KIND_HEADER, DRIVER_TYPE_HEADER, ERROR_KIND_HEADER, anyhow_to_status,
        map_daq_error_to_status,
    };
    use common::error::{DaqError, DriverError, DriverErrorKind, StorageError, StorageErrorKind};
    use tonic::Code;

    fn assert_status_code(err: DaqError, expected: Code) {
        let status = map_daq_error_to_status(&err);
        assert_eq!(status.code(), expected);
    }

    fn assert_metadata(status: &tonic::Status, key: &str, expected: &str) {
        let value = status
            .metadata()
            .get(key)
            .and_then(|val| val.to_str().ok())
            .unwrap_or("<missing>");
        assert_eq!(value, expected, "metadata key '{key}' mismatch");
    }

    fn assert_has_error_kind(err: DaqError, expected_kind: &str) {
        let status = map_daq_error_to_status(&err);
        assert_metadata(&status, ERROR_KIND_HEADER, expected_kind);
    }

    mod configuration_errors {
        use super::*;

        #[test]
        fn config_error_maps_to_invalid_argument() {
            let err = DaqError::Config("bad config".into());
            assert_status_code(err, Code::InvalidArgument);
        }

        #[test]
        fn config_error_has_metadata() {
            assert_has_error_kind(DaqError::Config("bad".into()), "config");
        }

        #[test]
        fn configuration_error_maps_to_invalid_argument() {
            let err = DaqError::Configuration("bad config".into());
            assert_status_code(err, Code::InvalidArgument);
        }

        #[test]
        fn configuration_error_has_metadata() {
            assert_has_error_kind(DaqError::Configuration("bad".into()), "configuration");
        }
    }

    mod hardware_errors {
        use super::*;

        #[test]
        fn instrument_error_maps_to_unavailable() {
            // Hardware faults are expected runtime conditions, not server bugs
            let err = DaqError::Instrument("camera fault".into());
            assert_status_code(err, Code::Unavailable);
        }

        #[test]
        fn driver_init_error_maps_to_failed_precondition() {
            let err = DaqError::Driver(DriverError::new(
                "mock_camera",
                DriverErrorKind::Initialization,
                "failed",
            ));
            assert_status_code(err, Code::FailedPrecondition);
        }

        #[test]
        fn driver_error_includes_metadata() {
            let err = DaqError::Driver(DriverError::new(
                "mock_camera",
                DriverErrorKind::Initialization,
                "failed",
            ));
            let status = map_daq_error_to_status(&err);

            assert_metadata(&status, ERROR_KIND_HEADER, "driver");
            assert_metadata(&status, DRIVER_TYPE_HEADER, "mock_camera");
            assert_metadata(&status, DRIVER_KIND_HEADER, "initialization");
        }

        #[test]
        fn driver_config_error_maps_to_invalid_argument() {
            let err = DaqError::Driver(DriverError::new(
                "mock_camera",
                DriverErrorKind::Configuration,
                "bad config",
            ));
            assert_status_code(err, Code::InvalidArgument);
        }

        #[test]
        fn serial_port_not_connected_maps_to_unavailable() {
            assert_status_code(DaqError::SerialPortNotConnected, Code::Unavailable);
        }

        #[test]
        fn serial_port_not_connected_has_metadata() {
            assert_has_error_kind(DaqError::SerialPortNotConnected, "serial");
        }

        #[test]
        fn instrument_error_includes_metadata() {
            let err = DaqError::Instrument("camera fault".into());
            let status = map_daq_error_to_status(&err);

            assert_metadata(&status, ERROR_KIND_HEADER, "instrument");
        }

        #[test]
        fn serial_unexpected_eof_maps_to_aborted() {
            assert_status_code(DaqError::SerialUnexpectedEof, Code::Aborted);
        }

        #[test]
        fn serial_unexpected_eof_has_metadata() {
            assert_has_error_kind(DaqError::SerialUnexpectedEof, "serial_eof");
        }

        #[test]
        fn serial_feature_disabled_maps_to_unimplemented() {
            assert_status_code(DaqError::SerialFeatureDisabled, Code::Unimplemented);
        }

        #[test]
        fn serial_feature_disabled_has_metadata() {
            assert_has_error_kind(DaqError::SerialFeatureDisabled, "serial_disabled");
        }

        #[test]
        fn driver_timeout_error_maps_to_deadline_exceeded() {
            let err = DaqError::Driver(DriverError::new(
                "mock_camera",
                DriverErrorKind::Timeout,
                "operation timed out",
            ));
            assert_status_code(err, Code::DeadlineExceeded);
        }

        #[test]
        fn driver_permission_error_maps_to_permission_denied() {
            let err = DaqError::Driver(DriverError::new(
                "comedi",
                DriverErrorKind::Permission,
                "access denied",
            ));
            assert_status_code(err, Code::PermissionDenied);
        }

        #[test]
        fn driver_hardware_error_maps_to_unavailable() {
            let err = DaqError::Driver(DriverError::new(
                "comedi",
                DriverErrorKind::Hardware,
                "buffer overflow",
            ));
            assert_status_code(err, Code::Unavailable);
        }

        #[test]
        fn driver_invalid_parameter_maps_to_invalid_argument() {
            let err = DaqError::Driver(DriverError::new(
                "comedi",
                DriverErrorKind::InvalidParameter,
                "channel out of range",
            ));
            assert_status_code(err, Code::InvalidArgument);
        }

        #[test]
        fn driver_busy_maps_to_unavailable() {
            let err = DaqError::Driver(DriverError::new(
                "camera",
                DriverErrorKind::Busy,
                "acquisition in progress",
            ));
            assert_status_code(err, Code::Unavailable);
        }

        #[test]
        fn driver_not_found_maps_to_not_found() {
            let err = DaqError::Driver(DriverError::new(
                "stage",
                DriverErrorKind::NotFound,
                "device not connected",
            ));
            assert_status_code(err, Code::NotFound);
        }

        #[test]
        fn driver_safety_maps_to_failed_precondition() {
            let err = DaqError::Driver(DriverError::new(
                "laser",
                DriverErrorKind::Safety,
                "interlock open",
            ));
            assert_status_code(err, Code::FailedPrecondition);
        }

        #[test]
        fn driver_communication_error_metadata() {
            let err = DaqError::Driver(DriverError::new(
                "pvcam",
                DriverErrorKind::Communication,
                "usb disconnect",
            ));
            let status = map_daq_error_to_status(&err);
            assert_metadata(&status, ERROR_KIND_HEADER, "driver");
            assert_metadata(&status, DRIVER_TYPE_HEADER, "pvcam");
            assert_metadata(&status, DRIVER_KIND_HEADER, "communication");
        }

        #[test]
        fn driver_shutdown_maps_to_internal() {
            let err = DaqError::Driver(DriverError::new(
                "mock",
                DriverErrorKind::Shutdown,
                "cleanup failed",
            ));
            assert_status_code(err, Code::Internal);
        }

        #[test]
        fn driver_unknown_maps_to_internal() {
            let err = DaqError::Driver(DriverError::new(
                "mock",
                DriverErrorKind::Unknown,
                "mystery",
            ));
            assert_status_code(err, Code::Internal);
        }
    }

    mod runtime_errors {
        use super::*;

        #[test]
        fn processing_error_maps_to_internal() {
            let err = DaqError::Processing("fft failed".into());
            assert_status_code(err, Code::Internal);
        }

        #[test]
        fn processing_error_has_metadata() {
            assert_has_error_kind(DaqError::Processing("fft".into()), "processing");
        }

        #[test]
        fn frame_dimensions_too_large_maps_to_resource_exhausted() {
            let err = DaqError::FrameDimensionsTooLarge {
                width: 2048,
                height: 2048,
                max_dimension: 1024,
            };
            assert_status_code(err, Code::ResourceExhausted);
        }

        #[test]
        fn frame_dimensions_has_metadata() {
            let err = DaqError::FrameDimensionsTooLarge {
                width: 2048,
                height: 2048,
                max_dimension: 1024,
            };
            assert_has_error_kind(err, "frame_dimensions");
        }

        #[test]
        fn frame_too_large_maps_to_resource_exhausted() {
            let err = DaqError::FrameTooLarge {
                bytes: 4_096,
                max_bytes: 2_048,
            };
            assert_status_code(err, Code::ResourceExhausted);
        }

        #[test]
        fn response_too_large_maps_to_resource_exhausted() {
            let err = DaqError::ResponseTooLarge {
                bytes: 8_192,
                max_bytes: 4_096,
            };
            assert_status_code(err, Code::ResourceExhausted);
        }

        #[test]
        fn script_too_large_maps_to_resource_exhausted() {
            let err = DaqError::ScriptTooLarge {
                bytes: 8_192,
                max_bytes: 4_096,
            };
            assert_status_code(err, Code::ResourceExhausted);
        }

        #[test]
        fn size_overflow_maps_to_resource_exhausted() {
            let err = DaqError::SizeOverflow { context: "frame" };
            assert_status_code(err, Code::ResourceExhausted);
        }
    }

    mod module_errors {
        use super::*;

        #[test]
        fn module_operation_not_supported_maps_to_unimplemented() {
            let err = DaqError::ModuleOperationNotSupported("no frames".into());
            assert_status_code(err, Code::Unimplemented);
        }

        #[test]
        fn module_operation_not_supported_has_metadata() {
            assert_has_error_kind(
                DaqError::ModuleOperationNotSupported("x".into()),
                "module_unsupported",
            );
        }

        #[test]
        fn module_busy_during_operation_maps_to_unavailable() {
            assert_status_code(DaqError::ModuleBusyDuringOperation, Code::Unavailable);
        }

        #[test]
        fn module_busy_has_metadata() {
            assert_has_error_kind(DaqError::ModuleBusyDuringOperation, "module_busy");
        }

        #[test]
        fn camera_not_assigned_maps_to_failed_precondition() {
            assert_status_code(DaqError::CameraNotAssigned, Code::FailedPrecondition);
        }

        #[test]
        fn camera_not_assigned_has_metadata() {
            assert_has_error_kind(DaqError::CameraNotAssigned, "camera_not_assigned");
        }
    }

    mod feature_errors {
        use super::*;

        #[test]
        fn feature_not_enabled_maps_to_unimplemented() {
            let err = DaqError::FeatureNotEnabled("storage_hdf5".into());
            assert_status_code(err, Code::Unimplemented);
        }

        #[test]
        fn feature_not_enabled_has_metadata() {
            assert_has_error_kind(
                DaqError::FeatureNotEnabled("x".into()),
                "feature_not_enabled",
            );
        }

        #[test]
        fn feature_incomplete_maps_to_unimplemented() {
            let err = DaqError::FeatureIncomplete("driver".into(), "todo".into());
            assert_status_code(err, Code::Unimplemented);
        }

        #[test]
        fn feature_incomplete_has_metadata() {
            assert_has_error_kind(
                DaqError::FeatureIncomplete("x".into(), "y".into()),
                "feature_incomplete",
            );
        }
    }

    mod shutdown_errors {
        use super::*;

        #[test]
        fn shutdown_failed_maps_to_internal() {
            let err = DaqError::ShutdownFailed(vec![DaqError::Instrument("camera".into())]);
            assert_status_code(err, Code::Internal);
        }

        #[test]
        fn shutdown_failed_has_metadata() {
            assert_has_error_kind(DaqError::ShutdownFailed(vec![]), "shutdown_failed");
        }
    }

    mod parameter_errors {
        use super::*;

        #[test]
        fn parameter_no_subscribers_maps_to_failed_precondition() {
            // Cannot return Ok status from error mapper - handle benign suppression in service layer
            assert_status_code(DaqError::ParameterNoSubscribers, Code::FailedPrecondition);
        }

        #[test]
        fn parameter_no_subscribers_has_metadata() {
            assert_has_error_kind(DaqError::ParameterNoSubscribers, "parameter_no_subscribers");
        }

        #[test]
        fn parameter_read_only_maps_to_permission_denied() {
            assert_status_code(DaqError::ParameterReadOnly, Code::PermissionDenied);
        }

        #[test]
        fn parameter_read_only_has_metadata() {
            assert_has_error_kind(DaqError::ParameterReadOnly, "parameter_read_only");
        }

        #[test]
        fn parameter_invalid_choice_maps_to_invalid_argument() {
            assert_status_code(DaqError::ParameterInvalidChoice, Code::InvalidArgument);
        }

        #[test]
        fn parameter_invalid_choice_has_metadata() {
            assert_has_error_kind(DaqError::ParameterInvalidChoice, "parameter_invalid_choice");
        }

        #[test]
        fn parameter_no_hardware_reader_maps_to_failed_precondition() {
            assert_status_code(
                DaqError::ParameterNoHardwareReader,
                Code::FailedPrecondition,
            );
        }

        #[test]
        fn parameter_no_hardware_reader_has_metadata() {
            assert_has_error_kind(DaqError::ParameterNoHardwareReader, "parameter_no_reader");
        }
    }

    mod io_errors {
        use super::*;

        #[test]
        fn io_error_maps_to_internal() {
            let err = DaqError::Io(std::io::Error::other("io"));
            assert_status_code(err, Code::Internal);
        }

        #[test]
        fn io_error_has_metadata() {
            assert_has_error_kind(DaqError::Io(std::io::Error::other("x")), "io");
        }

        #[test]
        fn tokio_error_maps_to_internal() {
            let err = DaqError::Tokio(std::io::Error::other("tokio"));
            assert_status_code(err, Code::Internal);
        }

        #[test]
        fn tokio_error_has_metadata() {
            assert_has_error_kind(DaqError::Tokio(std::io::Error::other("x")), "tokio");
        }
    }

    mod storage_errors {
        use super::*;

        #[test]
        fn storage_config_error_maps_to_invalid_argument() {
            let err = DaqError::Storage(StorageError::new(
                StorageErrorKind::Configuration,
                "bad path",
            ));
            assert_status_code(err, Code::InvalidArgument);
        }

        #[test]
        fn storage_io_error_maps_to_internal() {
            let err = DaqError::Storage(StorageError::new(StorageErrorKind::Io, "disk full"));
            assert_status_code(err, Code::Internal);
        }

        #[test]
        fn storage_hdf5_error_maps_to_internal() {
            let err = DaqError::Storage(StorageError::new(StorageErrorKind::Hdf5, "corrupt file"));
            assert_status_code(err, Code::Internal);
        }

        #[test]
        fn storage_error_has_metadata() {
            assert_has_error_kind(
                DaqError::Storage(StorageError::new(StorageErrorKind::Other, "x")),
                "storage",
            );
        }
    }

    mod anyhow_downcast {
        use super::*;

        #[test]
        fn anyhow_with_daq_error_downcasts_correctly() {
            let err: anyhow::Error = DaqError::SerialPortNotConnected.into();
            let status = anyhow_to_status(err);
            assert_eq!(status.code(), Code::Unavailable);
            assert_metadata(&status, ERROR_KIND_HEADER, "serial");
        }

        #[test]
        fn anyhow_with_driver_error_downcasts_correctly() {
            let err: anyhow::Error =
                DriverError::new("pvcam", DriverErrorKind::Communication, "usb error").into();
            let status = anyhow_to_status(err);
            assert_eq!(status.code(), Code::Unavailable);
            assert_metadata(&status, ERROR_KIND_HEADER, "driver");
            assert_metadata(&status, DRIVER_TYPE_HEADER, "pvcam");
            assert_metadata(&status, DRIVER_KIND_HEADER, "communication");
        }

        #[test]
        fn anyhow_with_storage_error_downcasts_correctly() {
            let err: anyhow::Error =
                StorageError::new(StorageErrorKind::Configuration, "bad path").into();
            let status = anyhow_to_status(err);
            assert_eq!(status.code(), Code::InvalidArgument);
            assert_metadata(&status, ERROR_KIND_HEADER, "storage");
        }

        #[test]
        fn anyhow_unknown_error_falls_back_to_internal() {
            let err = anyhow::anyhow!("something unexpected");
            let status = anyhow_to_status(err);
            assert_eq!(status.code(), Code::Internal);
            assert_metadata(&status, ERROR_KIND_HEADER, "unknown");
            assert!(status.message().contains("something unexpected"));
        }

        #[test]
        fn anyhow_with_context_preserves_downcast() {
            // anyhow::Context wraps the error, but anyhow_to_status walks the
            // full chain so the buried DaqError is still found and mapped.
            use anyhow::Context;
            let result: Result<(), _> =
                Err(DaqError::ParameterReadOnly).context("while setting exposure");
            let err = result.expect_err("context wrapper should produce an Err");
            let status = anyhow_to_status(err);
            assert_eq!(status.code(), Code::PermissionDenied);
            assert_metadata(&status, ERROR_KIND_HEADER, "parameter_read_only");
        }
    }

    mod serde_error {
        use super::*;

        #[test]
        fn serde_error_maps_to_internal() {
            // Create a real serde_json error via a failed parse
            let err: serde_json::Error = serde_json::from_str::<String>("not valid json")
                .expect_err("invalid JSON should fail to parse");
            assert_status_code(DaqError::Serde(err), Code::Internal);
        }

        #[test]
        fn serde_error_has_metadata() {
            let err: serde_json::Error = serde_json::from_str::<String>("bad")
                .expect_err("invalid JSON should fail to parse");
            assert_has_error_kind(DaqError::Serde(err), "serde");
        }
    }
}
