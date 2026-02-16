//! Phase 0 spike cost measurement utilities.
//!
//! Provides functions to measure SurrealDB startup costs that Criterion
//! benchmarks alone can't capture (memory usage, binary size deltas, etc.).
//!
#![allow(dead_code)] // Used by benches/ harness, not from lib code
//! Run the standalone report via:
//! ```bash
//! cargo test -p db --features kv-mem --test spike_report -- --nocapture
//! ```

use std::time::Instant;

use crate::{DaqDb, DbConfig};

/// Measure cold-start time (wall clock) and basic resource usage.
///
/// Returns a [`SpikeReport`] with the measurements.
pub async fn measure_startup_costs() -> SpikeReport {
    // --- Cold start ---
    let t0 = Instant::now();
    let db = DaqDb::init(DbConfig::in_memory())
        .await
        .expect("DaqDb::init failed");
    let cold_start = t0.elapsed();

    // --- Health check latency ---
    let t1 = Instant::now();
    let healthy = db.health_check().await;
    let health_check_latency = t1.elapsed();

    // --- Write latency (single record) ---
    let t2 = Instant::now();
    db.client()
        .query(
            "CREATE instrument SET \
                device_id = 'bench_0', \
                name = 'Bench Device', \
                driver_type = 'mock', \
                config = { port: '/dev/null' }, \
                enabled = true, \
                status = 'offline', \
                created_at = time::now(), \
                updated_at = time::now()",
        )
        .await
        .expect("insert failed");
    let write_latency = t2.elapsed();

    // --- Read latency (single record) ---
    let t3 = Instant::now();
    let mut response = db
        .client()
        .query("SELECT device_id, name FROM instrument WHERE device_id = 'bench_0'")
        .await
        .expect("query failed");
    let _rows: Vec<serde_json::Value> = response.take(0).expect("take failed");
    let read_latency = t3.elapsed();

    // --- Bulk write (100 instruments) ---
    let t4 = Instant::now();
    for i in 1..=100 {
        db.client()
            .query(
                "CREATE instrument SET \
                    device_id = $device_id, \
                    name = $name, \
                    driver_type = 'mock', \
                    config = { port: '/dev/null' }, \
                    enabled = true, \
                    status = 'offline', \
                    created_at = time::now(), \
                    updated_at = time::now()",
            )
            .bind(("device_id", format!("bulk_{i}")))
            .bind(("name", format!("Bulk Device {i}")))
            .await
            .expect("bulk insert failed");
    }
    let bulk_write_100 = t4.elapsed();

    // --- Full table scan ---
    let t5 = Instant::now();
    let mut response = db
        .client()
        .query("SELECT device_id, name, driver_type FROM instrument")
        .await
        .expect("scan failed");
    let rows: Vec<serde_json::Value> = response.take(0).expect("take failed");
    let scan_latency = t5.elapsed();
    let row_count = rows.len();

    SpikeReport {
        cold_start,
        health_check_latency,
        healthy,
        write_latency,
        read_latency,
        bulk_write_100,
        scan_latency,
        row_count,
    }
}

/// Phase 0 spike measurement results.
#[derive(Debug)]
pub struct SpikeReport {
    pub cold_start: std::time::Duration,
    pub health_check_latency: std::time::Duration,
    pub healthy: bool,
    pub write_latency: std::time::Duration,
    pub read_latency: std::time::Duration,
    pub bulk_write_100: std::time::Duration,
    pub scan_latency: std::time::Duration,
    pub row_count: usize,
}

impl std::fmt::Display for SpikeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== SurrealDB Phase 0 Spike Report ===")?;
        writeln!(f)?;
        writeln!(f, "Cold start (init + schema): {:>10.2?}", self.cold_start)?;
        writeln!(
            f,
            "Health check latency:       {:>10.2?}",
            self.health_check_latency
        )?;
        writeln!(f, "Health check passed:        {:>10}", self.healthy)?;
        writeln!(
            f,
            "Single write latency:       {:>10.2?}",
            self.write_latency
        )?;
        writeln!(
            f,
            "Single read latency:        {:>10.2?}",
            self.read_latency
        )?;
        writeln!(
            f,
            "Bulk write (100 records):   {:>10.2?}",
            self.bulk_write_100
        )?;
        writeln!(
            f,
            "  Per-record avg:           {:>10.2?}",
            self.bulk_write_100 / 100
        )?;
        writeln!(
            f,
            "Full table scan ({} rows): {:>10.2?}",
            self.row_count, self.scan_latency
        )?;
        writeln!(f)?;
        writeln!(f, "=== End Report ===")
    }
}
