//! Shared test fixtures for circular buffer diagnostic tests.
//!
//! Contains helper functions, constants, callback context types, and FFI wrappers
//! used across the split circ_buffer_diagnostic test modules:
//! - `buffer_overflow.rs` — Buffer overflow and wrap-around scenarios
//! - `frame_integrity.rs` — Frame data integrity and callback isolation
//! - `acquisition_timing.rs` — Timing, throughput, and driver infrastructure tests

#![allow(dead_code)]

use pvcam_sys::*;
use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};

// PARAM IDs - camera-specific values verified on maitai (not in pvcam_sys)
pub const PARAM_CIRC_BUFFER: u32 = 184746283;
pub const PARAM_EXPOSURE_MODE: u32 = 151126551;
pub const PARAM_EXPOSE_OUT_MODE: u32 = 151126576;
pub const PARAM_SER_SIZE: u32 = 100794426;
pub const PARAM_PAR_SIZE: u32 = 100794425;
pub const PARAM_READOUT_PORT: u32 = 151126263;
pub const PARAM_SPDTAB_INDEX: u32 = 16908801;
pub const PARAM_GAIN_INDEX: u32 = 16908800;
pub const PARAM_BIT_DEPTH: u32 = 16908799;
pub const PARAM_FRAME_BUFFER_SIZE: u32 = 184746284;

/// Get PVCAM error message
pub fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Check if a parameter is available
pub fn is_param_available(hcam: i16, param_id: u32) -> bool {
    let mut avail: rs_bool = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_AVAIL,
            &mut avail as *mut _ as *mut c_void,
        ) != 0
        {
            avail != 0
        } else {
            false
        }
    }
}

/// Get current value of a boolean parameter
pub fn get_bool_param(hcam: i16, param_id: u32) -> Option<bool> {
    let mut value: rs_bool = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_CURRENT,
            &mut value as *mut _ as *mut c_void,
        ) != 0
        {
            Some(value != 0)
        } else {
            None
        }
    }
}

/// Get enum count for a parameter
pub fn get_enum_count(hcam: i16, param_id: u32) -> Option<u32> {
    let mut count: uns32 = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_COUNT,
            &mut count as *mut _ as *mut c_void,
        ) != 0
        {
            Some(count)
        } else {
            None
        }
    }
}

/// Get default value of an i32 parameter (uses ATTR_DEFAULT)
/// This is what PVCamTestCli uses for "<camera default>" values!
pub fn get_default_i32_param(hcam: i16, param_id: u32) -> Option<i32> {
    let mut value: i32 = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_DEFAULT,
            &mut value as *mut _ as *mut c_void,
        ) != 0
        {
            Some(value)
        } else {
            None
        }
    }
}

/// Set an i32 parameter value
pub fn set_i32_param(hcam: i16, param_id: u32, value: i32) -> bool {
    unsafe {
        // pl_set_param takes *mut c_void - cast through raw pointer
        let value_ptr = &value as *const i32 as *mut c_void;
        pl_set_param(hcam, param_id, value_ptr) != 0
    }
}

/// Mimic SDK BuildSpeedTable() - set readout parameters to their defaults
/// The SDK does this in Camera::Open BEFORE any acquisition setup
pub fn init_readout_params_like_sdk(hcam: i16) -> bool {
    println!("  Setting PARAM_READOUT_PORT to default...");
    if is_param_available(hcam, PARAM_READOUT_PORT) {
        if let Some(def_val) = get_default_i32_param(hcam, PARAM_READOUT_PORT) {
            if set_i32_param(hcam, PARAM_READOUT_PORT, def_val) {
                println!("    [OK] PARAM_READOUT_PORT = {}", def_val);
            } else {
                println!(
                    "    [WARN] Failed to set PARAM_READOUT_PORT: {}",
                    get_error_message()
                );
            }
        }
    } else {
        println!("    [SKIP] PARAM_READOUT_PORT not available");
    }

    println!("  Setting PARAM_SPDTAB_INDEX to default...");
    if is_param_available(hcam, PARAM_SPDTAB_INDEX) {
        if let Some(def_val) = get_default_i32_param(hcam, PARAM_SPDTAB_INDEX) {
            if set_i32_param(hcam, PARAM_SPDTAB_INDEX, def_val) {
                println!("    [OK] PARAM_SPDTAB_INDEX = {}", def_val);
            } else {
                println!(
                    "    [WARN] Failed to set PARAM_SPDTAB_INDEX: {}",
                    get_error_message()
                );
            }
        }
    } else {
        println!("    [SKIP] PARAM_SPDTAB_INDEX not available");
    }

    println!("  Setting PARAM_GAIN_INDEX to default...");
    if is_param_available(hcam, PARAM_GAIN_INDEX) {
        if let Some(def_val) = get_default_i32_param(hcam, PARAM_GAIN_INDEX) {
            if set_i32_param(hcam, PARAM_GAIN_INDEX, def_val) {
                println!("    [OK] PARAM_GAIN_INDEX = {}", def_val);
            } else {
                println!(
                    "    [WARN] Failed to set PARAM_GAIN_INDEX: {}",
                    get_error_message()
                );
            }
        }
    } else {
        println!("    [SKIP] PARAM_GAIN_INDEX not available");
    }

    // Check BIT_DEPTH to see current state
    println!("  Checking PARAM_BIT_DEPTH...");
    if is_param_available(hcam, PARAM_BIT_DEPTH) {
        let mut bit_depth: i32 = 0;
        unsafe {
            if pl_get_param(
                hcam,
                PARAM_BIT_DEPTH,
                ATTR_CURRENT,
                &mut bit_depth as *mut _ as *mut c_void,
            ) != 0
            {
                println!("    [OK] PARAM_BIT_DEPTH = {} bits", bit_depth);
            }
        }
    }

    true
}

/// Get enum entry name and value
pub fn get_enum_entry(hcam: i16, param_id: u32, index: u32) -> Option<(i32, String)> {
    let mut value: i32 = 0;
    let mut name = [0i8; 100];
    let mut name_len: uns32 = 100;

    unsafe {
        if pl_enum_str_length(hcam, param_id, index, &mut name_len) == 0 {
            return None;
        }
        if pl_get_enum_param(hcam, param_id, index, &mut value, name.as_mut_ptr(), 100) != 0 {
            let name_str = CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
            Some((value, name_str))
        } else {
            None
        }
    }
}

/// Simple EOF callback for testing
pub extern "system" fn test_eof_callback(_frame_info: *const FRAME_INFO, _context: *mut c_void) {
    // Just log that we received a callback
    eprintln!("[CALLBACK] EOF received");
}

// ============================================================================
// MinimalContext — for test_17 (minimal SDK-style callback)
// ============================================================================

/// Minimal callback context matching C++ SDK pattern
pub struct MinimalContext {
    pub mutex: Mutex<bool>,
    pub condvar: Condvar,
    pub eof_flag: AtomicBool,
    pub frame_nr: AtomicI32,
    pub callback_count: AtomicI32,
}

impl MinimalContext {
    pub fn new() -> Self {
        Self {
            mutex: Mutex::new(false),
            condvar: Condvar::new(),
            eof_flag: AtomicBool::new(false),
            frame_nr: AtomicI32::new(0),
            callback_count: AtomicI32::new(0),
        }
    }

    pub fn signal(&self, frame_nr: i32) {
        self.frame_nr.store(frame_nr, Ordering::Release);
        self.callback_count.fetch_add(1, Ordering::AcqRel);
        let mut guard = self.mutex.lock().unwrap();
        *guard = true;
        self.eof_flag.store(true, Ordering::Release);
        self.condvar.notify_one();
    }

    pub fn wait(&self, timeout_ms: u64) -> bool {
        let guard = self.mutex.lock().unwrap();
        let timeout = std::time::Duration::from_millis(timeout_ms);
        let result = self
            .condvar
            .wait_timeout_while(guard, timeout, |flag| !*flag);
        match result {
            Ok((mut guard, timeout_result)) => {
                *guard = false;
                self.eof_flag.store(false, Ordering::Release);
                !timeout_result.timed_out()
            }
            Err(_) => false,
        }
    }
}

/// Static context for minimal callback (C++ pattern uses local stack variable + pointer)
/// Uses AtomicPtr for safe concurrent access (matching FULL_CTX pattern).
pub static MINIMAL_CTX: std::sync::atomic::AtomicPtr<MinimalContext> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Minimal callback - NO catch_unwind, NO extra synchronization
/// This matches the C++ SDK example exactly
pub extern "system" fn minimal_eof_callback(
    p_frame_info: *const FRAME_INFO,
    _p_context: *mut c_void,
) {
    let ctx_ptr = MINIMAL_CTX.load(Ordering::Acquire);
    if ctx_ptr.is_null() {
        return;
    }
    let ctx = unsafe { &*ctx_ptr };
    let frame_nr = if !p_frame_info.is_null() {
        unsafe { (*p_frame_info).FrameNr }
    } else {
        -1
    };
    let count = ctx.callback_count.load(Ordering::Acquire) + 1;
    eprintln!("[CALLBACK {}] Frame {} ready", count, frame_nr);
    ctx.signal(frame_nr);
}

// ============================================================================
// FullCallbackContext — for tests 18-23, 25
// ============================================================================

/// CallbackContext matching the full driver's structure exactly
#[derive(Debug)]
pub struct FullCallbackContext {
    pub pending_frames: std::sync::atomic::AtomicU32,
    pub latest_frame_nr: AtomicI32,
    pub condvar: Condvar,
    pub mutex: Mutex<bool>,
    pub shutdown: AtomicBool,
    pub hcam: AtomicI16,
    pub frame_ptr: std::sync::atomic::AtomicPtr<c_void>,
    pub frame_info: Mutex<FRAME_INFO>,
    pub circ_overwrite: AtomicBool,
}

impl FullCallbackContext {
    pub fn new(hcam: i16) -> Self {
        Self {
            pending_frames: std::sync::atomic::AtomicU32::new(0),
            latest_frame_nr: AtomicI32::new(-1),
            condvar: Condvar::new(),
            mutex: Mutex::new(false),
            shutdown: AtomicBool::new(false),
            hcam: AtomicI16::new(hcam),
            frame_ptr: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()),
            frame_info: Mutex::new(unsafe { std::mem::zeroed() }),
            circ_overwrite: AtomicBool::new(false), // CIRC_NO_OVERWRITE
        }
    }

    pub fn signal_frame_ready(&self, frame_nr: i32) {
        self.latest_frame_nr.store(frame_nr, Ordering::Release);
        self.pending_frames.fetch_add(1, Ordering::AcqRel);
        let mut guard = match self.mutex.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = true;
        self.condvar.notify_one();
    }

    pub fn wait_for_frames(&self, timeout_ms: u64) -> u32 {
        if self.shutdown.load(Ordering::Acquire) {
            return 0;
        }
        let guard = match self.mutex.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let timeout_duration = std::time::Duration::from_millis(timeout_ms);
        let result = self
            .condvar
            .wait_timeout_while(guard, timeout_duration, |notified| {
                !*notified && !self.shutdown.load(Ordering::Acquire)
            });
        match result {
            Ok((mut guard, timeout_result)) => {
                *guard = false;
                if timeout_result.timed_out() {
                    0
                } else {
                    self.pending_frames.load(Ordering::Acquire).max(1)
                }
            }
            Err(poisoned) => {
                let (mut guard, _) = poisoned.into_inner();
                *guard = false;
                0
            }
        }
    }

    pub fn consume_one(&self) {
        let _ = self
            .pending_frames
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                if n > 0 {
                    Some(n - 1)
                } else {
                    None
                }
            });
    }

    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut guard) = self.mutex.lock() {
            *guard = true;
            self.condvar.notify_all();
        }
    }
}

/// Static global pointer for FullCallbackContext (like full driver's GLOBAL_CALLBACK_CTX)
pub static FULL_CTX: std::sync::atomic::AtomicPtr<FullCallbackContext> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Callback for tests using FullCallbackContext - matches full driver's pvcam_eof_callback
pub extern "system" fn full_eof_callback(p_frame_info: *const FRAME_INFO, _p_context: *mut c_void) {
    static CALLBACK_ENTRY_COUNT: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0);
    let entry_count = CALLBACK_ENTRY_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let ctx_ptr = FULL_CTX.load(Ordering::Acquire);

    if entry_count <= 25 || entry_count % 50 == 0 {
        eprintln!("[FULL CALLBACK ENTRY] #{}, ctx={:?}", entry_count, ctx_ptr);
    }

    if ctx_ptr.is_null() {
        return;
    }
    let ctx = unsafe { &*ctx_ptr };

    let frame_nr = if !p_frame_info.is_null() {
        let info = unsafe { *p_frame_info };
        if info.FrameNr <= 25 || info.FrameNr % 50 == 0 {
            eprintln!(
                "[FULL CALLBACK] Frame {} ready, timestamp={}",
                info.FrameNr, info.TimeStamp
            );
        }
        info.FrameNr
    } else {
        -1
    };

    ctx.signal_frame_ready(frame_nr);
}
