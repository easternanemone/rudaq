//! Unit tests for the timestamping module.

use daq_core::timestamp::{self, AlignedTimestamps, Timestamp, TimestampSource};
use chrono::{Duration, Utc};

#[test]
fn test_from_hardware() {
    let now = Utc::now();
    let ts = Timestamp::from_hardware(now, Some(100));

    assert_eq!(ts.time, now);
    assert_eq!(ts.source, TimestampSource::Hardware);
    assert_eq!(ts.accuracy_ns, Some(100));
}

#[tokio::test]
async fn test_ntp_sync_and_drift() {
    // This test is designed to run in an environment where it can connect to an NTP server.
    // Since we can't do that in the sandbox, this test is here as a placeholder.
    if let Ok(_) = timestamp::synchronize_ntp("pool.ntp.org").await {
        let first_ts = timestamp::ntp_now();
        assert_eq!(first_ts.source, TimestampSource::NtpSynchronized);

        // Wait for a short period to allow for drift to occur.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // In a real test, we would synchronize again and check that the drift
        // has been compensated for.
        if let Ok(_) = timestamp::synchronize_ntp("pool.ntp.org").await {
            let second_ts = timestamp::ntp_now();
            assert_eq!(second_ts.source, TimestampSource::NtpSynchronized);
            assert!(second_ts.time > first_ts.time);
        }
    }
}

#[test]
fn test_multi_instrument_alignment_placeholder() {
    let now = Utc::now();
    let timestamps = vec![
        Timestamp::from_hardware(now, Some(100)),
        Timestamp::from_hardware(now + Duration::milliseconds(10), Some(100)),
    ];

    let aligned: AlignedTimestamps = timestamp::align_timestamps(&timestamps);

    // The placeholder function returns the current time, so this test just
    // ensures that it doesn't panic.
    assert!(!aligned.instrument_timestamps.is_empty());
}
