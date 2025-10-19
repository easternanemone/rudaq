//! The storage manager panel for the GUI.
//!
//! This module provides a `StorageManager` struct and its implementation, which creates
//! a UI panel for managing data files saved by the application. This panel can be
//! displayed as a side panel in the main GUI.
//!
//! ## Features
//!
//! - **File Listing:** Displays a list of files from the configured storage directory.
//!   The list includes file name, size, and last modified timestamp.
//! - **Search/Filter:** A search box allows users to filter the list of files by name.
//! - **File Actions:**
//!   - **Open:** Opens the selected file using the system's default application (`opener` crate).
//!   - **Delete:** Deletes the selected file with a confirmation dialog.
//! - **File Preview:** When a file is selected, a preview panel shows its contents or metadata.
//!   - Supports CSV, HDF5, and Arrow file formats (HDF5 and Arrow previews are
//!     conditional on feature flags).
//!   - For CSV, it displays the first few rows in a table.
//!   - For HDF5, it lists the datasets in the root group.
//!   - For Arrow, it displays the file's schema.
//! - **Storage Statistics:** Shows basic statistics like the total number of files and
//!   their combined size.
//! - **Refresh:** A manual refresh button to reload the file list from the disk.
//!
//! The `StorageManager` is integrated into the main `Gui` and is rendered when toggled
//! by the user.

use crate::app::DaqApp;
use crate::measurement::Measure;
#[cfg(feature = "storage_arrow")]
use arrow2::io::ipc::read;
use csv;
use eframe::egui;
use egui_extras::{Column, TableBuilder};
#[cfg(feature = "storage_hdf5")]
use hdf5;
use log::{error, warn};
use opener;
use std::fs;
use std::path::PathBuf;

#[derive(Clone)]
struct FileInfo {
    path: PathBuf,
    name: String,
    size: u64,
    modified: std::time::SystemTime,
}

pub struct StorageManager {
    files: Vec<FileInfo>,
    search_query: String,
    storage_path: PathBuf,
    file_to_delete: Option<FileInfo>,
    needs_refresh: bool,
    selected_file: Option<FileInfo>,
}

impl Default for StorageManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageManager {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            search_query: String::new(),
            storage_path: PathBuf::new(),
            file_to_delete: None,
            needs_refresh: false,
            selected_file: None,
        }
    }

    fn refresh_files(&mut self) {
        self.files.clear();
        let entries = match fs::read_dir(&self.storage_path) {
            Ok(entries) => entries,
            Err(e) => {
                error!(
                    "Failed to read storage directory '{}': {}",
                    self.storage_path.display(),
                    e
                );
                return;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    warn!("Failed to read directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();
            if path.is_file() {
                let metadata = match entry.metadata() {
                    Ok(meta) => meta,
                    Err(e) => {
                        warn!("Failed to get metadata for '{}': {}", path.display(), e);
                        continue;
                    }
                };

                self.files.push(FileInfo {
                    name: path.file_name().unwrap().to_string_lossy().to_string(),
                    size: metadata.len(),
                    modified: metadata.modified().unwrap(),
                    path,
                });
            }
        }
    }

    fn storage_stats_ui(&self, ui: &mut egui::Ui) {
        let total_size: u64 = self.files.iter().map(|f| f.size).sum();
        let num_files = self.files.len();

        let mut largest_files = self.files.clone();
        largest_files.sort_by(|a, b| b.size.cmp(&a.size));
        largest_files.truncate(5);

        ui.group(|ui| {
            ui.heading("Storage Statistics");
            ui.label(format!("Total files: {}", num_files));
            ui.label(format!("Total size: {} bytes", total_size));
            ui.separator();
            ui.label("Largest files:");
            for file in largest_files {
                ui.label(format!("- {} ({} bytes)", file.name, file.size));
            }
        });
    }

    pub fn ui<M>(&mut self, ui: &mut egui::Ui, app: &DaqApp<M>)
    where
        M: Measure + 'static,
        M::Data: Into<daq_core::DataPoint>,
    {
        let storage_path =
            PathBuf::from(&app.with_inner(|inner| inner.settings.storage.default_path.clone()));
        if self.storage_path != storage_path {
            self.storage_path = storage_path;
            self.refresh_files();
        }

        ui.heading("Storage Manager");

        self.storage_stats_ui(ui);

        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.search_query);
            if ui.button("Refresh").clicked() {
                self.refresh_files();
            }
        });

        ui.separator();

        let table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto())
            .column(Column::auto())
            .column(Column::auto())
            .column(Column::auto())
            .min_scrolled_height(0.0);

        table
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Filename");
                });
                header.col(|ui| {
                    ui.strong("Size");
                });
                header.col(|ui| {
                    ui.strong("Modified");
                });
                header.col(|ui| {
                    ui.strong("Actions");
                });
            })
            .body(|mut body| {
                let files: Vec<_> = self
                    .files
                    .iter()
                    .filter(|f| f.name.contains(&self.search_query))
                    .cloned()
                    .collect();
                for file in &files {
                    body.row(30.0, |mut row| {
                        row.col(|ui| {
                            if ui
                                .selectable_label(
                                    self.selected_file
                                        .as_ref()
                                        .is_some_and(|f| f.path == file.path),
                                    &file.name,
                                )
                                .clicked()
                            {
                                self.selected_file = Some(file.clone());
                            }
                        });
                        row.col(|ui| {
                            ui.label(format!("{} bytes", file.size));
                        });
                        row.col(|ui| {
                            let datetime: chrono::DateTime<chrono::Local> = file.modified.into();
                            ui.label(datetime.format("%Y-%m-%d %H:%M:%S").to_string());
                        });
                        row.col(|ui| {
                            ui.horizontal(|ui| {
                                if ui.button("Open").clicked() {
                                    if let Err(e) = opener::open(&file.path) {
                                        error!(
                                            "Failed to open file '{}': {}",
                                            file.path.display(),
                                            e
                                        );
                                    }
                                }
                                if ui.button("Delete").clicked() {
                                    self.file_to_delete = Some(file.clone());
                                }
                            });
                        });
                    });
                }
            });

        if let Some(file_to_delete) = &self.file_to_delete.clone() {
            let mut open = true;
            egui::Window::new("Confirm Deletion")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    ui.label(format!(
                        "Are you sure you want to delete '{}'?",
                        file_to_delete.name
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.file_to_delete = None;
                        }
                        if ui.button("Delete").clicked() {
                            if let Err(e) = fs::remove_file(&file_to_delete.path) {
                                error!(
                                    "Failed to delete file '{}': {}",
                                    file_to_delete.path.display(),
                                    e
                                );
                            } else {
                                self.needs_refresh = true;
                            }
                            self.file_to_delete = None;
                        }
                    });
                });
            if !open {
                self.file_to_delete = None;
            }
        }

        if self.needs_refresh {
            self.refresh_files();
            self.needs_refresh = false;
        }

        ui.separator();

        if let Some(selected_file) = &self.selected_file {
            ui.group(|ui| {
                ui.heading("Preview");
                ui.label(format!("File: {}", selected_file.name));
                ui.separator();
                self.file_preview(ui, selected_file);
            });
        }
    }

    fn file_preview(&self, ui: &mut egui::Ui, file: &FileInfo) {
        let extension = file.path.extension().and_then(|s| s.to_str());
        match extension {
            Some("csv") => self.csv_preview(ui, file),
            #[cfg(feature = "storage_hdf5")]
            Some("h5") | Some("hdf5") => self.hdf5_preview(ui, file),
            #[cfg(feature = "storage_arrow")]
            Some("arrow") => self.arrow_preview(ui, file),
            _ => {
                ui.label("Preview not available for this file type.");
            }
        }
    }

    #[cfg(feature = "storage_hdf5")]
    fn hdf5_preview(&self, ui: &mut egui::Ui, file: &FileInfo) {
        match hdf5::File::open(&file.path) {
            Ok(f) => {
                ui.label("HDF5 File Structure:");
                f.group("/")
                    .unwrap()
                    .member_names()
                    .unwrap()
                    .iter()
                    .for_each(|name| {
                        ui.label(format!("- {}", name));
                    });
            }
            Err(e) => {
                error!("Failed to read HDF5 file '{}': {}", file.path.display(), e);
                ui.label("Error reading HDF5 file.");
            }
        }
    }

    #[cfg(feature = "storage_arrow")]
    fn arrow_preview(&self, ui: &mut egui::Ui, file: &FileInfo) {
        let mut reader = match std::fs::File::open(&file.path) {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to open Arrow file '{}': {}", file.path.display(), e);
                ui.label("Error opening Arrow file.");
                return;
            }
        };
        match read::read_file_metadata(&mut reader) {
            Ok(metadata) => {
                ui.label("Arrow File Schema:");
                for field in metadata.schema.fields {
                    ui.label(format!("- {}: {:?}", field.name, field.data_type));
                }
            }
            Err(e) => {
                error!(
                    "Failed to read Arrow metadata from '{}': {}",
                    file.path.display(),
                    e
                );
                ui.label("Error reading Arrow file metadata.");
            }
        }
    }

    fn csv_preview(&self, ui: &mut egui::Ui, file: &FileInfo) {
        let mut reader = match csv::Reader::from_path(&file.path) {
            Ok(reader) => reader,
            Err(e) => {
                error!("Failed to read CSV file '{}': {}", file.path.display(), e);
                ui.label("Error reading CSV file.");
                return;
            }
        };

        let headers = match reader.headers() {
            Ok(headers) => headers.clone(),
            Err(e) => {
                error!(
                    "Failed to read CSV headers for '{}': {}",
                    file.path.display(),
                    e
                );
                ui.label("Error reading CSV headers.");
                return;
            }
        };

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .header(20.0, |mut header_row| {
                for header in headers.iter() {
                    header_row.col(|ui| {
                        ui.strong(header);
                    });
                }
            })
            .body(|mut body| {
                for (i, result) in reader.records().enumerate() {
                    if i >= 10 {
                        break;
                    }
                    let record = match result {
                        Ok(record) => record,
                        Err(e) => {
                            warn!("Error reading CSV record: {}", e);
                            continue;
                        }
                    };
                    body.row(20.0, |mut row| {
                        for field in record.iter() {
                            row.col(|ui| {
                                ui.label(field);
                            });
                        }
                    });
                }
            });
    }
}
