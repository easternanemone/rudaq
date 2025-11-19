//! Newport 1830-C Optical Power Meter Actor
//!
//! Kameo actor implementation with supervision and fault tolerance.

use crate::hardware::SerialAdapterV4;
use crate::traits::power_meter::{PowerMeasurement, PowerMeter, PowerUnit, Wavelength};
use anyhow::{anyhow, Context as AnyhowContext, Result};
use kameo::actor::{ActorRef, WeakActorRef};
use kameo::error::{BoxSendError, SendError};
use kameo::message::{Context, Message};
use std::any::Any;
use std::time::{SystemTime, UNIX_EPOCH};

/// Newport 1830-C actor state
pub struct Newport1830C {
    /// Current wavelength setting
    wavelength: Wavelength,
    /// Current power unit
    unit: PowerUnit,
    /// Hardware adapter for serial communication
    adapter: Option<SerialAdapterV4>,
}

impl Newport1830C {
    /// Create new Newport 1830-C actor with mock adapter (for testing)
    pub fn new() -> Self {
        Self {
            wavelength: Wavelength { nm: 633.0 }, // Default HeNe wavelength
            unit: PowerUnit::Watts,
            adapter: None,
        }
    }

    /// Create new Newport 1830-C actor with real hardware
    ///
    /// # Arguments
    /// * `port` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `baud_rate` - Communication speed (typically 9600 for Newport 1830-C)
    ///
    /// # Example
    /// ```no_run
    /// use rust_daq::actors::Newport1830C;
    ///
    /// let actor = Newport1830C::with_serial("/dev/ttyUSB0".to_string(), 9600);
    /// ```
    pub fn with_serial(port: String, baud_rate: u32) -> Self {
        let adapter = SerialAdapterV4::new(port, baud_rate);

        Self {
            wavelength: Wavelength { nm: 633.0 },
            unit: PowerUnit::Watts,
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

        // Set wavelength
        adapter
            .send_command(&format!("PM:Lambda {}", self.wavelength.nm))
            .await
            .context("Failed to set wavelength")?;

        tracing::info!("Set wavelength to {} nm", self.wavelength.nm);

        // Set units
        let unit_code = match self.unit {
            PowerUnit::Watts => 0,
            PowerUnit::MilliWatts => 0, // Newport doesn't have mW, use W
            PowerUnit::MicroWatts => 0,
            PowerUnit::NanoWatts => 0,
            PowerUnit::Dbm => 1,
        };

        adapter
            .send_command(&format!("PM:Units {}", unit_code))
            .await
            .context("Failed to set units")?;

        tracing::info!("Set units to {:?}", self.unit);

        Ok(())
    }

    /// Read power from hardware
    async fn read_hardware_power(&self) -> Result<f64> {
        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("No hardware adapter configured"))?;

        let response = adapter
            .send_command("PM:Power?")
            .await
            .context("Failed to read power")?;

        response
            .trim()
            .parse::<f64>()
            .with_context(|| format!("Failed to parse power value: '{}'", response))
    }
}

impl Default for Newport1830C {
    fn default() -> Self {
        Self::new()
    }
}

impl kameo::Actor for Newport1830C {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(
        mut args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        tracing::info!("Newport 1830-C actor started");

        // Connect and configure hardware if adapter present
        if args.adapter.is_some() {
            if let Err(err) = args.configure_hardware().await {
                tracing::error!("Failed to configure hardware on start: {err}");
                let error_msg: Box<dyn Any + Send> =
                    Box::new(format!("Hardware configuration failed: {err}"));
                return Err(SendError::HandlerError(error_msg));
            }
        }

        Ok(args)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        tracing::info!("Newport 1830-C actor stopped");
        Ok(())
    }
}

/// Messages for Newport 1830-C actor
#[derive(Debug)]
pub struct ReadPower;

impl Message<ReadPower> for Newport1830C {
    type Reply = Result<PowerMeasurement>;

    async fn handle(
        &mut self,
        _msg: ReadPower,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time error")
            .as_nanos() as i64;

        let power = if self.adapter.is_some() {
            self.read_hardware_power().await? // Propagate error
        } else {
            return Err(anyhow!("No hardware adapter configured"));
        };

        Ok(PowerMeasurement {
            timestamp_ns,
            power,
            unit: self.unit,
            wavelength: Some(self.wavelength),
        })
    }
}

#[derive(Debug)]
pub struct SetWavelength(pub Wavelength);

impl Message<SetWavelength> for Newport1830C {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: SetWavelength,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.wavelength = msg.0;
        tracing::debug!("Set wavelength to {} nm", msg.0.nm);

        // Update hardware if connected
        if let Some(adapter) = &self.adapter {
            adapter
                .send_command(&format!("PM:Lambda {}", msg.0.nm))
                .await
                .context("Failed to set wavelength on hardware")?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct GetWavelength;

impl Message<GetWavelength> for Newport1830C {
    type Reply = Wavelength;

    async fn handle(
        &mut self,
        _msg: GetWavelength,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.wavelength
    }
}

#[derive(Debug)]
pub struct SetUnit(pub PowerUnit);

impl Message<SetUnit> for Newport1830C {
    type Reply = Result<()>;

    async fn handle(&mut self, msg: SetUnit, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        self.unit = msg.0;
        tracing::debug!("Set power unit to {:?}", msg.0);

        // Update hardware if connected
        if let Some(adapter) = &self.adapter {
            let unit_code = match msg.0 {
                PowerUnit::Watts
                | PowerUnit::MilliWatts
                | PowerUnit::MicroWatts
                | PowerUnit::NanoWatts => 0,
                PowerUnit::Dbm => 1,
            };

            adapter
                .send_command(&format!("PM:Units {}", unit_code))
                .await
                .context("Failed to set units on hardware")?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct GetUnit;

impl Message<GetUnit> for Newport1830C {
    type Reply = PowerUnit;

    async fn handle(
        &mut self,
        _msg: GetUnit,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.unit
    }
}

/// Implement PowerMeter trait for actor reference
#[async_trait::async_trait]
impl PowerMeter for ActorRef<Newport1830C> {
    async fn read_power(&self) -> Result<PowerMeasurement> {
        use anyhow::Context as _;
        self.ask(ReadPower)
            .await
            .context("Failed to send ReadPower message to actor")
    }

    async fn set_wavelength(&self, wavelength: Wavelength) -> Result<()> {
        use anyhow::Context as _;
        self.ask(SetWavelength(wavelength))
            .await
            .context("Failed to send SetWavelength message to actor")
    }

    async fn get_wavelength(&self) -> Result<Wavelength> {
        use anyhow::Context as _;
        self.ask(GetWavelength)
            .await
            .context("Failed to send message to actor")
    }

    async fn set_unit(&self, unit: PowerUnit) -> Result<()> {
        use anyhow::Context as _;
        self.ask(SetUnit(unit))
            .await
            .context("Failed to send SetUnit message to actor")
    }

    async fn get_unit(&self) -> Result<PowerUnit> {
        use anyhow::Context as _;
        self.ask(GetUnit)
            .await
            .context("Failed to send message to actor")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kameo::actor::spawn;

    #[tokio::test]
    async fn test_newport_actor_lifecycle() {
        let actor_ref = spawn(Newport1830C::new());

        // Test reading power
        let measurement = actor_ref.read_power().await.unwrap();
        assert_eq!(measurement.unit, PowerUnit::Watts);

        // Test setting wavelength
        let new_wavelength = Wavelength { nm: 780.0 };
        actor_ref
            .clone()
            .set_wavelength(new_wavelength)
            .await
            .unwrap();
        let retrieved = actor_ref.get_wavelength().await.unwrap();
        assert_eq!(retrieved.nm, 780.0);

        // Shutdown
        actor_ref.kill().await;
    }

    #[tokio::test]
    async fn test_power_unit_setting() {
        let actor_ref = spawn(Newport1830C::new());

        // Test setting power unit
        actor_ref
            .clone()
            .set_unit(PowerUnit::MilliWatts)
            .await
            .unwrap();
        let unit = actor_ref.get_unit().await.unwrap();
        assert_eq!(unit, PowerUnit::MilliWatts);

        // Shutdown
        actor_ref.kill().await;
    }

    #[tokio::test]
    async fn test_multiple_measurements() {
        let actor_ref = spawn(Newport1830C::new());

        // Take multiple measurements
        for _ in 0..5 {
            let measurement = actor_ref.read_power().await.unwrap();
            assert!(measurement.timestamp_ns > 0);
            assert!(measurement.power > 0.0);
            assert_eq!(measurement.unit, PowerUnit::Watts);
        }

        // Shutdown
        actor_ref.kill().await;
    }
}
