//! CRUD operations for hardware configuration records.
//!
//! This module stores and retrieves device configuration data in SurrealDB.
//! Types here are DB-native (no dependency on `hardware` crate) to avoid
//! circular dependencies — conversion to/from `DeviceConfig` happens in `bin`.

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::DaqDb;
use crate::error::Result;

// ---------------------------------------------------------------------------
// DB-native types
// ---------------------------------------------------------------------------

fn deserialize_vec_or_default<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Option::<Vec<String>>::deserialize(deserializer)?;
    Ok(values.unwrap_or_default())
}

/// A driver definition stored in SurrealDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbDriver {
    /// Driver type identifier (e.g., "ell14", "pvcam", "mock").
    pub driver_type: String,
    /// Human-readable name.
    pub name: String,
    /// Capability strings (e.g., `["movable", "readable"]`).
    #[serde(default, deserialize_with = "deserialize_vec_or_default")]
    pub capabilities: Vec<String>,
    /// Available command names (primarily for universal TOML drivers).
    #[serde(default, deserialize_with = "deserialize_vec_or_default")]
    pub commands: Vec<String>,
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

/// A device feature metadata record stored in SurrealDB.
///
/// Caches parameter metadata (type, ranges, enum values) discovered at
/// registration time. Does NOT store live values — only static metadata
/// for UI pre-rendering and offline feature queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbDeviceFeature {
    /// Device ID this feature belongs to (e.g., "andor_istar_0").
    pub device_id: String,
    /// Feature/parameter name (e.g., "ExposureTime", "mcp_gain").
    pub feature_name: String,
    /// Data type: "float", "int", "bool", "enum", "string", "command".
    pub feature_type: String,
    /// Whether this feature can be read.
    pub readable: bool,
    /// Whether this feature can be written.
    pub writable: bool,
    /// Minimum value for numeric features.
    #[serde(default)]
    pub min_value: Option<f64>,
    /// Maximum value for numeric features.
    #[serde(default)]
    pub max_value: Option<f64>,
    /// Step size for numeric features.
    #[serde(default)]
    pub step: Option<f64>,
    /// Allowed values for enum features.
    #[serde(default)]
    pub enum_values: Vec<String>,
    /// Physical unit (e.g., "s", "C", "ps").
    #[serde(default)]
    pub unit: Option<String>,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// UI grouping category (e.g., "Intensifier", "Acquisition").
    #[serde(default)]
    pub group_name: Option<String>,
}

/// A device parameter's persisted runtime state.
///
/// Stored in the `device_runtime_state` table (schema v7+) for restart
/// recovery and UI favorites. Keyed by `(device_id, param_name)`.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceParamState {
    /// Device ID this state belongs to (e.g., "stage_1").
    pub device_id: String,
    /// Parameter name (e.g., "position", "ExposureTime").
    pub param_name: String,
    /// Last-known parameter value as JSON.
    pub param_value: serde_json::Value,
    /// Whether this parameter is pinned to the UI quick-access section (bd-4wf7).
    #[serde(default)]
    pub is_favorite: bool,
}

/// A device lifecycle state transition event (bd-oqo7.9).
///
/// Stored in the `device_lifecycle_event` table for post-mortem analysis
/// of camera health during long experiments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceLifecycleEvent {
    /// Device ID that transitioned (e.g., "pvcam_0").
    pub device_id: String,
    /// State before the transition.
    pub from_state: String,
    /// State after the transition.
    pub to_state: String,
    /// Human-readable reason for the transition (e.g., "controller dead").
    pub reason: Option<String>,
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
                     capabilities = $capabilities, \
                     commands = $commands \
                     WHERE driver_type = $driver_type",
                )
                .bind(("driver_type", drv.driver_type.clone()))
                .bind(("name", drv.name.clone()))
                .bind(("capabilities", drv.capabilities.clone()))
                .bind(("commands", drv.commands.clone()))
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
        > + use<>,
    > {
        let stream = self.client().select("instrument").live().await?;
        Ok(stream)
    }

    /// Retrieve all drivers from the database.
    pub async fn get_all_drivers(&self) -> Result<Vec<DbDriver>> {
        let mut response = self
            .client()
            .query(
                "SELECT driver_type, name, capabilities, commands FROM driver ORDER BY driver_type",
            )
            .await?;
        let rows: Vec<DbDriver> = response.take(0)?;
        Ok(rows)
    }

    // -------------------------------------------------------------------
    // Device Features
    // -------------------------------------------------------------------

    /// Upsert device feature metadata in a single atomic transaction.
    ///
    /// Uses (device_id, feature_name) as the unique key — existing records
    /// are updated, new records are created. Batches all features into one
    /// SurrealQL `FOR` loop inside a transaction for atomicity and performance.
    pub async fn upsert_device_features(&self, features: &[DbDeviceFeature]) -> Result<usize> {
        if features.is_empty() {
            return Ok(0);
        }
        let count = features.len();
        self.client()
            .query(
                "BEGIN TRANSACTION; \
                 FOR $feat IN $features { \
                     UPSERT device_feature SET \
                         device_id = $feat.device_id, \
                         feature_name = $feat.feature_name, \
                         feature_type = $feat.feature_type, \
                         readable = $feat.readable, \
                         writable = $feat.writable, \
                         min_value = $feat.min_value, \
                         max_value = $feat.max_value, \
                         step = $feat.step, \
                         enum_values = $feat.enum_values, \
                         unit = $feat.unit, \
                         description = $feat.description, \
                         group_name = $feat.group_name, \
                         discovered_at = time::now() \
                     WHERE device_id = $feat.device_id \
                         AND feature_name = $feat.feature_name; \
                 }; \
                 COMMIT TRANSACTION;",
            )
            .bind(("features", features.to_vec()))
            .await?;
        info!(count, "device feature upsert complete");
        Ok(count)
    }

    /// Retrieve all feature metadata for a specific device.
    pub async fn get_device_features(&self, device_id: &str) -> Result<Vec<DbDeviceFeature>> {
        let mut response = self
            .client()
            .query(
                "SELECT device_id, feature_name, feature_type, readable, writable, \
                 min_value, max_value, step, enum_values, unit, description, group_name \
                 FROM device_feature WHERE device_id = $device_id \
                 ORDER BY feature_name",
            )
            .bind(("device_id", device_id.to_owned()))
            .await?;
        let rows: Vec<DbDeviceFeature> = response.take(0)?;
        Ok(rows)
    }

    /// Delete all feature metadata for a specific device.
    ///
    /// Called when a device is unregistered to clean up stale metadata.
    pub async fn delete_device_features(&self, device_id: &str) -> Result<usize> {
        let mut response = self
            .client()
            .query("DELETE FROM device_feature WHERE device_id = $device_id RETURN BEFORE")
            .bind(("device_id", device_id.to_owned()))
            .await?;
        let deleted: Vec<DbDeviceFeature> = response.take(0)?;
        let count = deleted.len();
        if count > 0 {
            info!(device_id, count, "deleted device features");
        }
        Ok(count)
    }

    // -------------------------------------------------------------------
    // Device Runtime State
    // -------------------------------------------------------------------

    /// Upsert a single device parameter's runtime state.
    ///
    /// Uses `(device_id, param_name)` as the unique key via the
    /// `idx_device_param` index. Existing records are updated with the new
    /// value and a fresh `updated_at` timestamp.
    pub async fn upsert_device_state(
        &self,
        device_id: &str,
        param_name: &str,
        param_value: &serde_json::Value,
    ) -> Result<()> {
        self.client()
            .query(
                "UPSERT device_runtime_state SET \
                 device_id = $device_id, \
                 param_name = $param_name, \
                 param_value = $param_value, \
                 updated_at = time::now() \
                 WHERE device_id = $device_id AND param_name = $param_name",
            )
            .bind(("device_id", device_id.to_owned()))
            .bind(("param_name", param_name.to_owned()))
            .bind(("param_value", param_value.clone()))
            .await?;
        Ok(())
    }

    /// Batch upsert multiple parameter states in a single transaction.
    ///
    /// Accepts a slice of `(device_id, param_name, param_value)` tuples.
    /// All upserts are wrapped in a transaction for atomicity — either all
    /// succeed or none are applied. Intended for debounced writes where
    /// multiple parameter changes are batched together.
    pub async fn batch_upsert_device_state(
        &self,
        states: &[(String, String, serde_json::Value)],
    ) -> Result<()> {
        if states.is_empty() {
            return Ok(());
        }

        // Serialize tuples into a JSON array that SurrealQL can iterate.
        let items: Vec<serde_json::Value> = states
            .iter()
            .map(|(device_id, param_name, param_value)| {
                serde_json::json!({
                    "device_id": device_id,
                    "param_name": param_name,
                    "param_value": param_value,
                })
            })
            .collect();

        self.client()
            .query(
                "BEGIN TRANSACTION; \
                 FOR $item IN $items { \
                     UPSERT device_runtime_state SET \
                         device_id = $item.device_id, \
                         param_name = $item.param_name, \
                         param_value = $item.param_value, \
                         updated_at = time::now() \
                     WHERE device_id = $item.device_id \
                         AND param_name = $item.param_name; \
                 }; \
                 COMMIT TRANSACTION;",
            )
            .bind(("items", items))
            .await?;

        info!(count = states.len(), "device state batch upsert complete");
        Ok(())
    }

    /// Retrieve all persisted runtime state for a device.
    ///
    /// Returns an empty `Vec` if the device has no persisted state.
    /// Results are ordered by `param_name` for deterministic output.
    ///
    /// **Note:** Selects specific fields rather than `SELECT *` to avoid
    /// SurrealDB `id` (Thing) and `updated_at` (Datetime) types that
    /// cannot deserialize into `serde_json::Value`.
    pub async fn get_device_state(&self, device_id: &str) -> Result<Vec<DeviceParamState>> {
        let mut response = self
            .client()
            .query(
                "SELECT device_id, param_name, param_value, is_favorite \
                 FROM device_runtime_state WHERE device_id = $device_id \
                 ORDER BY param_name",
            )
            .bind(("device_id", device_id.to_owned()))
            .await?;
        let rows: Vec<DeviceParamState> = response.take(0)?;
        Ok(rows)
    }

    /// Set the `is_favorite` flag for a specific parameter (bd-4wf7).
    ///
    /// Creates the record if it doesn't exist (with null param_value).
    pub async fn set_parameter_favorite(
        &self,
        device_id: &str,
        param_name: &str,
        is_favorite: bool,
    ) -> Result<()> {
        self.client()
            .query(
                "UPSERT device_runtime_state SET \
                 device_id = $device_id, \
                 param_name = $param_name, \
                 is_favorite = $is_favorite, \
                 updated_at = time::now() \
                 WHERE device_id = $device_id AND param_name = $param_name",
            )
            .bind(("device_id", device_id.to_owned()))
            .bind(("param_name", param_name.to_owned()))
            .bind(("is_favorite", is_favorite))
            .await?;
        Ok(())
    }

    /// Get all favorite parameters for a device (bd-4wf7).
    pub async fn get_favorites(&self, device_id: &str) -> Result<Vec<String>> {
        let mut response = self
            .client()
            .query(
                "SELECT param_name FROM device_runtime_state \
                 WHERE device_id = $device_id AND is_favorite = true \
                 ORDER BY param_name",
            )
            .bind(("device_id", device_id.to_owned()))
            .await?;
        #[derive(Deserialize)]
        struct Row {
            param_name: String,
        }
        let rows: Vec<Row> = response.take(0)?;
        Ok(rows.into_iter().map(|r| r.param_name).collect())
    }

    /// Delete all persisted runtime state for a device.
    ///
    /// Called when a device is unregistered to clean up stale state.
    pub async fn delete_device_state(&self, device_id: &str) -> Result<usize> {
        let mut response = self
            .client()
            .query("DELETE FROM device_runtime_state WHERE device_id = $device_id RETURN BEFORE")
            .bind(("device_id", device_id.to_owned()))
            .await?;
        let deleted: Vec<DeviceParamState> = response.take(0)?;
        let count = deleted.len();
        if count > 0 {
            info!(device_id, count, "deleted device runtime state");
        }
        Ok(count)
    }

    // -------------------------------------------------------------------
    // Device Lifecycle Events (bd-oqo7.9)
    // -------------------------------------------------------------------

    /// Record a device lifecycle state transition.
    ///
    /// Also updates the `_lifecycle_state` pseudo-parameter in
    /// `device_runtime_state` so the current state is queryable for
    /// restart recovery.
    pub async fn record_lifecycle_event(&self, event: &DeviceLifecycleEvent) -> Result<()> {
        self.client()
            .query(
                "CREATE device_lifecycle_event SET \
                 device_id = $device_id, \
                 from_state = $from_state, \
                 to_state = $to_state, \
                 reason = $reason, \
                 timestamp = time::now()",
            )
            .bind(("device_id", event.device_id.clone()))
            .bind(("from_state", event.from_state.clone()))
            .bind(("to_state", event.to_state.clone()))
            .bind(("reason", event.reason.clone()))
            .await?;

        // Also persist the current state for restart recovery.
        self.upsert_device_state(
            &event.device_id,
            "_lifecycle_state",
            &serde_json::json!(event.to_state),
        )
        .await?;

        Ok(())
    }

    /// Get the most recent lifecycle events for a device, newest first.
    ///
    /// Returns up to `limit` events. Use for post-mortem analysis.
    pub async fn get_lifecycle_events(
        &self,
        device_id: &str,
        limit: u32,
    ) -> Result<Vec<DeviceLifecycleEvent>> {
        // SurrealDB requires ORDER BY fields to appear in SELECT.
        // We select timestamp for ordering but ignore it in deserialization
        // (DeviceLifecycleEvent uses #[serde(default)] behavior — extra fields
        // are simply discarded by serde).
        let mut response = self
            .client()
            .query(
                "SELECT device_id, from_state, to_state, reason, timestamp \
                 FROM device_lifecycle_event \
                 WHERE device_id = $device_id \
                 ORDER BY timestamp DESC \
                 LIMIT $limit",
            )
            .bind(("device_id", device_id.to_owned()))
            .bind(("limit", limit))
            .await?;
        let rows: Vec<DeviceLifecycleEvent> = response.take(0)?;
        Ok(rows)
    }

    /// Delete all lifecycle events for a device.
    ///
    /// Called when a device is unregistered to clean up stale history.
    pub async fn delete_lifecycle_events(&self, device_id: &str) -> Result<usize> {
        let mut response = self
            .client()
            .query(
                "DELETE FROM device_lifecycle_event \
                 WHERE device_id = $device_id RETURN BEFORE",
            )
            .bind(("device_id", device_id.to_owned()))
            .await?;
        let deleted: Vec<DeviceLifecycleEvent> = response.take(0)?;
        let count = deleted.len();
        if count > 0 {
            info!(device_id, count, "deleted device lifecycle events");
        }
        Ok(count)
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
// Config Hashing (for change detection)
// ---------------------------------------------------------------------------

/// Compute a deterministic hash of a JSON config for change detection.
///
/// Uses canonical JSON serialization (sorted keys) to ensure the hash is
/// independent of key insertion order, then hashes with `DefaultHasher`.
///
/// # Stability
///
/// Note: `DefaultHasher` is not stable across Rust versions, but that's
/// acceptable here — hashes are only compared within a single process run
/// (used by the reconciler to detect config changes).
///
/// # Example
///
/// ```rust
/// use db::config_store::config_hash;
/// use serde_json::json;
///
/// let config1 = json!({"port": "/dev/ttyS0", "address": "2"});
/// let config2 = json!({"address": "2", "port": "/dev/ttyS0"});
/// assert_eq!(config_hash(&config1), config_hash(&config2));
/// ```
pub fn config_hash(config: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let s = canonical_json(config);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Produce a canonical JSON string with sorted object keys.
///
/// Ensures deterministic serialization regardless of `serde_json` map backend
/// (BTreeMap vs IndexMap with `preserve_order` feature).
///
/// This function recursively sorts all object keys to produce a consistent
/// string representation that can be hashed for change detection.
fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut pairs: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            pairs.sort_by_key(|(k, _)| *k);
            let inner: String = pairs
                .into_iter()
                .map(|(k, v)| {
                    // Use serde_json for proper key escaping (handles quotes,
                    // backslashes, control chars in keys).
                    let key = serde_json::to_string(k).expect("String serialization is infallible");
                    format!("{}:{}", key, canonical_json(v))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        serde_json::Value::Array(arr) => {
            let inner: String = arr.iter().map(canonical_json).collect::<Vec<_>>().join(",");
            format!("[{inner}]")
        }
        _ => v.to_string(),
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
                commands: vec![],
            },
            DbDriver {
                driver_type: "newport1830_c".into(),
                name: "Newport 1830-C".into(),
                capabilities: vec!["readable".into()],
                commands: vec![],
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
        assert!(all[0].commands.is_empty());
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
            commands: vec![],
        }];
        db.upsert_drivers(&drivers).await.unwrap();

        // Update capabilities
        let drivers = vec![DbDriver {
            driver_type: "mock".into(),
            name: "Mock Driver v2".into(),
            capabilities: vec!["readable".into(), "movable".into(), "configurable".into()],
            commands: vec!["self_test".into(), "reset".into()],
        }];
        db.upsert_drivers(&drivers).await.unwrap();

        let all = db.get_all_drivers().await.unwrap();
        assert_eq!(all.len(), 1, "should not duplicate driver");
        assert_eq!(all[0].name, "Mock Driver v2");
        assert_eq!(all[0].capabilities.len(), 3);
        assert_eq!(all[0].commands, vec!["self_test", "reset"]);
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

    #[test]
    fn test_config_hash_deterministic() {
        // Same config with different key order should produce same hash
        let config1 = serde_json::json!({
            "port": "/dev/ttyS0",
            "address": "2",
            "enabled": true
        });
        let config2 = serde_json::json!({
            "enabled": true,
            "address": "2",
            "port": "/dev/ttyS0"
        });
        assert_eq!(config_hash(&config1), config_hash(&config2));
    }

    #[test]
    fn test_config_hash_detects_changes() {
        let config1 = serde_json::json!({"port": "/dev/ttyS0", "address": "2"});
        let config2 = serde_json::json!({"port": "/dev/ttyS1", "address": "2"});
        assert_ne!(config_hash(&config1), config_hash(&config2));
    }

    #[test]
    fn test_config_hash_nonzero() {
        // Hash should be non-zero for non-empty configs
        let config = serde_json::json!({"port": "/dev/ttyS0"});
        assert_ne!(config_hash(&config), 0);
    }

    // -------------------------------------------------------------------
    // Device Feature tests
    // -------------------------------------------------------------------

    fn sample_device_features() -> Vec<DbDeviceFeature> {
        vec![
            DbDeviceFeature {
                device_id: "andor_istar_0".into(),
                feature_name: "ExposureTime".into(),
                feature_type: "float".into(),
                readable: true,
                writable: true,
                min_value: Some(0.0001),
                max_value: Some(10.0),
                step: Some(0.0001),
                enum_values: vec![],
                unit: Some("s".into()),
                description: Some("Camera exposure time".into()),
                group_name: Some("Acquisition".into()),
            },
            DbDeviceFeature {
                device_id: "andor_istar_0".into(),
                feature_name: "MCPGain".into(),
                feature_type: "int".into(),
                readable: true,
                writable: true,
                min_value: Some(0.0),
                max_value: Some(4095.0),
                step: Some(1.0),
                enum_values: vec![],
                unit: None,
                description: Some("Intensifier MCP gain".into()),
                group_name: Some("Intensifier".into()),
            },
            DbDeviceFeature {
                device_id: "andor_istar_0".into(),
                feature_name: "TriggerMode".into(),
                feature_type: "enum".into(),
                readable: true,
                writable: true,
                min_value: None,
                max_value: None,
                step: None,
                enum_values: vec!["Internal".into(), "External".into(), "Software".into()],
                unit: None,
                description: Some("Trigger source selection".into()),
                group_name: Some("Acquisition".into()),
            },
        ]
    }

    #[tokio::test]
    async fn test_upsert_and_get_device_features() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let features = sample_device_features();
        let count = db.upsert_device_features(&features).await.unwrap();
        assert_eq!(count, 3);

        let retrieved = db.get_device_features("andor_istar_0").await.unwrap();
        assert_eq!(retrieved.len(), 3);

        // Results are ordered by feature_name
        assert_eq!(retrieved[0].feature_name, "ExposureTime");
        assert_eq!(retrieved[0].feature_type, "float");
        assert!(retrieved[0].readable);
        assert!(retrieved[0].writable);
        assert_eq!(retrieved[0].min_value, Some(0.0001));
        assert_eq!(retrieved[0].max_value, Some(10.0));
        assert_eq!(retrieved[0].unit, Some("s".into()));
        assert_eq!(retrieved[0].group_name, Some("Acquisition".into()));

        assert_eq!(retrieved[1].feature_name, "MCPGain");
        assert_eq!(retrieved[1].feature_type, "int");
        assert_eq!(retrieved[1].max_value, Some(4095.0));

        assert_eq!(retrieved[2].feature_name, "TriggerMode");
        assert_eq!(retrieved[2].feature_type, "enum");
        assert_eq!(
            retrieved[2].enum_values,
            vec!["Internal", "External", "Software"]
        );
    }

    #[tokio::test]
    async fn test_upsert_device_features_idempotent() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let features = sample_device_features();
        db.upsert_device_features(&features).await.unwrap();
        db.upsert_device_features(&features).await.unwrap();

        let retrieved = db.get_device_features("andor_istar_0").await.unwrap();
        assert_eq!(
            retrieved.len(),
            3,
            "upsert should not duplicate device features"
        );
    }

    #[tokio::test]
    async fn test_delete_device_features() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let features = sample_device_features();
        db.upsert_device_features(&features).await.unwrap();

        let deleted = db.delete_device_features("andor_istar_0").await.unwrap();
        assert_eq!(deleted, 3);

        let remaining = db.get_device_features("andor_istar_0").await.unwrap();
        assert!(remaining.is_empty(), "all features should be deleted");

        // Deleting again should return 0
        let deleted_again = db.delete_device_features("andor_istar_0").await.unwrap();
        assert_eq!(deleted_again, 0);
    }

    #[tokio::test]
    async fn test_get_device_features_empty() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let features = db.get_device_features("nonexistent_device").await.unwrap();
        assert!(
            features.is_empty(),
            "querying non-existent device should return empty vec"
        );
    }

    // -------------------------------------------------------------------
    // Device Runtime State tests
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_upsert_and_get_device_state() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        db.upsert_device_state("stage_1", "position", &serde_json::json!(42.5))
            .await
            .unwrap();
        db.upsert_device_state("stage_1", "velocity", &serde_json::json!(10.0))
            .await
            .unwrap();

        let states = db.get_device_state("stage_1").await.unwrap();
        assert_eq!(states.len(), 2);

        // Results are ordered by param_name
        assert_eq!(states[0].device_id, "stage_1");
        assert_eq!(states[0].param_name, "position");
        assert_eq!(states[0].param_value, serde_json::json!(42.5));

        assert_eq!(states[1].param_name, "velocity");
        assert_eq!(states[1].param_value, serde_json::json!(10.0));
    }

    #[tokio::test]
    async fn test_upsert_device_state_overwrites() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        // Insert initial value
        db.upsert_device_state("camera_0", "ExposureTime", &serde_json::json!(0.001))
            .await
            .unwrap();

        // Overwrite with new value
        db.upsert_device_state("camera_0", "ExposureTime", &serde_json::json!(0.05))
            .await
            .unwrap();

        let states = db.get_device_state("camera_0").await.unwrap();
        assert_eq!(states.len(), 1, "upsert should not duplicate records");
        assert_eq!(
            states[0].param_value,
            serde_json::json!(0.05),
            "should have the latest value"
        );
    }

    #[tokio::test]
    async fn test_batch_upsert_device_state() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let states = vec![
            (
                "stage_1".to_owned(),
                "position".to_owned(),
                serde_json::json!(100.0),
            ),
            (
                "stage_1".to_owned(),
                "velocity".to_owned(),
                serde_json::json!(5.0),
            ),
            (
                "camera_0".to_owned(),
                "ExposureTime".to_owned(),
                serde_json::json!(0.01),
            ),
        ];
        db.batch_upsert_device_state(&states).await.unwrap();

        // Check stage_1 params
        let stage_states = db.get_device_state("stage_1").await.unwrap();
        assert_eq!(stage_states.len(), 2);
        assert_eq!(stage_states[0].param_name, "position");
        assert_eq!(stage_states[0].param_value, serde_json::json!(100.0));
        assert_eq!(stage_states[1].param_name, "velocity");
        assert_eq!(stage_states[1].param_value, serde_json::json!(5.0));

        // Check camera_0 params
        let camera_states = db.get_device_state("camera_0").await.unwrap();
        assert_eq!(camera_states.len(), 1);
        assert_eq!(camera_states[0].param_name, "ExposureTime");
        assert_eq!(camera_states[0].param_value, serde_json::json!(0.01));
    }

    #[tokio::test]
    async fn test_batch_upsert_device_state_empty() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        // Empty batch should be a no-op
        db.batch_upsert_device_state(&[]).await.unwrap();
    }

    #[tokio::test]
    async fn test_get_device_state_empty() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let states = db.get_device_state("nonexistent_device").await.unwrap();
        assert!(
            states.is_empty(),
            "querying non-existent device should return empty vec"
        );
    }

    #[tokio::test]
    async fn test_delete_device_state() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        db.upsert_device_state("stage_1", "position", &serde_json::json!(42.5))
            .await
            .unwrap();
        db.upsert_device_state("stage_1", "velocity", &serde_json::json!(10.0))
            .await
            .unwrap();

        let deleted = db.delete_device_state("stage_1").await.unwrap();
        assert_eq!(deleted, 2);

        let remaining = db.get_device_state("stage_1").await.unwrap();
        assert!(remaining.is_empty(), "all state should be deleted");

        // Deleting again should return 0
        let deleted_again = db.delete_device_state("stage_1").await.unwrap();
        assert_eq!(deleted_again, 0);
    }

    #[tokio::test]
    async fn test_upsert_device_state_complex_values() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        // Test with various JSON value types
        db.upsert_device_state("dev_0", "string_param", &serde_json::json!("Internal"))
            .await
            .unwrap();
        db.upsert_device_state("dev_0", "bool_param", &serde_json::json!(true))
            .await
            .unwrap();
        db.upsert_device_state("dev_0", "int_param", &serde_json::json!(4095))
            .await
            .unwrap();
        db.upsert_device_state(
            "dev_0",
            "object_param",
            &serde_json::json!({"x": 1.0, "y": 2.0}),
        )
        .await
        .unwrap();

        let states = db.get_device_state("dev_0").await.unwrap();
        assert_eq!(states.len(), 4);

        // Ordered by param_name
        assert_eq!(states[0].param_name, "bool_param");
        assert_eq!(states[0].param_value, serde_json::json!(true));

        assert_eq!(states[1].param_name, "int_param");
        assert_eq!(states[1].param_value, serde_json::json!(4095));

        assert_eq!(states[2].param_name, "object_param");
        assert_eq!(
            states[2].param_value,
            serde_json::json!({"x": 1.0, "y": 2.0})
        );

        assert_eq!(states[3].param_name, "string_param");
        assert_eq!(states[3].param_value, serde_json::json!("Internal"));
    }

    // -------------------------------------------------------------------
    // Device Lifecycle Event tests (bd-oqo7.9)
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn test_record_and_get_lifecycle_events() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let event1 = DeviceLifecycleEvent {
            device_id: "pvcam_0".into(),
            from_state: "initializing".into(),
            to_state: "ready".into(),
            reason: None,
        };
        db.record_lifecycle_event(&event1).await.unwrap();

        let event2 = DeviceLifecycleEvent {
            device_id: "pvcam_0".into(),
            from_state: "ready".into(),
            to_state: "streaming".into(),
            reason: Some("acquisition started".into()),
        };
        db.record_lifecycle_event(&event2).await.unwrap();

        let events = db.get_lifecycle_events("pvcam_0", 10).await.unwrap();
        assert_eq!(events.len(), 2);

        // Newest first (ORDER BY timestamp DESC).
        assert_eq!(events[0].from_state, "ready");
        assert_eq!(events[0].to_state, "streaming");
        assert_eq!(events[0].reason, Some("acquisition started".into()));

        assert_eq!(events[1].from_state, "initializing");
        assert_eq!(events[1].to_state, "ready");
        assert!(events[1].reason.is_none());
    }

    #[tokio::test]
    async fn test_lifecycle_event_persists_current_state() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let event = DeviceLifecycleEvent {
            device_id: "pvcam_0".into(),
            from_state: "initializing".into(),
            to_state: "ready".into(),
            reason: None,
        };
        db.record_lifecycle_event(&event).await.unwrap();

        // Should also update device_runtime_state with _lifecycle_state.
        let states = db.get_device_state("pvcam_0").await.unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].param_name, "_lifecycle_state");
        assert_eq!(states[0].param_value, serde_json::json!("ready"));
    }

    #[tokio::test]
    async fn test_lifecycle_event_limit() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        for i in 0..5 {
            let event = DeviceLifecycleEvent {
                device_id: "cam".into(),
                from_state: format!("state_{i}"),
                to_state: format!("state_{}", i + 1),
                reason: None,
            };
            db.record_lifecycle_event(&event).await.unwrap();
        }

        let events = db.get_lifecycle_events("cam", 3).await.unwrap();
        assert_eq!(events.len(), 3, "should respect limit");
    }

    #[tokio::test]
    async fn test_delete_lifecycle_events() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let event = DeviceLifecycleEvent {
            device_id: "pvcam_0".into(),
            from_state: "ready".into(),
            to_state: "error".into(),
            reason: Some("controller dead".into()),
        };
        db.record_lifecycle_event(&event).await.unwrap();

        let deleted = db.delete_lifecycle_events("pvcam_0").await.unwrap();
        assert_eq!(deleted, 1);

        let events = db.get_lifecycle_events("pvcam_0", 10).await.unwrap();
        assert!(events.is_empty());

        // Deleting again returns 0.
        let deleted_again = db.delete_lifecycle_events("pvcam_0").await.unwrap();
        assert_eq!(deleted_again, 0);
    }

    #[tokio::test]
    async fn test_lifecycle_events_empty_for_unknown_device() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let events = db.get_lifecycle_events("nonexistent", 10).await.unwrap();
        assert!(events.is_empty());
    }
}
