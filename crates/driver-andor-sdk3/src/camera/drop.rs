//! Drop implementation for `AndorCameraInner`.

use super::AndorCameraInner;
#[cfg(feature = "camera")]
use super::{AndorCamera, FeatureCallbackBridge};
#[cfg(feature = "camera")]
use andor_sdk3_sys::*;
use std::sync::atomic::Ordering;

impl Drop for AndorCameraInner {
    fn drop(&mut self) {
        // Abort drift polling task if running
        if let Ok(mut guard) = self.drift_task_handle.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }

        // Abort acquisition task if running
        if let Ok(mut guard) = self.acq_task_handle.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }

        // Shut down feature callback pipeline:
        // 1. Drop the sender to close the channel (receiver task will exit)
        // 2. Abort the receiver task as a backstop
        // The bridge Box is dropped automatically with the struct fields,
        // AFTER AT_Close() below unregisters all SDK callbacks.
        #[cfg(feature = "camera")]
        {
            if let Ok(mut guard) = self.callback_tx.try_lock() {
                guard.take(); // Drop the sender → channel closes
            }
        }
        if let Ok(mut guard) = self.callback_task_handle.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }

        #[cfg(feature = "camera")]
        unsafe {
            if self.handle != AT_HANDLE_UNINITIALISED {
                // Stop acquisition if still streaming
                if self.streaming.load(Ordering::Relaxed) {
                    let feature = to_wide_string("AcquisitionStop");
                    let _ = AT_Command(self.handle, feature.as_ptr());
                    self.streaming.store(false, Ordering::Relaxed);
                }

                // Flush all queued buffers before closing
                let _ = AT_Flush(self.handle);

                // Explicitly unregister feature callbacks before AT_Close (bd-cytq).
                // The SDK manual requires explicit unregistration; relying on AT_Close
                // to clean up callbacks is undocumented behavior.
                // The context pointer must match what was passed to AT_RegisterFeatureCallback.
                let bridge_ctx = self._callback_bridge.try_lock().ok().and_then(|guard| {
                    guard
                        .as_ref()
                        .map(|b| &**b as *const FeatureCallbackBridge as *mut std::os::raw::c_void)
                });
                if let Ok(cbs) = self.registered_callbacks.lock() {
                    let ctx = bridge_ctx.unwrap_or(std::ptr::null_mut());
                    if !cbs.is_empty() {
                        tracing::info!(
                            sdk_handle = self.handle,
                            count = cbs.len(),
                            callback_ctx = ?ctx,
                            "Unregistering Andor SDK3 feature callbacks before AT_Close"
                        );
                    }
                    for name in cbs.iter() {
                        let feature_wide = andor_sdk3_sys::to_wide_string(name);
                        let _ = AT_UnregisterFeatureCallback(
                            self.handle,
                            feature_wide.as_ptr(),
                            Some(AndorCamera::sdk_feature_callback),
                            ctx,
                        );
                        tracing::debug!(
                            sdk_handle = self.handle,
                            feature = %name,
                            callback_ctx = ?ctx,
                            "AT_UnregisterFeatureCallback issued"
                        );
                    }
                    if !cbs.is_empty() {
                        tracing::debug!(
                            sdk_handle = self.handle,
                            count = cbs.len(),
                            callback_ctx = ?ctx,
                            "Unregistered Andor SDK3 feature callbacks"
                        );
                    }
                }

                AT_Close(self.handle);

                // Only finalize library when last instance is dropped
                if super::LIBRARY_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                    AT_FinaliseLibrary();
                }
            }
        }
    }
}
