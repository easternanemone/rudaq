use echelle::{EchelleCalibrationProfile, EchelleFrameContext};

#[test]
fn fixture_profile_loads_and_validates() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/echelle_profile_v1.toml");

    let profile = EchelleCalibrationProfile::load_from_path(&path).expect("fixture should load");
    assert_eq!(profile.display_name, "Mechelle Demo Calibration (Fixture)");
    assert_eq!(profile.orders.len(), 2);
    assert_eq!(profile.compatibility.frame_width, 1024);
}

#[test]
fn fixture_profile_runtime_compatibility_passes_for_matching_frame() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/echelle_profile_v1.toml");
    let profile = EchelleCalibrationProfile::load_from_path(&path).unwrap();

    profile
        .validate_for_frame(EchelleFrameContext {
            width: 1024,
            height: 512,
            roi_x: Some(128),
            roi_y: Some(256),
            binning_x: Some(1),
            binning_y: Some(1),
            bit_depth: Some(16),
        })
        .expect("matching frame context should validate");
}
