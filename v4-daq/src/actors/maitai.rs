//! MaiTai Tunable Laser Actor
//!
//! Kameo actor implementation for Spectra Physics MaiTai Ti:Sapphire laser.
//! Supports wavelength tuning (700-1000 nm), power monitoring, and shutter control.

use crate::hardware::{SerialAdapterV4, SerialAdapterV4Builder};
use crate::traits::tunable_laser::{LaserMeasurement, ShutterState, TunableLaser, Wavelength};
use anyhow::{anyhow, Context as AnyhowContext, Result};
use kameo::actor::{ActorRef, WeakActorRef};
use kameo::error::BoxSendError;
use kameo::message::{Context, Message};
use std::any::Any;
use std::time::{SystemTime, UNIX_EPOCH};

/// MaiTai actor state
pub struct MaiTai {
    /// Current wavelength setting
    wavelength: Wavelength,
    /// Current shutter state
    shutter: ShutterState,
    /// Hardware adapter for serial communication
    adapter: Option<SerialAdapterV4>,
}

impl MaiTai {
    /// Create new MaiTai actor with mock adapter (for testing)
    ///
    /// # Example
    /// ```no_run
    /// use v4_daq::actors::MaiTai;
    /// use kameo::Actor;
    ///
    /// let actor_ref = MaiTai::spawn(MaiTai::new());
    /// ```
    pub fn new() -> Self {
        Self {
            wavelength: Wavelength { nm: 800.0 }, // Default Ti:Sapphire wavelength
            shutter: ShutterState::Closed,
            adapter: None,
        }
    }

    /// Create new MaiTai actor with real hardware
    ///
    /// # Arguments
    /// * `port` - Serial port path (e.g., "/dev/ttyUSB5")
    /// * `baud_rate` - Communication speed (9600 for MaiTai)
    ///
    /// # Example
    /// ```no_run
    /// use v4_daq::actors::MaiTai;
    ///
    /// let laser = MaiTai::spawn(MaiTai::with_serial("/dev/ttyUSB5".to_string(), 9600));
    /// ```
    pub fn with_serial(port: String, baud_rate: u32) -> Self {
        // MaiTai requires software flow control (XON/XOFF) and '\r' line terminator
        // Use longer timeout (2s) as MaiTai can be slow to respond
        let adapter = SerialAdapterV4Builder::new(port, baud_rate)
            .with_line_terminator("\r".to_string())
            .with_response_delimiter('\r')
            .with_timeout(std::time::Duration::from_secs(2))
            .build();

        Self {
            wavelength: Wavelength { nm: 800.0 },
            shutter: ShutterState::Closed,
            adapter: Some(adapter),
        }
    }

    /// Configure the instrument after connection
    async fn configure_hardware(&self) -> Result<()> {
        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("No hardware adapter configured"))?;

        // Ensure connected
        if !adapter.is_connected().await {
            adapter.connect().await?;
        }

        // MaiTai requires 300ms initialization delay after connection
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        tracing::info!("MaiTai connected to serial port");

        // Initialize wavelength
        adapter
            .send_command_no_response(&format!("WAVELENGTH:{}", self.wavelength.nm))
            .await
            .context("Failed to set initial wavelength")?;

        tracing::info!("Set wavelength to {} nm", self.wavelength.nm);

        // Ensure shutter is closed on startup
        adapter
            .send_command_no_response("SHUTTER:0")
            .await
            .context("Failed to close shutter")?;

        tracing::info!("Shutter closed (safe startup)");

        Ok(())
    }

    /// Read wavelength from hardware
    async fn read_hardware_wavelength(&self) -> Result<f64> {
        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("No hardware adapter configured"))?;

        let response = adapter
            .send_command("WAVELENGTH?")
            .await
            .context("Failed to query wavelength")?;

        // MaiTai may echo the command, extract value after colon if present
        let value_str = response.split(':').last().unwrap_or(&response);

        let wavelength: f64 = value_str
            .trim()
            .parse()
            .with_context(|| format!("Failed to parse wavelength response: '{}'", response))?;

        Ok(wavelength)
    }

    /// Read power from hardware
    async fn read_hardware_power(&self) -> Result<f64> {
        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("No hardware adapter configured"))?;

        let response = adapter
            .send_command("POWER?")
            .await
            .context("Failed to query power")?;

        // MaiTai may echo the command, extract value after colon if present
        let value_str = response.split(':').last().unwrap_or(&response);

        let power: f64 = value_str
            .trim()
            .parse()
            .with_context(|| format!("Failed to parse power response: '{}'", response))?;

        Ok(power)
    }

    /// Read shutter state from hardware
    async fn read_hardware_shutter(&self) -> Result<ShutterState> {
        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("No hardware adapter configured"))?;

        let response = adapter
            .send_command("SHUTTER?")
            .await
            .context("Failed to query shutter")?;

        // MaiTai may echo the command, extract value after colon if present
        let value_str = response.split(':').last().unwrap_or(&response);

        let state: i32 = value_str
            .trim()
            .parse()
            .with_context(|| format!("Failed to parse shutter response: '{}'", response))?;

        Ok(match state {
            1 => ShutterState::Open,
            0 => ShutterState::Closed,
            _ => return Err(anyhow!("Invalid shutter state: {}", state)),
        })
    }
}

impl Default for MaiTai {
    fn default() -> Self {
        Self::new()
    }
}

// Kameo Actor implementation
impl kameo::Actor for MaiTai {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(
        args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        tracing::info!("MaiTai actor started");

        // Configure hardware if adapter is present
        if args.adapter.is_some() {
            if let Err(err) = args.configure_hardware().await {
                tracing::error!("Failed to configure hardware on start: {err}");
                let error_msg: Box<dyn Any + Send> =
                    Box::new(format!("Hardware configuration failed: {err}"));
                return Err(kameo::error::SendError::HandlerError(error_msg));
            }
        }

        Ok(args)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        tracing::info!("MaiTai actor stopping - closing shutter for safety");

        // Close shutter before shutdown
        if let Some(adapter) = &self.adapter {
            if adapter.is_connected().await {
                let _ = adapter.send_command_no_response("SHUTTER:0").await;
            }
        }

        Ok(())
    }
}

// Message: Set Wavelength
#[derive(Clone)]
pub struct SetWavelength {
    pub wavelength: Wavelength,
}

impl Message<SetWavelength> for MaiTai {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: SetWavelength,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let Some(adapter) = &self.adapter {
            adapter
                .send_command_no_response(&format!("WAVELENGTH:{}", msg.wavelength.nm))
                .await
                .context("Failed to set wavelength")?;
        }

        self.wavelength = msg.wavelength;
        tracing::debug!("Wavelength set to {} nm", msg.wavelength.nm);

        Ok(())
    }
}

// Message: Get Wavelength
#[derive(Clone)]
pub struct GetWavelength;

impl Message<GetWavelength> for MaiTai {
    type Reply = Result<Wavelength>;

    async fn handle(
        &mut self,
        _msg: GetWavelength,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let Some(_adapter) = &self.adapter {
            let nm = self.read_hardware_wavelength().await?;
            self.wavelength = Wavelength { nm };
        }

        Ok(self.wavelength)
    }
}

// Message: Read Power
#[derive(Clone)]
pub struct ReadPower;

impl Message<ReadPower> for MaiTai {
    type Reply = Result<f64>;

    async fn handle(
        &mut self,
        _msg: ReadPower,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let Some(_adapter) = &self.adapter {
            let power = self.read_hardware_power().await?;
            Ok(power)
        } else {
            Ok(0.0) // Mock: return zero power
        }
    }
}

// Message: Open Shutter
#[derive(Clone)]
pub struct OpenShutter;

impl Message<OpenShutter> for MaiTai {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        _msg: OpenShutter,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let Some(adapter) = &self.adapter {
            adapter
                .send_command_no_response("SHUTTER:1")
                .await
                .context("Failed to open shutter")?;
        }

        self.shutter = ShutterState::Open;
        tracing::debug!("Shutter opened");

        Ok(())
    }
}

// Message: Close Shutter
#[derive(Clone)]
pub struct CloseShutter;

impl Message<CloseShutter> for MaiTai {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        _msg: CloseShutter,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let Some(adapter) = &self.adapter {
            adapter
                .send_command_no_response("SHUTTER:0")
                .await
                .context("Failed to close shutter")?;
        }

        self.shutter = ShutterState::Closed;
        tracing::debug!("Shutter closed");

        Ok(())
    }
}

// Message: Get Shutter State
#[derive(Clone)]
pub struct GetShutterState;

impl Message<GetShutterState> for MaiTai {
    type Reply = Result<ShutterState>;

    async fn handle(
        &mut self,
        _msg: GetShutterState,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let Some(_adapter) = &self.adapter {
            let state = self.read_hardware_shutter().await?;
            self.shutter = state;
        }

        Ok(self.shutter)
    }
}

// Message: Take Measurement
#[derive(Clone)]
pub struct Measure;

impl Message<Measure> for MaiTai {
    type Reply = Result<LaserMeasurement>;

    async fn handle(
        &mut self,
        _msg: Measure,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let wavelength = if let Some(_adapter) = &self.adapter {
            let nm = self.read_hardware_wavelength().await?;
            Wavelength { nm }
        } else {
            self.wavelength
        };

        let power_watts = if let Some(_adapter) = &self.adapter {
            self.read_hardware_power().await?
        } else {
            0.0
        };

        let shutter = if let Some(_adapter) = &self.adapter {
            self.read_hardware_shutter().await?
        } else {
            self.shutter
        };

        Ok(LaserMeasurement {
            timestamp_ns,
            wavelength,
            power_watts,
            shutter,
        })
    }
}

// TunableLaser trait implementation for ActorRef
#[async_trait::async_trait]
impl TunableLaser for ActorRef<MaiTai> {
    async fn set_wavelength(&self, wavelength: Wavelength) -> Result<()> {
        use anyhow::Context as _;
        self.ask(SetWavelength { wavelength })
            .await
            .context("Failed to send SetWavelength message to actor")
    }

    async fn get_wavelength(&self) -> Result<Wavelength> {
        use anyhow::Context as _;
        self.ask(GetWavelength)
            .await
            .context("Failed to send GetWavelength message to actor")
    }

    async fn read_power(&self) -> Result<f64> {
        use anyhow::Context as _;
        self.ask(ReadPower)
            .await
            .context("Failed to send ReadPower message to actor")
    }

    async fn open_shutter(&self) -> Result<()> {
        use anyhow::Context as _;
        self.ask(OpenShutter)
            .await
            .context("Failed to send OpenShutter message to actor")
    }

    async fn close_shutter(&self) -> Result<()> {
        use anyhow::Context as _;
        self.ask(CloseShutter)
            .await
            .context("Failed to send CloseShutter message to actor")
    }

    async fn get_shutter_state(&self) -> Result<ShutterState> {
        use anyhow::Context as _;
        self.ask(GetShutterState)
            .await
            .context("Failed to send GetShutterState message to actor")
    }

    async fn measure(&self) -> Result<LaserMeasurement> {
        use anyhow::Context as _;
        self.ask(Measure)
            .await
            .context("Failed to send Measure message to actor")
    }

    fn to_arrow(&self, measurements: &[LaserMeasurement]) -> Result<arrow::record_batch::RecordBatch> {
        use arrow::array::{Float64Array, Int64Array, StringArray, TimestampNanosecondArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use once_cell::sync::Lazy;
        use std::sync::Arc;

        static SCHEMA: Lazy<Arc<Schema>> = Lazy::new(|| {
            Arc::new(Schema::new(vec![
                Field::new(
                    "timestamp",
                    DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                    false,
                ),
                Field::new("wavelength_nm", DataType::Float64, false),
                Field::new("power_watts", DataType::Float64, false),
                Field::new("shutter_state", DataType::Utf8, false),
                Field::new("shutter_open", DataType::Int64, false),
            ]))
        });

        let timestamps: Vec<i64> = measurements.iter().map(|m| m.timestamp_ns).collect();
        let wavelengths: Vec<f64> = measurements.iter().map(|m| m.wavelength.nm).collect();
        let powers: Vec<f64> = measurements.iter().map(|m| m.power_watts).collect();
        let shutter_states: StringArray = measurements
            .iter()
            .map(|m| {
                Some(match m.shutter {
                    ShutterState::Open => "open",
                    ShutterState::Closed => "closed",
                })
            })
            .collect();
        let shutter_open: Vec<i64> = measurements
            .iter()
            .map(|m| match m.shutter {
                ShutterState::Open => 1,
                ShutterState::Closed => 0,
            })
            .collect();

        let batch = RecordBatch::try_new(
            SCHEMA.clone(),
            vec![
                Arc::new(TimestampNanosecondArray::from(timestamps)),
                Arc::new(Float64Array::from(wavelengths)),
                Arc::new(Float64Array::from(powers)),
                Arc::new(shutter_states),
                Arc::new(Int64Array::from(shutter_open)),
            ],
        )?;

        Ok(batch)
    }
}
