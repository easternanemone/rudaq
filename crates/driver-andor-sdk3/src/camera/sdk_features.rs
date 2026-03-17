//! SDK3 feature get/set helpers for the Andor camera.

use super::AndorCamera;
use anyhow::Result;

#[cfg(feature = "camera")]
use andor_sdk3_sys::*;

impl AndorCamera {
    #[cfg(feature = "camera")]
    pub(crate) fn set_enum_feature(handle: AT_H, feature: &str, value: &str) -> Result<()> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let value_wide = to_wide_string(value);
            // SAFETY: handle is valid (camera is open), feature and value are valid wide strings
            let ret = AT_SetEnumString(handle, feature_wide.as_ptr(), value_wide.as_ptr());
            sdk_result(ret)?;
            Ok(())
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn set_int_feature(handle: AT_H, feature: &str, value: i64) -> Result<()> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            // SAFETY: handle is valid (camera is open), feature is valid wide string, value is i64
            let ret = AT_SetInt(handle, feature_wide.as_ptr(), value);
            sdk_result(ret)?;
            Ok(())
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn set_float_feature(handle: AT_H, feature: &str, value: f64) -> Result<()> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            // SAFETY: handle is valid (camera is open), feature is valid wide string, value is f64
            let ret = AT_SetFloat(handle, feature_wide.as_ptr(), value);
            sdk_result(ret)?;
            Ok(())
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn set_bool_feature(handle: AT_H, feature: &str, value: bool) -> Result<()> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            // SAFETY: handle is valid (camera is open), feature is valid wide string,
            // bool value is converted to SDK's AT_TRUE/AT_FALSE constants
            let ret = AT_SetBool(
                handle,
                feature_wide.as_ptr(),
                if value { AT_TRUE } else { AT_FALSE },
            );
            sdk_result(ret)?;
            Ok(())
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn get_float_feature(handle: AT_H, feature: &str) -> Result<f64> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: f64 = 0.0;
            // SAFETY: handle is valid (camera is open), feature is valid wide string,
            // value pointer is valid for writing f64
            let ret = AT_GetFloat(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value)
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn get_float_min(handle: AT_H, feature: &str) -> Result<f64> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: f64 = 0.0;
            // SAFETY: handle is valid (camera is open), feature_wide is NUL-terminated,
            // value is a valid aligned f64 pointer
            let ret = AT_GetFloatMin(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value)
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn get_float_max(handle: AT_H, feature: &str) -> Result<f64> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: f64 = 0.0;
            // SAFETY: handle is valid (camera is open), feature_wide is NUL-terminated,
            // value is a valid aligned f64 pointer
            let ret = AT_GetFloatMax(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value)
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn get_int_feature(handle: AT_H, feature: &str) -> Result<i64> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: AT_64 = 0;
            let ret = AT_GetInt(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value)
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn get_bool_feature(handle: AT_H, feature: &str) -> Result<bool> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: AT_BOOL = 0;
            let ret = AT_GetBool(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value != AT_FALSE)
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn get_enum_string(handle: AT_H, feature: &str) -> Result<String> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut index: std::os::raw::c_int = 0;
            let ret = AT_GetEnumIndex(handle, feature_wide.as_ptr(), &mut index);
            sdk_result(ret)?;

            let mut buffer = wide_string_buffer(256);
            let ret = AT_GetEnumStringByIndex(
                handle,
                feature_wide.as_ptr(),
                index,
                buffer.as_mut_ptr(),
                256,
            );
            sdk_result(ret)?;
            Ok(from_wide_string(&buffer))
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn is_feature_implemented(handle: AT_H, feature: &str) -> Result<bool> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut implemented: AT_BOOL = AT_FALSE;
            // SAFETY: handle is valid (camera is open), feature_wide is NUL-terminated,
            // implemented is a valid aligned AT_BOOL pointer
            let ret = AT_IsImplemented(handle, feature_wide.as_ptr(), &mut implemented);
            sdk_result(ret)?;
            Ok(implemented != AT_FALSE)
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn is_feature_writable(handle: AT_H, feature: &str) -> Result<bool> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut writable: AT_BOOL = AT_FALSE;
            // SAFETY: handle is valid (camera is open), feature_wide is NUL-terminated,
            // writable is a valid aligned AT_BOOL pointer
            let ret = AT_IsWritable(handle, feature_wide.as_ptr(), &mut writable);
            sdk_result(ret)?;
            Ok(writable != AT_FALSE)
        }
    }

    /// Check if streaming and bail if so. Used by parameter setters that
    /// cannot safely change while the SDK acquisition loop is running.
    pub(crate) fn check_not_streaming(&self) -> Result<()> {
        if self
            .inner
            .streaming
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            anyhow::bail!(
                "Cannot change parameter while acquisition is running. Stop stream first."
            );
        }
        Ok(())
    }
}
