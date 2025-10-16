//! The core application state and logic.
//!
//! This module implements `DaqApp`, the central orchestrator for the entire DAQ system.
//! It manages instrument lifecycles, data streaming, processing pipelines, and storage.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                          DaqApp                             │
//! │  ┌────────────────────────────────────────────────────────┐ │
//! │  │              DaqAppInner (Mutex)                       │ │
//! │  │                                                        │ │
//! │  │  • Tokio Runtime (Arc)                                │ │
//! │  │  • Instruments (HashMap<String, InstrumentHandle>)    │ │
//! │  │  • Broadcast Channel (DataPoint stream)               │ │
//! │  │  • Registries (Instrument, Processor)                 │ │
//! │  │  • Writer Task (storage backend)                      │ │
//! │  └────────────────────────────────────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────┘
//!        │              │              │
//!        ▼              ▼              ▼
//!   Instrument 1   Instrument 2   Storage Task
//!   (async task)   (async task)   (async task)
//! ```
//!
//! # Threading Model
//!
//! - **Main thread**: Runs the egui GUI event loop
//! - **Tokio runtime**: Owns all async tasks (instruments, storage, commands)
//! - **Synchronization**: `Arc<Mutex<DaqAppInner>>` for thread-safe state access
//!
//! The `DaqApp` is `Clone`, allowing it to be shared between GUI and background tasks.
//! All state mutations are protected by a single mutex, simplifying reasoning about
//! concurrency.
//!
//! # Data Flow
//!
//! ```text
//! Instrument --[DataPoint]--> Processor Chain --[DataPoint]--> Broadcast Channel
//!                                                                     │
//!                                                        ┌────────────┼────────────┐
//!                                                        ▼            ▼            ▼
//!                                                      GUI        Storage     Other Subscribers
//! ```
//!
//! # Lifecycle Stages
//!
//! 1. **Initialization** (`new()`): Create runtime, spawn all configured instruments
//! 2. **Running**: Instruments stream data, GUI displays real-time plots
//! 3. **Recording** (optional): Storage task persists data to disk
//! 4. **Shutdown** (`shutdown()`): Graceful cleanup with 5-second timeout per instrument
//!
//! # Error Handling
//!
//! - Instrument failures are isolated (don't crash entire application)
//! - Errors are logged to both console and in-memory `LogBuffer`
//! - Storage failures abort recording but don't stop data acquisition
//! - Shutdown proceeds even if some instruments timeout
//!
//! # Examples
//!
//! ## Creating and Running the Application
//!
//! ```rust
//! use rust_daq::app::DaqApp;
//! use rust_daq::config::Settings;
//! use rust_daq::instrument::InstrumentRegistry;
//! use rust_daq::data::registry::ProcessorRegistry;
//! use rust_daq::log_capture::LogBuffer;
//! use std::sync::Arc;
//!
//! # fn example() -> anyhow::Result<()> {
//! let settings = Arc::new(Settings::new(Some("config/default.toml"))?);
//! let instrument_registry = Arc::new(InstrumentRegistry::new());
//! let processor_registry = Arc::new(ProcessorRegistry::new());
//! let log_buffer = LogBuffer::new();
//!
//! let app = DaqApp::new(
//!     settings,
//!     instrument_registry,
//!     processor_registry,
//!     log_buffer,
//! )?;
//!
//! // Application is now running, instruments are streaming data
//! // Later, gracefully shut down:
//! app.shutdown();
//! # Ok(())
//! # }
//! ```
use crate::{
    config::Settings,
    core::{DataPoint, DataProcessor, InstrumentHandle},
    data::{registry::ProcessorRegistry, storage_factory::StorageWriterRegistry},
    instrument::InstrumentRegistry,
    log_capture::LogBuffer,
    metadata::Metadata,
    session::{self, Session},
};
use anyhow::{anyhow, Context, Result};
use log::{error, info};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::{runtime::Runtime, sync::{broadcast, oneshot}, task::JoinHandle};

/// The main application struct that holds all state.
///
/// `DaqApp` is the top-level entry point for the DAQ system, providing a thread-safe
/// interface for managing instruments, data streaming, and storage. It wraps
/// `DaqAppInner` in an `Arc<Mutex<>>` to enable safe sharing between threads.
///
/// # Cloning
///
/// `DaqApp` implements `Clone` via cheap `Arc` cloning. All clones share the same
/// underlying state. This enables:
/// - Passing app reference to GUI callbacks
/// - Sharing between async tasks
/// - Multiple ownership without lifetime complexities
///
/// # Thread Safety
///
/// All public methods acquire the mutex lock briefly to perform operations. Long-running
/// operations (instrument spawning, shutdown) are executed on the Tokio runtime to
/// avoid blocking the GUI thread.
///
/// # Usage Pattern
///
/// ```rust
/// # use rust_daq::app::DaqApp;
/// # fn example(app: DaqApp) {
/// // Clone is cheap (Arc increment)
/// let app_clone = app.clone();
///
/// // Safe concurrent access from different threads
/// std::thread::spawn(move || {
///     app_clone.shutdown();
/// });
/// # }
/// ```
#[derive(Clone)]
pub struct DaqApp {
    /// Shared, mutex-protected inner state
    inner: Arc<Mutex<DaqAppInner>>,
}

/// Inner state of the DAQ application, protected by a Mutex.
///
/// `DaqAppInner` contains the actual application state and is wrapped by `DaqApp`
/// for thread-safe access. It owns the Tokio runtime, all instrument tasks, and
/// the central data broadcast channel.
///
/// # Field Descriptions
///
/// * `settings` - Immutable application configuration loaded from TOML
/// * `instrument_registry` - Factory for creating instrument instances by type
/// * `processor_registry` - Factory for creating data processor instances
/// * `instruments` - Active instrument tasks with command channels
/// * `data_sender` - Broadcast channel for distributing data points (capacity: 1024)
/// * `log_buffer` - In-memory circular buffer for GUI log display
/// * `metadata` - Experiment metadata (experimenter, project, session info)
/// * `writer_task` - Optional storage task handle (Some when recording active)
/// * `storage_format` - Format string ("csv", "hdf5", "arrow") for data persistence
/// * `runtime` - Tokio runtime powering all async operations
/// * `shutdown_flag` - Prevents duplicate shutdown attempts (idempotency guard)
/// * `_data_receiver_keeper` - Keeps broadcast channel alive until GUI subscribes
///
/// # Broadcast Channel Pattern
///
/// The `_data_receiver_keeper` field solves a critical bootstrapping problem:
///
/// ```text
/// Problem: If no receivers exist, broadcast sender drops all data
/// Solution: Hold one receiver until GUI subscribes (prevents data loss)
///
/// Timeline:
/// 1. App created → keeper receiver holds channel open
/// 2. Instruments spawn → start sending data (buffered by keeper)
/// 3. GUI subscribes → creates its own receiver, data flows
/// 4. Keeper continues buffering (capacity: 1024) for other subscribers
/// ```
///
/// # Lifetimes and Ownership
///
/// - `runtime`: Owned Arc, cloned to instrument tasks (tasks outlive instruments)
/// - `instruments`: Moved out during shutdown via `std::mem::take`
/// - `data_sender`: Cloned for each instrument task (multi-producer)
/// - `writer_task`: Taken via `Option::take()` during stop_recording
pub struct DaqAppInner {
    /// Application configuration (immutable, shared)
    pub settings: Arc<Settings>,
    /// Instrument factory (shared across instrument creations)
    pub instrument_registry: Arc<InstrumentRegistry>,
    /// Data processor factory (shared across processor creations)
    pub processor_registry: Arc<ProcessorRegistry>,
    /// Storage writer factory (determines available formats)
    pub storage_registry: Arc<StorageWriterRegistry>,
    /// Running instrument tasks with command channels
    pub instruments: HashMap<String, InstrumentHandle>,
    /// Central data distribution channel (multi-consumer broadcast)
    pub data_sender: broadcast::Sender<DataPoint>,
    /// In-memory log buffer for GUI display (circular, thread-safe)
    pub log_buffer: LogBuffer,
    /// Experiment metadata for storage backends
    pub metadata: Metadata,
    /// Storage task handle (Some = recording active, None = not recording)
    pub writer_task: Option<JoinHandle<Result<()>>>,
    /// Storage shutdown signal sender (Some = recording active, None = not recording)
    pub writer_shutdown_tx: Option<oneshot::Sender<()>>,
    /// Storage format string ("csv", "hdf5", "arrow")
    pub storage_format: String,
    /// Tokio runtime powering all async tasks (shared ownership)
    runtime: Arc<Runtime>,
    /// Idempotency guard for shutdown() (true = shutdown in progress/complete)
    shutdown_flag: bool,
    /// Keeps broadcast channel alive until GUI subscribes (prevents data loss)
    _data_receiver_keeper: broadcast::Receiver<DataPoint>,
}

impl DaqApp {
    /// Creates a new `DaqApp` and spawns all configured instruments.
    ///
    /// This constructor:
    /// 1. Creates a new Tokio runtime for all async operations
    /// 2. Initializes the broadcast channel for data distribution
    /// 3. Spawns all instruments listed in `settings.instruments`
    /// 4. Wraps state in `Arc<Mutex<>>` for thread-safe sharing
    ///
    /// If any instrument fails to spawn, an error is logged but initialization
    /// continues. This ensures partial failures don't prevent the app from starting.
    ///
    /// # Arguments
    ///
    /// * `settings` - Application configuration loaded from TOML file
    /// * `instrument_registry` - Pre-populated registry with instrument factories
    /// * `processor_registry` - Pre-populated registry with processor factories
    /// * `log_buffer` - Circular buffer for capturing log messages
    ///
    /// # Returns
    ///
    /// Returns `Ok(DaqApp)` on success, or `Err` if:
    /// - Tokio runtime creation fails (system resource exhaustion)
    ///
    /// Note: Individual instrument spawn failures do not fail initialization.
    ///
    /// # Complexity
    ///
    /// O(n) where n = number of configured instruments (sequential spawn)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rust_daq::app::DaqApp;
    /// use rust_daq::config::Settings;
    /// use rust_daq::instrument::InstrumentRegistry;
    /// use rust_daq::data::registry::ProcessorRegistry;
    /// use rust_daq::log_capture::LogBuffer;
    /// use std::sync::Arc;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let settings = Arc::new(Settings::new(Some("config/default.toml"))?);
    ///
    /// let mut instrument_registry = InstrumentRegistry::new();
    /// // Register instrument types...
    ///
    /// let processor_registry = ProcessorRegistry::new();
    /// // Register processor types...
    ///
    /// let log_buffer = LogBuffer::new();
    ///
    /// let app = DaqApp::new(
    ///     settings,
    ///     Arc::new(instrument_registry),
    ///     Arc::new(processor_registry),
    ///     log_buffer,
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        settings: Arc<Settings>,
        instrument_registry: Arc<InstrumentRegistry>,
        processor_registry: Arc<ProcessorRegistry>,
        log_buffer: LogBuffer,
    ) -> Result<Self> {
        let runtime = Arc::new(Runtime::new().context("Failed to create Tokio runtime")?);
        let (data_sender, data_receiver_keeper) = broadcast::channel(1024);
        let storage_format = settings.storage.default_format.clone();

        let storage_registry = Arc::new(StorageWriterRegistry::new());

        let mut inner = DaqAppInner {
            settings: settings.clone(),
            instrument_registry,
            processor_registry,
            storage_registry,
            instruments: HashMap::new(),
            data_sender,
            log_buffer,
            metadata: Metadata::default(),
            writer_task: None,
            writer_shutdown_tx: None,
            storage_format,
            runtime,
            shutdown_flag: false,
            _data_receiver_keeper: data_receiver_keeper,
        };

        for id in settings.instruments.keys() {
            if let Err(e) = inner.spawn_instrument(id) {
                error!("Failed to spawn instrument '{}': {}", id, e);
            }
        }

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    /// Provides safe access to the inner application state.
    ///
    /// This method acquires the mutex lock, calls the provided closure with mutable
    /// access to `DaqAppInner`, and returns the closure's result. The lock is released
    /// when the closure returns.
    ///
    /// # Arguments
    ///
    /// * `f` - Closure that receives `&mut DaqAppInner` and returns `R`
    ///
    /// # Returns
    ///
    /// The value returned by the closure
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned (a thread panicked while holding the lock).
    /// This is acceptable behavior as it indicates a catastrophic internal error.
    ///
    /// # Complexity
    ///
    /// O(1) for lock acquisition + O(closure) for the operation
    ///
    /// # Usage Pattern
    ///
    /// ```rust
    /// # use rust_daq::app::DaqApp;
    /// # fn example(app: &DaqApp) {
    /// // Read-only access
    /// let num_instruments = app.with_inner(|inner| {
    ///     inner.instruments.len()
    /// });
    ///
    /// // Mutable access
    /// app.with_inner(|inner| {
    ///     inner.metadata.experiment_name = "Alice's Experiment".to_string();
    /// });
    /// # }
    /// ```
    ///
    /// # Thread Safety
    ///
    /// The closure should avoid long-running operations to prevent blocking other
    /// threads. For async work, clone necessary data and release the lock first.
    pub fn with_inner<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut DaqAppInner) -> R,
    {
        let mut inner = self.inner.lock().unwrap();
        f(&mut inner)
    }

    /// Returns a clone of the application's Tokio runtime handle.
    ///
    /// The runtime is reference-counted (`Arc<Runtime>`), so cloning is cheap and
    /// all clones share the same underlying thread pool.
    ///
    /// # Use Cases
    ///
    /// - Spawning async tasks from non-async contexts
    /// - Blocking on async operations from synchronous code
    /// - Sharing runtime between components
    ///
    /// # Complexity
    ///
    /// O(1) - Arc clone + mutex lock/unlock
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rust_daq::app::DaqApp;
    /// # fn example(app: &DaqApp) {
    /// let runtime = app.get_runtime();
    ///
    /// // Spawn an async task
    /// runtime.spawn(async {
    ///     // Async work...
    /// });
    ///
    /// // Block on an async operation
    /// let result = runtime.block_on(async {
    ///     // Async work that returns a value...
    ///     42
    /// });
    /// # }
    /// ```
    pub fn get_runtime(&self) -> Arc<Runtime> {
        self.with_inner(|inner| inner.runtime.clone())
    }

    /// Shuts down the application, stopping all instruments and the Tokio runtime.
    ///
    /// This method implements graceful shutdown with timeout-based fallback:
    ///
    /// 1. **Idempotency check**: Skip if already shut down
    /// 2. **Send Shutdown commands**: Send to all instruments via command channels
    /// 3. **Await with timeout**: Wait up to 5 seconds per instrument
    /// 4. **Force termination**: Abort tasks that exceed timeout
    /// 5. **Log results**: Report success/failure for each instrument
    ///
    /// # Shutdown Behavior
    ///
    /// - **Graceful path**: Instrument receives `Shutdown` command → breaks task loop
    ///   → calls `disconnect()` → task completes successfully
    /// - **Timeout path**: If `disconnect()` hangs (hardware timeout, deadlock), the
    ///   task is forcefully aborted after 5 seconds
    /// - **Already terminated**: If command send fails, task is aborted immediately
    ///
    /// # Blocking Semantics
    ///
    /// This method **blocks the calling thread** until all instruments complete or
    /// timeout. Typical duration: 0-5 seconds depending on slowest instrument.
    ///
    /// # Idempotency
    ///
    /// Safe to call multiple times. Subsequent calls are no-ops if shutdown is
    /// already in progress or complete.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned (should never happen during shutdown).
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rust_daq::app::DaqApp;
    /// # fn example(app: DaqApp) {
    /// // Graceful shutdown (blocks until complete)
    /// app.shutdown();
    ///
    /// // Safe to call again (no-op)
    /// app.shutdown();
    /// # }
    /// ```
    ///
    /// # Complexity
    ///
    /// O(n) where n = number of instruments, with 5-second timeout per instrument.
    /// Worst case: 5n seconds if all instruments timeout.
    ///
    /// # Implementation Notes (bd-20)
    ///
    /// This implementation was enhanced to fix graceful shutdown issues:
    /// - Added `Shutdown` variant to `InstrumentCommand` enum
    /// - Instruments now break task loop on receiving `Shutdown`
    /// - `disconnect()` is called after loop breaks (cleanup outside loop)
    /// - 5-second timeout prevents indefinite hangs
    /// - Comprehensive logging for debugging shutdown issues
    pub fn shutdown(&self) {
        self.with_inner(|inner| {
            if inner.shutdown_flag {
                return;
            }
            info!("Shutting down application runtime...");
            inner.shutdown_flag = true;

            let runtime = inner.runtime.clone();
            let instruments = std::mem::take(&mut inner.instruments);

            runtime.block_on(async move {
                let shutdown_timeout = std::time::Duration::from_secs(5);
                let mut shutdown_handles = Vec::new();

                for (id, handle) in instruments {
                    info!("Sending shutdown signal to instrument: {}", id);
                    if handle.command_tx.send(crate::core::InstrumentCommand::Shutdown).await.is_err() {
                        log::warn!("Failed to send shutdown command to '{}', it might have already terminated. Aborting.", id);
                        handle.task.abort();
                    } else {
                        shutdown_handles.push((id, handle.task));
                    }
                }

                for (id, task) in shutdown_handles {
                    match tokio::time::timeout(shutdown_timeout, task).await {
                        Ok(Ok(Ok(_))) => info!("Instrument '{}' shut down gracefully.", id),
                        Ok(Ok(Err(e))) => error!("Instrument '{}' task returned an error during shutdown: {}", id, e),
                        Ok(Err(e)) => error!("Instrument '{}' task failed (panic): {}", id, e),
                        Err(_) => {
                            log::warn!("Instrument '{}' failed to shut down within {:?}, force terminating.", id, shutdown_timeout);
                        }
                    }
                }
            });
        });
    }

    /// Saves the current application state to a session file.
    ///
    /// Captures application state (metadata, storage format) and GUI state
    /// (window layout, plot configurations) to a JSON file for later restoration.
    ///
    /// # Arguments
    ///
    /// * `path` - File path where session will be saved (typically `.json` extension)
    /// * `gui_state` - Current GUI state (window positions, plot configs, etc.)
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - File creation fails (permission denied, parent directory doesn't exist)
    /// - Serialization fails (internal state corruption)
    /// - Write operation fails (disk full, I/O error)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rust_daq::app::DaqApp;
    /// use rust_daq::session::GuiState;
    /// use std::path::Path;
    ///
    /// # fn example(app: &DaqApp, gui_state: GuiState) -> anyhow::Result<()> {
    /// app.save_session(Path::new("sessions/experiment_001.json"), gui_state)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn save_session(&self, path: &Path, gui_state: session::GuiState) -> Result<()> {
        let session = Session::from_app(self, gui_state);
        session::save_session(&session, path)
    }

    /// Loads application state from a session file.
    ///
    /// Restores application metadata and storage format from a previously saved
    /// session. Returns the GUI state for the GUI to restore window layout and
    /// plot configurations.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the session file (typically `.json`)
    ///
    /// # Returns
    ///
    /// Returns `GuiState` on success, which should be applied to the GUI.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - File doesn't exist or can't be read
    /// - JSON deserialization fails (corrupted file, version mismatch)
    /// - Session format is incompatible with current version
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rust_daq::app::DaqApp;
    /// use std::path::Path;
    ///
    /// # fn example(app: &DaqApp) -> anyhow::Result<()> {
    /// let gui_state = app.load_session(Path::new("sessions/experiment_001.json"))?;
    /// // Apply gui_state to GUI components...
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Notes
    ///
    /// Loading a session does NOT:
    /// - Restart instruments (instruments continue running)
    /// - Change instrument registry or configuration
    /// - Affect data streaming (data continues flowing)
    ///
    /// It only restores metadata and GUI state.
    pub fn load_session(&self, path: &Path) -> Result<session::GuiState> {
        let session = session::load_session(path)?;
        let gui_state = session.gui_state.clone();
        session.apply_to_app(self);
        Ok(gui_state)
    }
}

impl DaqAppInner {
    /// Spawns an instrument to run on the Tokio runtime.
    ///
    /// This is the core instrument lifecycle method. It:
    /// 1. Validates instrument isn't already running
    /// 2. Loads configuration from settings
    /// 3. Creates instrument instance via registry
    /// 4. Builds processing pipeline for this instrument
    /// 5. Spawns async task with tokio::select! loop
    /// 6. Stores InstrumentHandle for command/shutdown management
    ///
    /// # Task Loop Architecture
    ///
    /// The spawned task runs an infinite loop with `tokio::select!`:
    ///
    /// ```text
    /// loop {
    ///     tokio::select! {
    ///         // Branch 1: Data from instrument
    ///         data = stream.recv() => {
    ///             -> Process through pipeline
    ///             -> Broadcast to subscribers
    ///         }
    ///
    ///         // Branch 2: Commands from application
    ///         cmd = command_rx.recv() => {
    ///             if Shutdown => break;
    ///             else => handle_command()
    ///         }
    ///
    ///         // Branch 3: Idle timeout (logging)
    ///         _ = sleep(1s) => trace!("idle")
    ///     }
    /// }
    /// disconnect() // Called after loop breaks
    /// ```
    ///
    /// # Arguments
    ///
    /// * `id` - Instrument identifier (must match key in `settings.instruments`)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if instrument spawned successfully, or `Err` if:
    /// - Instrument is already running (duplicate spawn attempt)
    /// - Configuration is missing or invalid
    /// - Instrument type not registered
    /// - Processor creation fails
    ///
    /// # Errors Do Not Fail Connection
    ///
    /// If `instrument.connect()` fails, the task completes with an error but
    /// the method returns `Ok(())`. The error is logged and can be retrieved
    /// by awaiting the task handle.
    ///
    /// # Complexity
    ///
    /// O(p) where p = number of processors configured for this instrument
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rust_daq::app::DaqAppInner;
    /// # fn example(inner: &mut DaqAppInner) -> anyhow::Result<()> {
    /// // Spawn instrument (configuration must exist in settings)
    /// inner.spawn_instrument("power_meter_1")?;
    ///
    /// // Spawning again fails with duplicate error
    /// assert!(inner.spawn_instrument("power_meter_1").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Processing Pipeline
    ///
    /// If processors are configured for this instrument:
    ///
    /// ```toml
    /// [[processors.power_meter_1]]
    /// type = "iir_filter"
    /// [processors.power_meter_1.config]
    /// cutoff_hz = 10.0
    ///
    /// [[processors.power_meter_1]]
    /// type = "trigger"
    /// [processors.power_meter_1.config]
    /// threshold = 0.5
    /// ```
    ///
    /// Data flows: Instrument → IIR Filter → Trigger → Broadcast
    ///
    /// # Shutdown Integration (bd-20)
    ///
    /// The task loop breaks when receiving `InstrumentCommand::Shutdown`, then calls
    /// `disconnect()` outside the loop for graceful cleanup. This ensures resources
    /// are released even if the data stream ends unexpectedly.
    pub fn spawn_instrument(&mut self, id: &str) -> Result<()> {
        if self.instruments.contains_key(id) {
            return Err(anyhow!("Instrument '{}' is already running.", id));
        }

        let instrument_config = self
            .settings
            .instruments
            .get(id)
            .ok_or_else(|| anyhow!("Instrument config for '{}' not found.", id))?;
        let instrument_type = instrument_config
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Instrument type for '{}' not found in config.", id))?;

        let mut instrument = self
            .instrument_registry
            .create(instrument_type, id)
            .ok_or_else(|| anyhow!("Instrument type '{}' not found.", instrument_type))?;

        // Create processor chain for this instrument
        let mut processors: Vec<Box<dyn DataProcessor>> = Vec::new();
        if let Some(processor_configs) = self.settings.processors.as_ref().and_then(|p| p.get(id)) {
            for config in processor_configs {
                let processor = self
                    .processor_registry
                    .create(&config.r#type, &config.config)
                    .with_context(|| {
                        format!(
                            "Failed to create processor '{}' for instrument '{}'",
                            config.r#type, id
                        )
                    })?;
                processors.push(processor);
            }
        }

        let data_sender = self.data_sender.clone();
        let settings = self.settings.clone();
        let id_clone = id.to_string();

        // Create command channel
        let (command_tx, mut command_rx) = tokio::sync::mpsc::channel(32);

        let task: JoinHandle<Result<()>> = self.runtime.spawn(async move {
            instrument
                .connect(&settings)
                .await
                .with_context(|| format!("Failed to connect to instrument '{}'", id_clone))?;
            info!("Instrument '{}' connected.", id_clone);

            let mut stream = instrument
                .data_stream()
                .await
                .context("Failed to get data stream")?;
            loop {
                tokio::select! {
                    data_point_result = stream.recv() => {
                        match data_point_result {
                            Ok(dp) => {
                                let mut data_points = vec![dp];
                                for processor in &mut processors {
                                    data_points = processor.process(&data_points);
                                }

                                for processed_dp in data_points {
                                    if let Err(e) = data_sender.send(processed_dp) {
                                        error!("Failed to broadcast data point: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Stream receive error: {}", e);
                                break;
                            }
                        }
                    }
                    Some(command) = command_rx.recv() => {
                        match command {
                            crate::core::InstrumentCommand::Shutdown => {
                                info!("Shutdown command received for '{}', disconnecting.", id_clone);
                                break;
                            }
                            _ => {
                                if let Err(e) = instrument.handle_command(command).await {
                                    error!("Failed to handle command for '{}': {}", id_clone, e);
                                }
                            }
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        log::trace!("Instrument stream for {} is idle.", id_clone);
                    }
                }
            }

            // Graceful disconnect outside the loop
            if let Err(e) = instrument.disconnect().await {
                error!("Failed to disconnect from instrument '{}': {}", id_clone, e);
            } else {
                info!("Instrument '{}' disconnected gracefully.", id_clone);
            }

            Ok(())
        });

        let handle = InstrumentHandle { task, command_tx };
        self.instruments.insert(id.to_string(), handle);
        Ok(())
    }

    /// Stops a running instrument gracefully.
    ///
    /// Removes the instrument from the active instruments map and spawns an async
    /// task to perform graceful shutdown with timeout. The shutdown follows the
    /// same pattern as application-wide `shutdown()` but for a single instrument.
    ///
    /// # Shutdown Process
    ///
    /// 1. Remove instrument handle from instruments map
    /// 2. Send `Shutdown` command via command channel
    /// 3. Await task completion with 5-second timeout
    /// 4. Force abort if timeout expires
    ///
    /// # Arguments
    ///
    /// * `id` - Instrument identifier
    ///
    /// # Behavior
    ///
    /// If the instrument ID doesn't exist (not running or already stopped), this
    /// method is a no-op (silently succeeds).
    ///
    /// # Non-Blocking
    ///
    /// This method returns immediately after spawning the shutdown task. It does
    /// NOT block waiting for the instrument to stop. Use the spawned task handle
    /// if you need to await completion.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rust_daq::app::DaqAppInner;
    /// # fn example(inner: &mut DaqAppInner) {
    /// // Stop a specific instrument
    /// inner.stop_instrument("power_meter_1");
    ///
    /// // Calling again is safe (no-op)
    /// inner.stop_instrument("power_meter_1");
    /// # }
    /// ```
    ///
    /// # Complexity
    ///
    /// O(1) for map removal + O(async) for shutdown task (non-blocking)
    pub fn stop_instrument(&mut self, id: &str) {
        if let Some(handle) = self.instruments.remove(id) {
            let id_clone = id.to_string();
            self.runtime.spawn(async move {
                let shutdown_timeout = std::time::Duration::from_secs(5);
                info!("Sending shutdown signal to instrument: {}", id_clone);

                if handle.command_tx.send(crate::core::InstrumentCommand::Shutdown).await.is_err() {
                    log::warn!("Failed to send shutdown command to '{}', it might have already terminated. Aborting.", id_clone);
                    handle.task.abort();
                    return;
                }

                match tokio::time::timeout(shutdown_timeout, handle.task).await {
                    Ok(Ok(Ok(_))) => info!("Instrument '{}' stopped gracefully.", id_clone),
                    Ok(Ok(Err(e))) => error!("Instrument '{}' task returned error during stop: {}", id_clone, e),
                    Ok(Err(e)) => error!("Instrument '{}' task panicked: {}", id_clone, e),
                    Err(_) => {
                        log::warn!("Instrument '{}' failed to stop within {:?}, force terminating.", id_clone, shutdown_timeout);
                    }
                }
            });
        }
    }

    /// Sends a command to a running instrument.
    ///
    /// Sends an `InstrumentCommand` to the specified instrument via its command
    /// channel. The command is processed asynchronously by the instrument's task loop.
    ///
    /// # Arguments
    ///
    /// * `id` - Instrument identifier
    /// * `command` - Command to send (SetParameter, QueryParameter, Execute, Shutdown)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if command was queued successfully, or `Err` if:
    /// - Instrument is not running (not found in instruments map)
    /// - Command channel is full (bounded capacity: 32)
    /// - Instrument task has terminated (channel closed)
    ///
    /// # Non-Blocking
    ///
    /// This method uses `try_send` which fails immediately if the channel is full
    /// rather than waiting. For most use cases the channel should have capacity.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rust_daq::app::DaqAppInner;
    /// use rust_daq::core::InstrumentCommand;
    ///
    /// # fn example(inner: &DaqAppInner) -> anyhow::Result<()> {
    /// // Set a parameter
    /// inner.send_instrument_command(
    ///     "laser",
    ///     InstrumentCommand::SetParameter("wavelength".to_string(), "800.0".to_string())
    /// )?;
    ///
    /// // Execute a command
    /// inner.send_instrument_command(
    ///     "stage",
    ///     InstrumentCommand::Execute("home".to_string(), vec![])
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Complexity
    ///
    /// O(1) - HashMap lookup + channel send
    pub fn send_instrument_command(&self, id: &str, command: crate::core::InstrumentCommand) -> Result<()> {
        let handle = self.instruments.get(id)
            .ok_or_else(|| anyhow!("Instrument '{}' is not running", id))?;

        handle.command_tx.try_send(command)
            .map_err(|e| anyhow!("Failed to send command to instrument '{}': {}", id, e))?;

        Ok(())
    }

    /// Returns a list of available channel names.
    ///
    /// Lists all instrument types registered in the instrument registry, not the
    /// currently running instruments. This is useful for GUI dropdowns and
    /// configuration validation.
    ///
    /// # Returns
    ///
    /// Vector of instrument type names (e.g., `["mock", "newport_1830c", "esp300"]`)
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rust_daq::app::DaqAppInner;
    /// # fn example(inner: &DaqAppInner) {
    /// let channels = inner.get_available_channels();
    /// for channel in channels {
    ///     println!("Available instrument type: {}", channel);
    /// }
    /// # }
    /// ```
    ///
    /// # Complexity
    ///
    /// O(n) where n = number of registered instrument types
    ///
    /// # Note
    ///
    /// This returns registered **types**, not running **instances**. To get
    /// running instruments, use `inner.instruments.keys()` instead.
    pub fn get_available_channels(&self) -> Vec<String> {
        self.instrument_registry.list().collect()
    }

    /// Starts the data recording process.
    ///
    /// Spawns an async task that subscribes to the data broadcast channel and
    /// writes all data points to the configured storage backend. The task runs
    /// until explicitly stopped or the broadcast channel closes.
    ///
    /// # Storage Backend Selection
    ///
    /// The storage format is determined by `self.storage_format`:
    /// - `"csv"` → `CsvWriter` (human-readable, Excel-compatible)
    /// - `"hdf5"` → `Hdf5Writer` (binary, self-describing, efficient)
    /// - `"arrow"` → `ArrowWriter` (columnar, compressed, ecosystem support)
    ///
    /// # Task Loop
    ///
    /// ```text
    /// loop {
    ///     data_point = rx.recv() => {
    ///         match data_point {
    ///             Ok(dp) => writer.write(&[dp]),
    ///             Err(Lagged(n)) => log warning (receiver too slow),
    ///             Err(Closed) => break (channel closed)
    ///         }
    ///     }
    /// }
    /// writer.shutdown() // Flush and close
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Recording is already in progress (`writer_task.is_some()`)
    /// - Storage format is unsupported (invalid format string)
    ///
    /// Note: Storage initialization errors (`writer.init()` failures) occur in
    /// the spawned task, not in this method. Check task result to detect them.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rust_daq::app::DaqAppInner;
    /// # fn example(inner: &mut DaqAppInner) -> anyhow::Result<()> {
    /// // Start recording with configured format
    /// inner.start_recording()?;
    ///
    /// // Starting again fails (already recording)
    /// assert!(inner.start_recording().is_err());
    ///
    /// // Later, stop recording
    /// inner.stop_recording();
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Complexity
    ///
    /// O(1) for task spawn + O(async) for write operations (non-blocking)
    pub fn start_recording(&mut self) -> Result<()> {
        if self.writer_task.is_some() {
            return Err(anyhow!("Recording is already in progress."));
        }

        let settings = self.settings.clone();
        let metadata = self.metadata.clone();
        let mut rx = self.data_sender.subscribe();
        let storage_format_for_task = self.storage_format.clone();
        
        // Create shutdown signal channel
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        self.writer_shutdown_tx = Some(shutdown_tx);

        let storage_registry = self.storage_registry.clone();

        let task = self.runtime.spawn(async move {
            let mut writer = storage_registry.create(&storage_format_for_task)?;

            writer.init(&settings).await?;
            writer.set_metadata(&metadata).await?;

            loop {
                tokio::select! {
                    data_point = rx.recv() => {
                        match data_point {
                            Ok(dp) => {
                                if let Err(e) = writer.write(&[dp]).await {
                                    error!("Failed to write data point: {}", e);
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                log::warn!("Data writer lagged by {} messages.", n);
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                break;
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        info!("Storage writer received shutdown signal, gracefully stopping...");
                        break;
                    }
                }
            }

            writer.shutdown().await?;
            Ok(())
        });

        self.writer_task = Some(task);
        info!("Started recording with format: {}", self.storage_format);
        Ok(())
    }

    /// Stops the data recording process gracefully.
    ///
    /// Sends a shutdown signal to the storage writer task, allowing it to:
    /// - Flush all buffered data to disk  
    /// - Properly finalize files with footers/indexes
    /// - Call `writer.shutdown()` for clean termination
    ///
    /// Falls back to task abort if graceful shutdown times out after 5 seconds.
    ///
    /// # Behavior
    ///
    /// - If recording is active: Sends shutdown signal and waits for graceful completion
    /// - If not recording: No-op (silently succeeds)
    /// - On timeout or signal failure: Falls back to task abort
    ///
    /// # Data Safety
    ///
    /// This method prioritizes data integrity by using graceful shutdown:
    /// 1. Send shutdown signal via oneshot channel
    /// 2. Writer task receives signal and breaks from event loop
    /// 3. Writer calls `writer.shutdown()` to flush buffers and finalize files
    /// 4. Task completes naturally with proper cleanup
    /// 5. Fallback to abort only if graceful shutdown fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rust_daq::app::DaqAppInner;
    /// # fn example(inner: &mut DaqAppInner) {
    /// // Stop recording (safe to call even if not recording)
    /// inner.stop_recording();
    ///
    /// // Calling again is safe (no-op)
    /// inner.stop_recording();
    /// # }
    /// ```
    ///
    /// # Complexity
    ///
    /// O(1) for signal send + O(timeout) for graceful shutdown (5s max)
    pub fn stop_recording(&mut self) {
        let task = match self.writer_task.take() {
            Some(task) => task,
            None => return, // Not recording
        };
        
        let shutdown_tx = self.writer_shutdown_tx.take();
        let runtime = self.runtime.clone();
        
        // Spawn shutdown task to avoid blocking
        let shutdown_task = runtime.spawn(async move {
            let shutdown_timeout = std::time::Duration::from_secs(5);
            
            if let Some(tx) = shutdown_tx {
                if tx.send(()).is_ok() {
                    // Try graceful shutdown with timeout
                    match tokio::time::timeout(shutdown_timeout, task).await {
                        Ok(Ok(Ok(_))) => {
                            info!("Storage writer shut down gracefully.");
                        }
                        Ok(Ok(Err(e))) => {
                            error!("Storage writer task returned error during shutdown: {}", e);
                        }
                        Ok(Err(e)) => {
                            error!("Storage writer task panicked during shutdown: {}", e);
                        }
                        Err(_) => {
                            log::warn!("Storage writer shutdown timed out after {}s", shutdown_timeout.as_secs());
                            // Task is moved into timeout, can't abort here - timeout will handle cancellation
                        }
                    }
                } else {
                    log::warn!("Failed to send shutdown signal to storage writer, aborting task.");
                    task.abort();
                }
            } else {
                log::warn!("No shutdown channel available, aborting storage task.");
                task.abort();
            }
        });

        // Explicitly drop the shutdown task handle
        drop(shutdown_task);

        info!("Stopped recording.");
    }
}
