use anyhow::Result;
#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_hardware")]
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
                unsafe {
                    if pl_set_param(h, PARAM_READOUT_PORT, &p_val as *const _ as *mut _) == 0 {
                        continue; // Skip invalid ports
                    }
                }
                let p_name = get_enum_name(h, PARAM_READOUT_PORT, p_idx)?;

                let mut speeds = Vec::new();
                // 3. Iterate Speeds for this Port
                if let Ok(speed_count) = PvcamFeatures::get_enum_count_impl(h, PARAM_SPDTAB_INDEX) {
                    for s_idx in 0..speed_count {
                        let s_val = s_idx as i32;
                        unsafe {
                            if pl_set_param(h, PARAM_SPDTAB_INDEX, &s_val as *const _ as *mut _)
                                == 0
                            {
                                continue;
                            }
                        }
                        let s_name = get_enum_name(h, PARAM_SPDTAB_INDEX, s_idx)?;
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
                                let g_name = get_enum_name(h, PARAM_GAIN_INDEX, g_idx)?;
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
            unsafe {
                let p = orig_port as i32;
                if pl_set_param(h, PARAM_READOUT_PORT, &p as *const _ as *mut _) == 0 {
                    tracing::warn!(
                        "Failed to restore original PARAM_READOUT_PORT to {} after building SpeedTable",
                        orig_port
                    );
                }
                let s = orig_speed as i32;
                if pl_set_param(h, PARAM_SPDTAB_INDEX, &s as *const _ as *mut _) == 0 {
                    tracing::warn!(
                        "Failed to restore original PARAM_SPDTAB_INDEX to {} after building SpeedTable",
                        orig_speed
                    );
                }
                let g = orig_gain as i32;
                if pl_set_param(h, PARAM_GAIN_INDEX, &g as *const _ as *mut _) == 0 {
                    tracing::warn!(
                        "Failed to restore original PARAM_GAIN_INDEX to {} after building SpeedTable",
                        orig_gain
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
#[cfg(feature = "pvcam_hardware")]
fn get_enum_name(h: i16, param: u32, index: u32) -> Result<String> {
    let mut name = [0i8; 256];
    let mut name_len: u32 = 256;
    let mut value: i32 = 0;
    unsafe {
        if pl_enum_str_length(h, param, index, &mut name_len) != 0 {
            if pl_get_enum_param(
                h,
                param,
                index,
                &mut value,
                name.as_mut_ptr(),
                name_len.min(256),
            ) != 0
            {
                return Ok(CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned());
            }
        }
    }
    // Fallback if SDK call fails (shouldn't happen for valid indices)
    Ok(format!("Index {}", index))
}
