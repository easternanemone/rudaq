//! Arrow extension metadata helpers for Python interop.

use std::collections::HashMap;

/// Build Arrow FixedShapeTensor extension metadata for Python interop.
///
/// This metadata enables zero-copy tensor sharing with PyArrow and conversion
/// to Xarray/NumPy via the Arrow FixedShapeTensor extension type.
///
/// # Arguments
/// * `shape` - Tensor dimensions (e.g., `[256, 256]` for 2D image)
/// * `dim_names` - Dimension labels (e.g., `["y", "x"]`)
///
/// # Example
/// ```
/// use daq_core::arrow_metadata::fixed_shape_tensor_metadata;
///
/// let meta = fixed_shape_tensor_metadata(&[256, 256], &["y", "x"]);
/// assert_eq!(meta.get("ARROW:extension:name").unwrap(), "arrow.fixed_shape_tensor");
/// ```
pub fn fixed_shape_tensor_metadata(shape: &[u64], dim_names: &[&str]) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    meta.insert(
        "ARROW:extension:name".into(),
        "arrow.fixed_shape_tensor".into(),
    );

    // Build metadata JSON manually to avoid serde_json dependency in daq-core
    let shape_json: String = format!(
        "[{}]",
        shape
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let names_json: String = format!(
        "[{}]",
        dim_names
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(",")
    );

    meta.insert(
        "ARROW:extension:metadata".into(),
        format!(r#"{{"shape":{},"dim_names":{}}}"#, shape_json, names_json),
    );
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_2d_tensor_metadata() {
        let meta = fixed_shape_tensor_metadata(&[256, 256], &["y", "x"]);
        assert_eq!(
            meta.get("ARROW:extension:name").unwrap(),
            "arrow.fixed_shape_tensor"
        );
        let ext_meta = meta.get("ARROW:extension:metadata").unwrap();
        assert!(ext_meta.contains("\"shape\":[256,256]"));
        assert!(ext_meta.contains("\"dim_names\":[\"y\",\"x\"]"));
    }

    #[test]
    fn test_4d_tensor_metadata() {
        let meta =
            fixed_shape_tensor_metadata(&[10, 5, 256, 256], &["wavelength", "position", "y", "x"]);
        let ext_meta = meta.get("ARROW:extension:metadata").unwrap();
        assert!(ext_meta.contains("\"shape\":[10,5,256,256]"));
        assert!(ext_meta.contains("\"wavelength\""));
    }
}
