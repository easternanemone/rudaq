// ============================================================================
// Camera Model Constants
// ============================================================================

/// Prime BSI sensor dimensions (default)
pub const PRIME_BSI_WIDTH: u32 = 2048;
pub const PRIME_BSI_HEIGHT: u32 = 2048;

/// Prime 95B sensor dimensions (optional)
#[cfg(feature = "prime_95b_tests")]
pub const PRIME_95B_WIDTH: u32 = 1200;
#[cfg(feature = "prime_95b_tests")]
pub const PRIME_95B_HEIGHT: u32 = 1200;

/// Get the default camera name for tests
pub fn default_camera_name() -> &'static str {
    #[cfg(feature = "prime_95b_tests")]
    {
        "Prime95B"
    }
    #[cfg(not(feature = "prime_95b_tests"))]
    {
        "PrimeBSI"
    }
}

/// Get the expected sensor width
pub fn expected_width() -> u32 {
    #[cfg(feature = "prime_95b_tests")]
    {
        PRIME_95B_WIDTH
    }
    #[cfg(not(feature = "prime_95b_tests"))]
    {
        PRIME_BSI_WIDTH
    }
}

/// Get the expected sensor height
pub fn expected_height() -> u32 {
    #[cfg(feature = "prime_95b_tests")]
    {
        PRIME_95B_HEIGHT
    }
    #[cfg(not(feature = "prime_95b_tests"))]
    {
        PRIME_BSI_HEIGHT
    }
}
