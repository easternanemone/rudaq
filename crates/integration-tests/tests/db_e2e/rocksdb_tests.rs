// ============================================================================
// WP4: RocksDB Persistence (T9) -- compile-gated
// ============================================================================

// File-backed SQLite tests require the db feature and use a temp directory.
// Uses subprocess isolation to verify persistence across process restarts.

use db::{DaqDb, DbConfig};
use tempfile::TempDir;

use super::helpers::*;

/// Helper subprocess that writes the full mock_maitai_lab config to a file-backed DB.
///
/// Invoked as a subprocess by the main test to verify data persists
/// across process restarts.
#[tokio::test]
#[ignore = "only invoked as a subprocess by test_t9_rocksdb_persistence_across_restart"]
async fn rocksdb_t9_writer_helper() {
    let db_path = std::env::var("ROCKSDB_TEST_PATH").expect("ROCKSDB_TEST_PATH must be set");
    let config = DbConfig::file(db_path);
    let db = DaqDb::init(config).await.unwrap();

    let hw_config = load_mock_maitai_config();
    shadow_write(&db, &hw_config).await.unwrap();

    // Verify data was written before exiting
    let instruments = db.get_all_instruments().await.unwrap();
    assert_eq!(instruments.len(), 9, "should write 9 instruments");
    let drivers = db.get_all_drivers().await.unwrap();
    assert_eq!(drivers.len(), 5, "should write 5 drivers");
}

/// T9: Instruments survive daemon restart with file-backed persistence.
///
/// Uses subprocess isolation: spawns `rocksdb_t9_writer_helper` as a
/// separate OS process to write data. The main test then reopens the
/// database and verifies all 9 instruments and 5 drivers persist.
#[tokio::test]
async fn test_t9_rocksdb_persistence_across_restart() {
    let tmpdir = TempDir::new().unwrap();
    let db_path = tmpdir.path().join("test.db");

    // First boot: spawn the writer helper as a separate OS process.
    let test_bin = std::env::current_exe().unwrap();
    let output = std::process::Command::new(&test_bin)
        .arg("rocksdb_tests::rocksdb_t9_writer_helper")
        .arg("--exact")
        .arg("--ignored")
        .arg("--nocapture")
        .env("ROCKSDB_TEST_PATH", db_path.to_str().unwrap())
        .output()
        .expect("failed to spawn writer subprocess");
    assert!(
        output.status.success(),
        "writer subprocess failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Second boot: re-open and verify data persists
    let db = DaqDb::init(DbConfig::file(db_path.display().to_string()))
        .await
        .unwrap();

    let instruments = db.get_all_instruments().await.unwrap();
    assert_eq!(
        instruments.len(),
        9,
        "instruments should persist across restart"
    );

    let drivers = db.get_all_drivers().await.unwrap();
    assert_eq!(drivers.len(), 5, "drivers should persist across restart");

    // Verify specific instrument round-trip
    let rotator = db.get_instrument("rotator_2").await.unwrap();
    assert!(rotator.is_some(), "rotator_2 should survive restart");
}
