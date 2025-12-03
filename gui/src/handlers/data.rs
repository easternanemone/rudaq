//! Data/Storage handlers
//!
//! Handles data export and storage configuration callbacks.
//! Part of GUI Phase 4 (bd-0stf)
//!
//! NOTE: This is a stub implementation. Full functionality requires
//! a StorageService in the backend.

use crate::state::SharedState;
use crate::ui::{
    AcquisitionInfo, MainWindow, SharedString, StorageFormat, StorageStatus, UiAdapter, VecModel,
    Weak,
};
use std::rc::Rc;
use tracing::{info, warn};

/// Register data-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, _state: SharedState) {
    let ui_weak = adapter.weak();
    register_refresh_acquisitions(ui, ui_weak.clone());
    register_configure_storage(ui, ui_weak.clone());
    register_export_acquisition(ui, ui_weak.clone());
    register_delete_acquisition(ui, ui_weak);

    // Initialize with default storage formats
    initialize_storage_formats(ui);
}

fn initialize_storage_formats(ui: &MainWindow) {
    // Provide available storage formats
    // NOTE: Feature detection happens server-side; here we show all formats
    // and the server will report which are actually available
    let formats = vec![
        StorageFormat {
            id: SharedString::from("hdf5"),
            name: SharedString::from("HDF5"),
            extension: SharedString::from(".h5"),
            available: true,
        },
        StorageFormat {
            id: SharedString::from("csv"),
            name: SharedString::from("CSV"),
            extension: SharedString::from(".csv"),
            available: true,
        },
        StorageFormat {
            id: SharedString::from("arrow"),
            name: SharedString::from("Apache Arrow"),
            extension: SharedString::from(".parquet"),
            available: true,
        },
        StorageFormat {
            id: SharedString::from("netcdf"),
            name: SharedString::from("NetCDF"),
            extension: SharedString::from(".nc"),
            available: false, // Typically less common
        },
    ];

    let model = Rc::new(VecModel::from(formats));
    ui.set_storage_formats(model.into());

    // Initialize storage status with defaults
    // Note: Using values that fit in i32 (max ~2GB display)
    ui.set_storage_status(StorageStatus {
        enabled: true,
        current_format: SharedString::from("CSV"),
        output_directory: SharedString::from("./data"),
        disk_free_bytes: 100_000_000, // 100 MB (stub, actual would come from server)
        disk_total_bytes: 500_000_000, // 500 MB (stub)
        active_writers: 0,
    });
}

fn register_refresh_acquisitions(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_refresh_acquisitions(move || {
        info!("Refreshing acquisitions list");

        // Stub: In the future, this would query the backend for recent acquisitions
        // For now, provide some example data
        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
            let acquisitions = vec![
                AcquisitionInfo {
                    acquisition_id: SharedString::from("acq-001"),
                    name: SharedString::from("Power Scan 2024-01-15"),
                    start_time: SharedString::from("2024-01-15 10:30:00"),
                    end_time: SharedString::from("2024-01-15 10:35:00"),
                    status: SharedString::from("completed"),
                    total_points: 100,
                    file_path: SharedString::from("./data/power_scan_001.csv"),
                    format: SharedString::from("CSV"),
                    size_bytes: 1_234_567,
                },
                AcquisitionInfo {
                    acquisition_id: SharedString::from("acq-002"),
                    name: SharedString::from("Position Calibration"),
                    start_time: SharedString::from("2024-01-15 11:00:00"),
                    end_time: SharedString::from("2024-01-15 11:15:00"),
                    status: SharedString::from("completed"),
                    total_points: 500,
                    file_path: SharedString::from("./data/calibration_002.h5"),
                    format: SharedString::from("HDF5"),
                    size_bytes: 5_678_901,
                },
            ];

            let model = Rc::new(VecModel::from(acquisitions));
            ui.set_acquisitions(model.into());

            ui.invoke_show_toast(
                SharedString::from("info"),
                SharedString::from("Acquisitions Refreshed"),
                SharedString::from("Found 2 acquisitions (stub data)"),
            );
        });
    });
}

fn register_configure_storage(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_configure_storage(move |format_id, output_dir| {
        let format_id = format_id.to_string();
        let output_dir = output_dir.to_string();

        info!(
            "Configuring storage: format={}, dir={}",
            format_id, output_dir
        );

        // Stub: In the future, this would configure the backend storage
        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
            // Update the storage status to reflect new configuration
            let mut status = ui.get_storage_status();
            status.current_format = SharedString::from(&format_id.to_uppercase());
            if !output_dir.is_empty() {
                status.output_directory = SharedString::from(&output_dir);
            }
            ui.set_storage_status(status);

            ui.invoke_show_toast(
                SharedString::from("success"),
                SharedString::from("Storage Configured"),
                SharedString::from(format!("Format: {}", format_id.to_uppercase())),
            );
        });
    });
}

fn register_export_acquisition(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_export_acquisition(move |request| {
        let acq_id = request.acquisition_id.to_string();
        let format = request.target_format.to_string();
        let path = request.output_path.to_string();
        let metadata = request.include_metadata;
        let compress = request.compress;

        info!(
            "Export requested: {} -> {} (format={}, metadata={}, compress={})",
            acq_id, path, format, metadata, compress
        );

        // Stub: In the future, this would trigger an export job
        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
            warn!("Export is a stub - no actual export performed");

            ui.invoke_show_toast(
                SharedString::from("warning"),
                SharedString::from("Export (Stub)"),
                SharedString::from(format!("Would export {} to {}", acq_id, path)),
            );
        });
    });
}

fn register_delete_acquisition(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_delete_acquisition(move |acq_id| {
        let acq_id = acq_id.to_string();

        info!("Deleting acquisition: {}", acq_id);

        // Stub: In the future, this would delete the acquisition
        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
            warn!("Delete is a stub - no actual deletion performed");

            ui.invoke_show_toast(
                SharedString::from("warning"),
                SharedString::from("Delete (Stub)"),
                SharedString::from(format!("Would delete {}", acq_id)),
            );
        });
    });
}
