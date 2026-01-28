#[cfg(test)]
mod tests {
    use crate::grpc::map_daq_error_to_status;
    use common::error::{DaqError, DriverError, DriverErrorKind};
    use tonic::Code;

    fn assert_status_code(err: DaqError, expected: Code) {
        let status = map_daq_error_to_status(err);
        assert_eq!(status.code(), expected);
    }

    fn assert_metadata(status: &tonic::Status, key: &str, expected: &str) {
        let value = status
            .metadata()
            .get(key)
            .and_then(|val| val.to_str().ok())
            .unwrap_or("<missing>");
        assert_eq!(value, expected);
    }

    mod configuration_errors {
        use super::*;

        #[test]
        fn config_error_maps_to_invalid_argument() {
            let err = DaqError::Config(config::ConfigError::Message("bad config".into()));
            assert_status_code(err, Code::InvalidArgument);
        }

        #[test]
        fn configuration_error_maps_to_invalid_argument() {
            let err = DaqError::Configuration("bad config".into());
            assert_status_code(err, Code::InvalidArgument);
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
            let status = map_daq_error_to_status(err);

            assert_metadata(&status, "x-daq-error-kind", "driver");
            assert_metadata(&status, "x-daq-driver-type", "mock_camera");
            assert_metadata(&status, "x-daq-driver-kind", "initialization");
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
        fn instrument_error_includes_metadata() {
            let err = DaqError::Instrument("camera fault".into());
            let status = map_daq_error_to_status(err);

            assert_metadata(&status, "x-daq-error-kind", "instrument");
        }

        #[test]
        fn serial_unexpected_eof_maps_to_aborted() {
            assert_status_code(DaqError::SerialUnexpectedEof, Code::Aborted);
        }

        #[test]
        fn serial_feature_disabled_maps_to_unimplemented() {
            assert_status_code(DaqError::SerialFeatureDisabled, Code::Unimplemented);
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
    }

    mod runtime_errors {
        use super::*;

        #[test]
        fn processing_error_maps_to_internal() {
            let err = DaqError::Processing("fft failed".into());
            assert_status_code(err, Code::Internal);
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
        fn module_busy_during_operation_maps_to_unavailable() {
            assert_status_code(DaqError::ModuleBusyDuringOperation, Code::Unavailable);
        }

        #[test]
        fn camera_not_assigned_maps_to_failed_precondition() {
            assert_status_code(DaqError::CameraNotAssigned, Code::FailedPrecondition);
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
        fn feature_incomplete_maps_to_unimplemented() {
            let err = DaqError::FeatureIncomplete("driver".into(), "todo".into());
            assert_status_code(err, Code::Unimplemented);
        }
    }

    mod shutdown_errors {
        use super::*;

        #[test]
        fn shutdown_failed_maps_to_internal() {
            let err = DaqError::ShutdownFailed(vec![DaqError::Instrument("camera".into())]);
            assert_status_code(err, Code::Internal);
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
        fn parameter_read_only_maps_to_permission_denied() {
            assert_status_code(DaqError::ParameterReadOnly, Code::PermissionDenied);
        }

        #[test]
        fn parameter_invalid_choice_maps_to_invalid_argument() {
            assert_status_code(DaqError::ParameterInvalidChoice, Code::InvalidArgument);
        }

        #[test]
        fn parameter_no_hardware_reader_maps_to_failed_precondition() {
            assert_status_code(
                DaqError::ParameterNoHardwareReader,
                Code::FailedPrecondition,
            );
        }
    }

    mod io_errors {
        use super::*;

        #[test]
        fn io_error_maps_to_internal() {
            let err = DaqError::Io(std::io::Error::new(std::io::ErrorKind::Other, "io"));
            assert_status_code(err, Code::Internal);
        }

        #[test]
        fn tokio_error_maps_to_internal() {
            let err = DaqError::Tokio(std::io::Error::new(std::io::ErrorKind::Other, "tokio"));
            assert_status_code(err, Code::Internal);
        }
    }
}
