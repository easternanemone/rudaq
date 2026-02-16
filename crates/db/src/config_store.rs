//! CRUD operations for hardware configuration records.
//!
//! This module stores and retrieves device configuration data in SurrealDB.
//! Types here are DB-native (no dependency on `hardware` crate) to avoid
//! circular dependencies — conversion to/from `DeviceConfig` happens in `bin`.

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::Result;
use crate::DaqDb;

// ---------------------------------------------------------------------------
// DB-native types
// ---------------------------------------------------------------------------

/// A driver definition stored in SurrealDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbDriver {
    /// Driver type identifier (e.g., "ell14", "pvcam", "mock").
    pub driver_type: String,
    /// Human-readable name.
    pub name: String,
    /// Capability strings (e.g., `["movable", "readable"]`).
    pub capabilities: Vec<String>,
}

/// An instrument (device instance) stored in SurrealDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbInstrument {
    /// Unique device ID (e.g., "rotator_2").
    pub device_id: String,
    /// Human-readable name (e.g., "ELL14 Rotator (Address 2)").
    pub name: String,
    /// Driver type that created this device.
    pub driver_type: String,
    /// Driver-specific configuration as JSON.
    /// This is the driver table minus the `type` key.
    pub config: serde_json::Value,
    /// Whether this device is active.
    pub enabled: bool,
}

/// Summary of a config import operation.
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub drivers_upserted: usize,
    pub instruments_upserted: usize,
    pub errors: Vec<String>,
}

/// Summary row returned by list operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentSummary {
    pub device_id: String,
    pub name: String,
    pub driver_type: String,
    pub enabled: bool,
}

impl DaqDb {
    // -------------------------------------------------------------------
    // Instruments
    // -------------------------------------------------------------------

    /// Upsert a batch of instruments into the database.
    ///
    /// Uses `device_id` as the unique key — existing records are updated,
    /// new records are created.
    pub async fn upsert_instruments(&self, instruments: &[DbInstrument]) -> Result<ImportReport> {
        let mut report = ImportReport::default();

        for inst in instruments {
            // Native UPSERT: emits a single Update notification via LIVE SELECT
            // (instead of Delete+Create from the old DELETE+CREATE pattern),
            // preserves record IDs, and is slightly more efficient.
            let result = self
                .client()
                .query(
                    "UPSERT instrument SET \
                     device_id = $device_id, \
                     name = $name, \
                     driver_type = $driver_type, \
                     config = $config, \
                     enabled = $enabled \
                     WHERE device_id = $device_id",
                )
                .bind(("device_id", inst.device_id.clone()))
                .bind(("name", inst.name.clone()))
                .bind(("driver_type", inst.driver_type.clone()))
                .bind(("config", inst.config.clone()))
                .bind(("enabled", inst.enabled))
                .await;

            match result {
                Ok(_) => report.instruments_upserted += 1,
                Err(e) => report
                    .errors
                    .push(format!("instrument '{}': {e}", inst.device_id)),
            }
        }

        info!(
            upserted = report.instruments_upserted,
            errors = report.errors.len(),
            "instrument import complete"
        );
        Ok(report)
    }

    /// Retrieve all instruments from the database.
    pub async fn get_all_instruments(&self) -> Result<Vec<DbInstrument>> {
        let mut response = self
            .client()
            .query(
                "SELECT device_id, name, driver_type, config, enabled \
                 FROM instrument ORDER BY device_id",
            )
            .await?;
        let rows: Vec<DbInstrument> = response.take(0)?;
        Ok(rows)
    }

    /// Retrieve a single instrument by device ID.
    pub async fn get_instrument(&self, device_id: &str) -> Result<Option<DbInstrument>> {
        let mut response = self
            .client()
            .query(
                "SELECT device_id, name, driver_type, config, enabled \
                 FROM instrument WHERE device_id = $device_id",
            )
            .bind(("device_id", device_id.to_owned()))
            .await?;
        let rows: Vec<DbInstrument> = response.take(0)?;
        Ok(rows.into_iter().next())
    }

    /// List instruments (lightweight summary without full config).
    pub async fn list_instruments(&self) -> Result<Vec<InstrumentSummary>> {
        let mut response = self
            .client()
            .query(
                "SELECT device_id, name, driver_type, enabled \
                 FROM instrument ORDER BY device_id",
            )
            .await?;
        let rows: Vec<InstrumentSummary> = response.take(0)?;
        Ok(rows)
    }

    /// Delete an instrument by device ID. Returns true if it existed.
    pub async fn delete_instrument(&self, device_id: &str) -> Result<bool> {
        // Single atomic query — avoids TOCTOU race between checking
        // existence and deleting.  RETURN BEFORE gives us the pre-delete
        // rows so we know whether anything was actually removed.
        let mut response = self
            .client()
            .query("DELETE FROM instrument WHERE device_id = $device_id RETURN BEFORE")
            .bind(("device_id", device_id.to_owned()))
            .await?;
        let deleted: Vec<DbInstrument> = response.take(0)?;
        Ok(!deleted.is_empty())
    }

    // -------------------------------------------------------------------
    // Drivers
    // -------------------------------------------------------------------

    /// Upsert a batch of drivers into the database.
    pub async fn upsert_drivers(&self, drivers: &[DbDriver]) -> Result<usize> {
        let mut count = 0;
        for drv in drivers {
            // Native UPSERT preserves record IDs and emits single LIVE SELECT
            // notification instead of Delete+Create pair.
            self.client()
                .query(
                    "UPSERT driver SET \
                     driver_type = $driver_type, \
                     name = $name, \
                     capabilities = $capabilities \
                     WHERE driver_type = $driver_type",
                )
                .bind(("driver_type", drv.driver_type.clone()))
                .bind(("name", drv.name.clone()))
                .bind(("capabilities", drv.capabilities.clone()))
                .await?;
            count += 1;
        }
        info!(count, "driver upsert complete");
        Ok(count)
    }

    /// Subscribe to live changes on the instrument table.
    ///
    /// Returns a stream of [`surrealdb::Notification<DbInstrument>`] for
    /// create, update, and delete events.  The caller should use
    /// `futures::StreamExt::next()` to iterate.
    ///
    /// This powers the watch-based reconciler (Phase 3b2) — LIVE SELECT
    /// events trigger `reconcile_once()` with debouncing.
    pub async fn live_instruments(
        &self,
    ) -> Result<
        impl futures::Stream<
            Item = std::result::Result<surrealdb::Notification<DbInstrument>, surrealdb::Error>,
        >,
    > {
        let stream = self.client().select("instrument").live().await?;
        Ok(stream)
    }

    /// Retrieve all drivers from the database.
    pub async fn get_all_drivers(&self) -> Result<Vec<DbDriver>> {
        let mut response = self
            .client()
            .query("SELECT driver_type, name, capabilities FROM driver ORDER BY driver_type")
            .await?;
        let rows: Vec<DbDriver> = response.take(0)?;
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// Conversion utilities (TOML <-> JSON for driver config)
// ---------------------------------------------------------------------------

/// Convert a `toml::Value` to `serde_json::Value`.
///
/// Used when importing TOML hardware configs into the JSON-native SurrealDB.
pub fn toml_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::json!(i),
        toml::Value::Float(f) => serde_json::json!(f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        toml::Value::Array(a) => serde_json::Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => {
            let map: serde_json::Map<String, serde_json::Value> = t
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

/// Convert a `serde_json::Value` to `toml::Value`.
///
/// Used when exporting from SurrealDB back to TOML format.
/// JSON `null` is mapped to an empty string since TOML has no null type.
pub fn json_to_toml(v: &serde_json::Value) -> toml::Value {
    match v {
        serde_json::Value::Null => toml::Value::String(String::new()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(a) => toml::Value::Array(a.iter().map(json_to_toml).collect()),
        serde_json::Value::Object(o) => {
            let mut map = toml::map::Map::new();
            for (k, v) in o {
                map.insert(k.clone(), json_to_toml(v));
            }
            toml::Value::Table(map)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "kv-mem")]
mod tests {
    use super::*;
    use crate::DbConfig;

    fn sample_instruments() -> Vec<DbInstrument> {
        vec![
            DbInstrument {
                device_id: "rotator_2".into(),
                name: "ELL14 Rotator (Address 2)".into(),
                driver_type: "ell14".into(),
                config: serde_json::json!({
                    "port": "/dev/serial/by-id/usb-FTDI-port0",
                    "address": "2"
                }),
                enabled: true,
            },
            DbInstrument {
                device_id: "power_meter".into(),
                name: "Newport 1830-C Power Meter".into(),
                driver_type: "newport1830_c".into(),
                config: serde_json::json!({
                    "port": "/dev/ttyS0"
                }),
                enabled: true,
            },
        ]
    }

    fn sample_drivers() -> Vec<DbDriver> {
        vec![
            DbDriver {
                driver_type: "ell14".into(),
                name: "Thorlabs ELL14".into(),
                capabilities: vec!["movable".into()],
            },
            DbDriver {
                driver_type: "newport1830_c".into(),
                name: "Newport 1830-C".into(),
                capabilities: vec!["readable".into()],
            },
        ]
    }

    #[tokio::test]
    async fn test_upsert_and_get_instruments() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let instruments = sample_instruments();
        let report = db.upsert_instruments(&instruments).await.unwrap();
        assert_eq!(report.instruments_upserted, 2);
        assert!(report.errors.is_empty());

        let all = db.get_all_instruments().await.unwrap();
        assert_eq!(all.len(), 2);

        let rotator = db.get_instrument("rotator_2").await.unwrap().unwrap();
        assert_eq!(rotator.name, "ELL14 Rotator (Address 2)");
        assert_eq!(rotator.driver_type, "ell14");
        assert_eq!(rotator.config["address"], "2");
    }

    #[tokio::test]
    async fn test_upsert_is_idempotent() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let instruments = sample_instruments();
        db.upsert_instruments(&instruments).await.unwrap();
        db.upsert_instruments(&instruments).await.unwrap();

        let all = db.get_all_instruments().await.unwrap();
        assert_eq!(all.len(), 2, "upsert should not duplicate records");
    }

    #[tokio::test]
    async fn test_list_instruments() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        db.upsert_instruments(&sample_instruments()).await.unwrap();

        let list = db.list_instruments().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].device_id, "power_meter"); // sorted alphabetically
        assert_eq!(list[1].device_id, "rotator_2");
    }

    #[tokio::test]
    async fn test_delete_instrument() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        db.upsert_instruments(&sample_instruments()).await.unwrap();

        let deleted = db.delete_instrument("rotator_2").await.unwrap();
        assert!(deleted);

        let deleted_again = db.delete_instrument("rotator_2").await.unwrap();
        assert!(!deleted_again);

        let remaining = db.get_all_instruments().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].device_id, "power_meter");
    }

    #[tokio::test]
    async fn test_upsert_and_get_drivers() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let count = db.upsert_drivers(&sample_drivers()).await.unwrap();
        assert_eq!(count, 2);

        let all = db.get_all_drivers().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].driver_type, "ell14");
        assert_eq!(all[0].capabilities, vec!["movable"]);
    }

    #[tokio::test]
    async fn test_toml_json_round_trip() {
        let toml_val: toml::Value = toml::from_str(
            r#"
            port = "/dev/ttyUSB0"
            baud_rate = 9600
            enabled = true
            channels = [1, 2, 3]
            "#,
        )
        .unwrap();

        let json_val = toml_to_json(&toml_val);
        assert_eq!(json_val["port"], "/dev/ttyUSB0");
        assert_eq!(json_val["baud_rate"], 9600);
        assert_eq!(json_val["enabled"], true);

        let roundtripped = json_to_toml(&json_val);
        assert_eq!(roundtripped, toml_val);
    }

    #[tokio::test]
    async fn test_upsert_with_changed_data() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let mut instruments = sample_instruments();
        db.upsert_instruments(&instruments).await.unwrap();

        // Change the name and config of rotator_2
        instruments[0].name = "Updated Rotator Name".into();
        instruments[0].config = serde_json::json!({"port": "/dev/ttyUSB1", "address": "3"});
        db.upsert_instruments(&instruments).await.unwrap();

        let rotator = db.get_instrument("rotator_2").await.unwrap().unwrap();
        assert_eq!(rotator.name, "Updated Rotator Name");
        assert_eq!(rotator.config["address"], "3");

        // Should still have only 2 instruments
        let all = db.get_all_instruments().await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_upsert_empty_batch() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let report = db.upsert_instruments(&[]).await.unwrap();
        assert_eq!(report.instruments_upserted, 0);
        assert!(report.errors.is_empty());
    }

    #[tokio::test]
    async fn test_get_nonexistent_instrument() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let result = db.get_instrument("does_not_exist").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let deleted = db.delete_instrument("does_not_exist").await.unwrap();
        assert!(!deleted);
    }

    #[tokio::test]
    async fn test_upsert_empty_device_id() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let instruments = vec![DbInstrument {
            device_id: String::new(),
            name: "Empty ID Device".into(),
            driver_type: "mock".into(),
            config: serde_json::json!({}),
            enabled: true,
        }];
        let report = db.upsert_instruments(&instruments).await.unwrap();
        assert_eq!(report.instruments_upserted, 1);
        // Should be retrievable with empty string key
        let found = db.get_instrument("").await.unwrap();
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn test_upsert_null_config() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let instruments = vec![DbInstrument {
            device_id: "null_config_device".into(),
            name: "Null Config".into(),
            driver_type: "mock".into(),
            config: serde_json::Value::Null,
            enabled: true,
        }];
        // SurrealDB accepts the null config on write but the record doesn't
        // round-trip through SELECT deserialization — get_instrument returns
        // None.  This documents the actual behavior: callers must supply a
        // valid JSON object for config, not null.
        let report = db.upsert_instruments(&instruments).await.unwrap();
        assert_eq!(
            report.instruments_upserted, 1,
            "write succeeds even with null config"
        );
        let found = db.get_instrument("null_config_device").await.unwrap();
        assert!(
            found.is_none(),
            "null config record is not retrievable — callers must use a JSON object"
        );
    }

    #[tokio::test]
    async fn test_upsert_duplicate_device_ids_in_batch() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let instruments = vec![
            DbInstrument {
                device_id: "dup".into(),
                name: "First".into(),
                driver_type: "mock".into(),
                config: serde_json::json!({"version": 1}),
                enabled: true,
            },
            DbInstrument {
                device_id: "dup".into(),
                name: "Second".into(),
                driver_type: "mock".into(),
                config: serde_json::json!({"version": 2}),
                enabled: false,
            },
        ];
        let report = db.upsert_instruments(&instruments).await.unwrap();
        assert_eq!(report.instruments_upserted, 2); // Both processed

        // Only the last one should remain (second DELETE+CREATE overwrites first)
        let all = db.get_all_instruments().await.unwrap();
        assert_eq!(all.len(), 1, "duplicates should be collapsed");
        assert_eq!(all[0].name, "Second");
        assert!(!all[0].enabled);
    }

    #[tokio::test]
    async fn test_driver_upsert_with_changed_capabilities() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let drivers = vec![DbDriver {
            driver_type: "mock".into(),
            name: "Mock Driver".into(),
            capabilities: vec!["readable".into()],
        }];
        db.upsert_drivers(&drivers).await.unwrap();

        // Update capabilities
        let drivers = vec![DbDriver {
            driver_type: "mock".into(),
            name: "Mock Driver v2".into(),
            capabilities: vec!["readable".into(), "movable".into(), "configurable".into()],
        }];
        db.upsert_drivers(&drivers).await.unwrap();

        let all = db.get_all_drivers().await.unwrap();
        assert_eq!(all.len(), 1, "should not duplicate driver");
        assert_eq!(all[0].name, "Mock Driver v2");
        assert_eq!(all[0].capabilities.len(), 3);
    }

    #[tokio::test]
    async fn test_concurrent_upsert_and_read() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let mut handles = vec![];

        // Spawn 10 tasks that each upsert a unique instrument
        for i in 0..10 {
            let db = db.clone();
            handles.push(tokio::spawn(async move {
                let instruments = vec![DbInstrument {
                    device_id: format!("concurrent_{i}"),
                    name: format!("Concurrent Device {i}"),
                    driver_type: "mock".into(),
                    config: serde_json::json!({"index": i}),
                    enabled: true,
                }];
                db.upsert_instruments(&instruments).await.unwrap();
                // Also read all instruments while writing
                let _all = db.get_all_instruments().await.unwrap();
            }));
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all 10 instruments exist
        let all = db.get_all_instruments().await.unwrap();
        assert_eq!(all.len(), 10, "all concurrent upserts should succeed");

        // Verify each one is present
        for i in 0..10 {
            let found = db.get_instrument(&format!("concurrent_{i}")).await.unwrap();
            assert!(found.is_some(), "instrument concurrent_{i} should exist");
        }
    }
}
