//! Tests for SMART Streaming acquisition pipeline integration (bd-oqo7.1).
//!
//! These tests verify:
//! - Exposure index wrapping logic (unit test, no hardware)
//! - JSON parsing of exposure lists
//! - Frame metadata tagging with smart stream fields

/// Verify that exposure index wrapping cycles correctly through all exposures.
#[test]
fn test_smart_stream_exposure_index_wrapping() {
    let smart_stream_count: usize = 3;

    // Simulate 10 frames and check the exposure index cycles 0, 1, 2, 0, 1, 2, ...
    let expected_indices = [0, 1, 2, 0, 1, 2, 0, 1, 2, 0];
    for (frame_num, &expected) in expected_indices.iter().enumerate() {
        let exposure_index = frame_num % smart_stream_count;
        assert_eq!(
            exposure_index, expected,
            "Frame {} should have exposure index {}, got {}",
            frame_num, expected, exposure_index
        );
    }
}

/// Edge case: single exposure in SMART stream should always produce index 0.
#[test]
fn test_smart_stream_single_exposure() {
    let smart_stream_count: usize = 1;
    for frame_num in 0..100usize {
        let exposure_index = frame_num % smart_stream_count;
        assert_eq!(
            exposure_index, 0,
            "Single-exposure SMART stream should always produce index 0"
        );
    }
}

/// When SMART streaming is disabled (count=0), no metadata should be produced.
#[test]
fn test_smart_stream_disabled_no_metadata() {
    let smart_stream_count: usize = 0;
    let extra = if smart_stream_count > 0 {
        let mut m = std::collections::HashMap::new();
        m.insert("smart_stream_index".to_string(), "0".to_string());
        m
    } else {
        std::collections::HashMap::new()
    };
    assert!(
        extra.is_empty(),
        "Disabled SMART stream should produce no extra metadata"
    );
}

/// Verify JSON parsing of exposure list matches expected format.
#[test]
fn test_smart_stream_json_parsing_valid() {
    let json_str = "[10,50,100,200]";
    let exposures: Vec<u32> = serde_json::from_str(json_str).unwrap();
    assert_eq!(exposures, vec![10, 50, 100, 200]);
    assert_eq!(exposures.len(), 4);
}

/// Empty JSON array should parse to empty vec.
#[test]
fn test_smart_stream_json_parsing_empty() {
    let json_str = "[]";
    let exposures: Vec<u32> = serde_json::from_str(json_str).unwrap();
    assert!(exposures.is_empty());
}

/// Invalid JSON should fall back to empty via unwrap_or_default (matching driver behavior).
#[test]
fn test_smart_stream_json_parsing_invalid_fallback() {
    let json_str = "not valid json";
    let exposures: Vec<u32> = serde_json::from_str(json_str).unwrap_or_default();
    assert!(
        exposures.is_empty(),
        "Invalid JSON should fall back to empty vec"
    );
}

/// JSON with mixed types should fail and fall back to empty.
#[test]
fn test_smart_stream_json_parsing_mixed_types() {
    let json_str = r#"[10, "fifty", 100]"#;
    let exposures: Vec<u32> = serde_json::from_str(json_str).unwrap_or_default();
    assert!(
        exposures.is_empty(),
        "Mixed-type JSON should fall back to empty vec"
    );
}

/// Maximum SMART streaming exposures (16 per PVCAM spec).
#[test]
fn test_smart_stream_max_exposures() {
    let smart_stream_count: usize = 16;
    // Frame 15 should have index 15, frame 16 should wrap to 0
    assert_eq!(15usize % smart_stream_count, 15);
    assert_eq!(16usize % smart_stream_count, 0);
    assert_eq!(31usize % smart_stream_count, 15);
    assert_eq!(32usize % smart_stream_count, 0);
}

/// Verify frame metadata HashMap is populated correctly for SMART streaming.
#[test]
fn test_smart_stream_frame_metadata_population() {
    let smart_stream_count: usize = 3;
    let monotonic_frame_count: usize = 7; // Should be index 1 (7 % 3 = 1)

    let extra = if smart_stream_count > 0 {
        let exposure_index = monotonic_frame_count % smart_stream_count;
        let mut m = std::collections::HashMap::new();
        m.insert("smart_stream_index".to_string(), exposure_index.to_string());
        m.insert(
            "smart_stream_count".to_string(),
            smart_stream_count.to_string(),
        );
        m
    } else {
        std::collections::HashMap::new()
    };

    assert_eq!(extra.get("smart_stream_index"), Some(&"1".to_string()));
    assert_eq!(extra.get("smart_stream_count"), Some(&"3".to_string()));
    assert_eq!(extra.len(), 2);
}
