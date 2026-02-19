#[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
use crate::components::connection::get_pvcam_error;
use anyhow::Result;
#[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
use pvcam_sys::*;
#[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
use std::ffi::CStr;

#[derive(Debug, Clone)]
pub struct GainEntry {
    pub index: i16,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct SpeedEntry {
    pub index: i16,
    pub name: String,
    pub pix_time_ns: u16,
    pub bit_depth: i16,
    pub gains: Vec<GainEntry>,
}

#[derive(Debug, Clone)]
pub struct PortEntry {
    pub value: i32,
    pub name: String,
    pub speeds: Vec<SpeedEntry>,
}

#[derive(Debug, Clone)]
pub struct SpeedTable {
    pub ports: Vec<PortEntry>,
}

impl SpeedTable {
    /// Build the speed table by probing the camera hardware.
    /// This is a slow operation that changes camera state!
    /// It must only be called during initialization.
    pub fn build(h: i16) -> Result<Self> {
        #[cfg(feature = "pvcam_hardware")]
        {
            use crate::components::features::PvcamFeatures;

            // 1. Save current state to restore later
            // We use get_u16_param_impl which returns u16, but SDK uses i16/i32 often.
            // param values are typically 0-based indices.
            let orig_port = PvcamFeatures::get_u16_param_impl(h, PARAM_READOUT_PORT).unwrap_or(0);
            let orig_speed = PvcamFeatures::get_u16_param_impl(h, PARAM_SPDTAB_INDEX).unwrap_or(0);
            let orig_gain = PvcamFeatures::get_u16_param_impl(h, PARAM_GAIN_INDEX).unwrap_or(0);

            let mut ports = Vec::new();

            // 2. Iterate Ports
            // We use a try block pattern or just standard Result propagation
            let port_count = PvcamFeatures::get_enum_count_impl(h, PARAM_READOUT_PORT)?;
            for p_idx in 0..port_count {
                // Set Port
                let p_val = p_idx as i32;
                // SAFETY: `h` is a valid camera handle from pl_cam_open().
                // `p_val` is a stack-allocated i32 whose address is valid for
                // the duration of this call. pl_set_param reads sizeof(i32)
                // bytes from the pointer per PVCAM SDK PARAM_READOUT_PORT docs.
                unsafe {
                    if pl_set_param(h, PARAM_READOUT_PORT, &p_val as *const _ as *mut _) == 0 {
                        continue; // Skip invalid ports
                    }
                }
                let p_name = get_enum_name(h, PARAM_READOUT_PORT, p_idx)
                    .unwrap_or_else(|_| format!("Port {}", p_idx));

                let mut speeds = Vec::new();
                // 3. Iterate Speeds for this Port
                if let Ok(speed_count) = PvcamFeatures::get_enum_count_impl(h, PARAM_SPDTAB_INDEX) {
                    for s_idx in 0..speed_count {
                        let s_val = s_idx as i32;
                        // SAFETY: `h` is a valid camera handle. `s_val` is a
                        // stack-allocated i32 valid for this call. The port has
                        // been set above, so PARAM_SPDTAB_INDEX is valid in
                        // this port context.
                        unsafe {
                            if pl_set_param(h, PARAM_SPDTAB_INDEX, &s_val as *const _ as *mut _)
                                == 0
                            {
                                continue;
                            }
                        }
                        let s_name = get_current_string_param(h, PARAM_SPDTAB_NAME)
                            .or_else(|| get_enum_name(h, PARAM_SPDTAB_INDEX, s_idx).ok())
                            .unwrap_or_else(|| format!("Speed {}", s_idx));
                        let pix_time = PvcamFeatures::get_u32_param_impl(h, PARAM_PIX_TIME)
                            .unwrap_or(0) as u16;
                        let bit_depth = PvcamFeatures::get_u16_param_impl(h, PARAM_BIT_DEPTH)
                            .unwrap_or(0) as i16;

                        let mut gains = Vec::new();
                        // 4. Iterate Gains for this Speed
                        if let Ok(gain_count) =
                            PvcamFeatures::get_enum_count_impl(h, PARAM_GAIN_INDEX)
                        {
                            for g_idx in 0..gain_count {
                                let g_val = g_idx as i32;
                                // SAFETY: h is valid and g_val points to a stack i32 for this call.
                                let set_ok = unsafe {
                                    pl_set_param(h, PARAM_GAIN_INDEX, &g_val as *const _ as *mut _)
                                        != 0
                                };
                                if !set_ok {
                                    continue;
                                }

                                let g_name = get_current_string_param(h, PARAM_GAIN_NAME)
                                    .or_else(|| get_enum_name(h, PARAM_GAIN_INDEX, g_idx).ok())
                                    .unwrap_or_else(|| format!("Gain {}", g_idx));
                                gains.push(GainEntry {
                                    index: g_idx as i16,
                                    name: g_name,
                                });
                            }
                        }

                        speeds.push(SpeedEntry {
                            index: s_idx as i16,
                            name: s_name,
                            pix_time_ns: pix_time,
                            bit_depth,
                            gains,
                        });
                    }
                }
                ports.push(PortEntry {
                    value: p_val,
                    name: p_name,
                    speeds,
                });
            }

            // 5. Restore original state
            // Dependency chain: Speed is defined within the selected Port, and Gain is defined within the selected Speed.
            // Therefore we must restore Port first, then Speed, then Gain so that each index is interpreted in the correct context.
            // SAFETY: `h` is a valid camera handle. All values (`p`, `s`, `g`)
            // are stack-allocated i32s valid for each call. Restore order is
            // Port→Speed→Gain because speed indices are port-relative and gain
            // indices are speed-relative. Original values were captured from
            // the same handle at function entry, so they are known-valid indices.
            unsafe {
                let p = orig_port as i32;
                if pl_set_param(h, PARAM_READOUT_PORT, &p as *const _ as *mut _) == 0 {
                    tracing::warn!(
                        "Failed to restore original PARAM_READOUT_PORT to {} after building SpeedTable: {}",
                        orig_port,
                        get_pvcam_error()
                    );
                }
                let s = orig_speed as i32;
                if pl_set_param(h, PARAM_SPDTAB_INDEX, &s as *const _ as *mut _) == 0 {
                    tracing::warn!(
                        "Failed to restore original PARAM_SPDTAB_INDEX to {} after building SpeedTable: {}",
                        orig_speed,
                        get_pvcam_error()
                    );
                }
                let g = orig_gain as i32;
                if pl_set_param(h, PARAM_GAIN_INDEX, &g as *const _ as *mut _) == 0 {
                    tracing::warn!(
                        "Failed to restore original PARAM_GAIN_INDEX to {} after building SpeedTable: {}",
                        orig_gain,
                        get_pvcam_error()
                    );
                }
            }

            Ok(SpeedTable { ports })
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            // Mock implementation
            let _ = h; // suppress unused var
            Ok(SpeedTable {
                ports: vec![PortEntry {
                    value: 0,
                    name: "Normal Port".to_string(),
                    speeds: vec![
                        SpeedEntry {
                            index: 0,
                            name: "100 MHz".to_string(),
                            pix_time_ns: 10,
                            bit_depth: 16,
                            gains: vec![
                                GainEntry {
                                    index: 0,
                                    name: "High Gain".to_string(),
                                },
                                GainEntry {
                                    index: 1,
                                    name: "Low Gain".to_string(),
                                },
                            ],
                        },
                        SpeedEntry {
                            index: 1,
                            name: "50 MHz".to_string(),
                            pix_time_ns: 20,
                            bit_depth: 12,
                            gains: vec![GainEntry {
                                index: 0,
                                name: "Medium Gain".to_string(),
                            }],
                        },
                    ],
                }],
            })
        }
    }
}

// Helper to get enum name by index
#[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
fn get_enum_name(h: i16, param: u32, index: u32) -> Result<String> {
    let mut name = [0i8; 256];
    let mut name_len: u32 = 0;
    let mut value: i32 = 0;
    // SAFETY: `h` is a valid camera handle. `param` and `index` are validated
    // by the caller (iterating 0..count from SDK). `name` is a stack-allocated
    // [i8; 256] buffer. We cap `name_len` at 256 to prevent buffer overflow.
    // pl_get_enum_param null-terminates the output per SDK docs, making
    // CStr::from_ptr safe.
    unsafe {
        if pl_enum_str_length(h, param, index, &mut name_len) != 0 {
            if pl_get_enum_param(
                h,
                param,
                index,
                &mut value,
                name.as_mut_ptr(),
                name_len.max(2).min(256),
            ) != 0
            {
                return Ok(CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned());
            }
        }
    }
    Err(anyhow::anyhow!(
        "Failed to get enum name for param {} index {}: {}",
        param,
        index,
        get_pvcam_error()
    ))
}

#[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
fn get_current_string_param(h: i16, param: u32) -> Option<String> {
    let mut buf = [0i8; 256];
    // SAFETY: h is valid and buf is a writable stack buffer for ATTR_CURRENT string params.
    let ok = unsafe { pl_get_param(h, param, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) != 0 };
    if !ok {
        return None;
    }
    // SAFETY: PVCAM string params are NUL-terminated.
    let s = unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
