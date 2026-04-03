//! Phase 0 spike benchmarks: measure SurrealDB startup costs.
//!
//! Run with:
//!   cargo bench -p db --features kv-mem --bench startup
//!
//! For RocksDB benchmarks:
//!   cargo bench -p db --features kv-rocksdb --bench startup

use criterion::{Criterion, criterion_group, criterion_main};

fn bench_init_in_memory(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("DaqDb::init (in-memory)", |b| {
        b.iter(|| {
            rt.block_on(async {
                let db = db::DaqDb::init(db::DbConfig::in_memory())
                    .await
                    .expect("init should succeed");
                // Ensure the DB is usable before dropping
                assert!(db.health_check().await);
            });
        });
    });
}

fn bench_health_check(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let db = rt
        .block_on(db::DaqDb::init(db::DbConfig::in_memory()))
        .expect("init should succeed");

    c.bench_function("DaqDb::health_check", |b| {
        b.iter(|| {
            rt.block_on(async {
                assert!(db.health_check().await);
            });
        });
    });
}

fn bench_insert_instrument(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let db = rt
        .block_on(db::DaqDb::init(db::DbConfig::in_memory()))
        .expect("init should succeed");

    let mut counter = 0u64;

    c.bench_function("insert instrument record", |b| {
        b.iter(|| {
            let id = counter;
            counter += 1;
            rt.block_on(async {
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
                    .bind(("device_id", format!("bench_device_{id}")))
                    .bind(("name", format!("Bench Device {id}")))
                    .await
                    .expect("insert should succeed");
            });
        });
    });
}

fn bench_query_instruments(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let db = rt
        .block_on(db::DaqDb::init(db::DbConfig::in_memory()))
        .expect("init should succeed");

    // Seed 100 instruments
    rt.block_on(async {
        for i in 0..100 {
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
                .bind(("device_id", format!("seed_{i}")))
                .bind(("name", format!("Seed Device {i}")))
                .await
                .expect("seed insert should succeed");
        }
    });

    c.bench_function("SELECT * FROM instrument (100 rows)", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut response = db
                    .client()
                    .query("SELECT device_id, name, driver_type FROM instrument")
                    .await
                    .expect("query should succeed");
                let rows: Vec<serde_json::Value> = response.take(0).expect("take should succeed");
                assert_eq!(rows.len(), 100);
            });
        });
    });
}

criterion_group!(
    benches,
    bench_init_in_memory,
    bench_health_check,
    bench_insert_instrument,
    bench_query_instruments
);
criterion_main!(benches);
