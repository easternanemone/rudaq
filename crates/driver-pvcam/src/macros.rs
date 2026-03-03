//! Declarative macros for reducing PVCAM parameter registration boilerplate.
//!
//! The `pvcam_parameters!` macro generates `Parameter<T>` instances with their
//! constraints, descriptions, and units, then registers them into a `ParameterSet`.
//! Hardware callbacks (which depend on FFI handles) are intentionally left out --
//! they must be wired up separately after construction.
//!
//! # Supported parameter kinds
//!
//! | Kind      | Rust type   | Constraints                       |
//! |-----------|-------------|-----------------------------------|
//! | `float`   | `f64`       | Optional range (introspectable)   |
//! | `int`     | `i64`       | Optional range (introspectable)   |
//! | `uint32`  | `u32`       | Optional range                    |
//! | `uint16`  | `u16`       | None (value-only)                 |
//! | `int16`   | `i16`       | None (value-only)                 |
//! | `bool`    | `bool`      | None                              |
//! | `enum`    | `String`    | Introspectable choices list       |

/// Generate `Parameter<T>` fields, build them with constraints, and register
/// them into a `ParameterSet`.
///
/// # Syntax
///
/// ```rust,ignore
/// pvcam_parameters! {
///     params,  // ParameterSet binding to register into
///
///     // Float with range
///     float exposure_ms {
///         name: "acquisition.exposure_ms",
///         default: 100.0,
///         description: "Exposure time",
///         unit: "ms",
///         range: (0.1, 60000.0),
///     }
///
///     // Read-only float
///     float temperature {
///         name: "thermal.temperature",
///         default: 0.0,
///         description: "Current sensor temperature",
///         unit: "C",
///         read_only: true,
///     }
///
///     // Read-only u32
///     uint32 readout_time_us {
///         name: "acquisition.readout_time_us",
///         default: 0u32,
///         description: "Readout time",
///         unit: "us",
///         read_only: true,
///     }
///
///     // Boolean
///     bool armed {
///         name: "acquisition.armed",
///         default: false,
///         description: "Camera armed for trigger",
///         read_only: true,
///     }
///
///     // Enum (String with choices)
///     enum fan_speed {
///         name: "thermal.fan_speed",
///         default: FanSpeed::High.as_str().to_string(),
///         description: "Cooling fan speed",
///         choices: FanSpeed::all_choices(),
///     }
/// }
/// ```
///
/// This expands to a sequence of `let <ident> = Parameter::new(...) ...;`
/// bindings followed by `params.register(<ident>.clone());` calls for each
/// parameter. The caller can then use each binding directly (e.g. to wire
/// up hardware callbacks or store in struct fields).
macro_rules! pvcam_parameters {
    // Entry point: collect the ParameterSet ident, then process items
    ($params:ident, $( $kind:ident $field:ident { $($body:tt)* } )* ) => {
        $(
            pvcam_parameters!(@single $params, $kind $field { $($body)* });
        )*
    };

    // =====================================================================
    // float — Parameter<f64>
    // =====================================================================
    (@single $params:ident, float $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
        unit: $unit:expr,
        range: ($min:expr, $max:expr),
        $(read_only: $ro:expr,)?
    }) => {
        let $field = Parameter::new($name, $default)
            .with_description($desc)
            .with_unit($unit)
            .with_range_introspectable($min, $max);
        $( let $field = if $ro { $field.read_only() } else { $field }; )?
        $params.register($field.clone());
    };

    // float without range
    (@single $params:ident, float $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
        unit: $unit:expr,
        read_only: $ro:expr,
    }) => {
        let $field = {
            let p = Parameter::new($name, $default)
                .with_description($desc)
                .with_unit($unit)
                .with_dtype("float");
            if $ro { p.read_only() } else { p }
        };
        $params.register($field.clone());
    };

    // =====================================================================
    // uint32 — Parameter<u32>
    // =====================================================================
    (@single $params:ident, uint32 $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
        unit: $unit:expr,
        read_only: $ro:expr,
    }) => {
        let $field = {
            let p = Parameter::new($name, $default)
                .with_description($desc)
                .with_unit($unit);
            if $ro { p.read_only() } else { p }
        };
        $params.register($field.clone());
    };

    // uint32 with range (writable)
    (@single $params:ident, uint32 $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
        range: ($min:expr, $max:expr),
    }) => {
        let $field = Parameter::new($name, $default)
            .with_description($desc)
            .with_range($min, $max)
            .with_dtype("int");
        $params.register($field.clone());
    };

    // uint32 with unit, no read_only (writable with unit)
    (@single $params:ident, uint32 $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
        unit: $unit:expr,
    }) => {
        let $field = Parameter::new($name, $default)
            .with_description($desc)
            .with_unit($unit);
        $params.register($field.clone());
    };

    // =====================================================================
    // uint16 — Parameter<u16>
    // =====================================================================
    (@single $params:ident, uint16 $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
        read_only: $ro:expr,
    }) => {
        let $field = {
            let p = Parameter::new($name, $default)
                .with_description($desc);
            if $ro { p.read_only() } else { p }
        };
        $params.register($field.clone());
    };

    // uint16 — writable, no read_only key
    (@single $params:ident, uint16 $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
    }) => {
        let $field = Parameter::new($name, $default)
            .with_description($desc);
        $params.register($field.clone());
    };

    // =====================================================================
    // int16 — Parameter<i16>
    // =====================================================================
    (@single $params:ident, int16 $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
    }) => {
        let $field = Parameter::new($name, $default)
            .with_description($desc);
        $params.register($field.clone());
    };

    // =====================================================================
    // bool — Parameter<bool>
    // =====================================================================
    (@single $params:ident, bool $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
        read_only: $ro:expr,
    }) => {
        let $field = {
            let p = Parameter::new($name, $default)
                .with_description($desc)
                .with_dtype("bool");
            if $ro { p.read_only() } else { p }
        };
        $params.register($field.clone());
    };

    // bool — writable (no read_only key)
    (@single $params:ident, bool $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
    }) => {
        let $field = Parameter::new($name, $default)
            .with_description($desc)
            .with_dtype("bool");
        $params.register($field.clone());
    };

    // =====================================================================
    // enum — Parameter<String> with choices
    // =====================================================================
    (@single $params:ident, enum $field:ident {
        name: $name:expr,
        default: $default:expr,
        description: $desc:expr,
        choices: $choices:expr,
        $(read_only: $ro:expr,)?
    }) => {
        let $field = Parameter::new($name, $default)
            .with_description($desc)
            .with_choices_introspectable($choices);
        $( let $field = if $ro { $field.read_only() } else { $field }; )?
        $params.register($field.clone());
    };
}

// The macro is made available within the crate via `#[macro_use]` on the
// module declaration in lib.rs. No `pub(crate) use` needed.

#[cfg(test)]
mod tests {
    use common::observable::ParameterSet;
    use common::parameter::Parameter;

    #[test]
    fn test_float_parameter_with_range() {
        let mut params = ParameterSet::new();
        pvcam_parameters! {
            params,
            float exposure_ms {
                name: "acquisition.exposure_ms",
                default: 100.0,
                description: "Exposure time",
                unit: "ms",
                range: (0.1, 60000.0),
            }
        }
        assert_eq!(exposure_ms.get(), 100.0);
        assert_eq!(exposure_ms.name(), "acquisition.exposure_ms");
        assert_eq!(exposure_ms.unit(), Some("ms".to_string()));
        assert!(params.get("acquisition.exposure_ms").is_some());
    }

    #[test]
    fn test_float_parameter_read_only() {
        let mut params = ParameterSet::new();
        pvcam_parameters! {
            params,
            float temperature {
                name: "thermal.temperature",
                default: 0.0,
                description: "Current sensor temperature",
                unit: "C",
                read_only: true,
            }
        }
        assert_eq!(temperature.get(), 0.0);
        assert!(temperature.is_read_only());
        assert!(params.get("thermal.temperature").is_some());
    }

    #[test]
    fn test_uint32_parameter_read_only() {
        let mut params = ParameterSet::new();
        pvcam_parameters! {
            params,
            uint32 readout_time_us {
                name: "acquisition.readout_time_us",
                default: 0u32,
                description: "Readout time",
                unit: "us",
                read_only: true,
            }
        }
        assert_eq!(readout_time_us.get(), 0u32);
        assert!(readout_time_us.is_read_only());
    }

    #[test]
    fn test_uint16_parameter_read_only() {
        let mut params = ParameterSet::new();
        pvcam_parameters! {
            params,
            uint16 pre_mask {
                name: "readout.pre_mask",
                default: 0u16,
                description: "Pre-mask pixels",
                read_only: true,
            }
        }
        assert_eq!(pre_mask.get(), 0u16);
        assert!(pre_mask.is_read_only());
    }

    #[test]
    fn test_bool_parameter() {
        let mut params = ParameterSet::new();
        pvcam_parameters! {
            params,
            bool armed {
                name: "acquisition.armed",
                default: false,
                description: "Camera armed for trigger",
                read_only: true,
            }
        }
        assert!(!armed.get());
        assert!(armed.is_read_only());
        assert!(params.get("acquisition.armed").is_some());
    }

    #[test]
    fn test_enum_parameter() {
        let mut params = ParameterSet::new();
        pvcam_parameters! {
            params,
            enum fan_speed {
                name: "thermal.fan_speed",
                default: "High".to_string(),
                description: "Cooling fan speed",
                choices: vec!["High".into(), "Medium".into(), "Low".into(), "Off".into()],
            }
        }
        assert_eq!(fan_speed.get(), "High");
        assert!(params.get("thermal.fan_speed").is_some());
    }

    #[test]
    fn test_multiple_parameters() {
        let mut params = ParameterSet::new();
        pvcam_parameters! {
            params,

            float exposure_ms {
                name: "acquisition.exposure_ms",
                default: 100.0,
                description: "Exposure time",
                unit: "ms",
                range: (0.1, 60000.0),
            }

            bool streaming {
                name: "acquisition.streaming",
                default: false,
                description: "Camera streaming state",
                read_only: true,
            }

            uint32 readout_time_us {
                name: "acquisition.readout_time_us",
                default: 0u32,
                description: "Readout time",
                unit: "us",
                read_only: true,
            }

            enum trigger_mode {
                name: "acquisition.trigger_mode",
                default: "Timed".to_string(),
                description: "Trigger mode",
                choices: vec!["Timed".into(), "Strobe".into(), "Bulb".into()],
            }

            uint16 bit_depth {
                name: "info.bit_depth",
                default: 16u16,
                description: "ADC Bit Depth",
                read_only: true,
            }
        }

        assert_eq!(exposure_ms.get(), 100.0);
        assert!(!streaming.get());
        assert_eq!(readout_time_us.get(), 0u32);
        assert_eq!(trigger_mode.get(), "Timed");
        assert_eq!(bit_depth.get(), 16u16);
        assert!(params.get("acquisition.exposure_ms").is_some());
        assert!(params.get("acquisition.streaming").is_some());
        assert!(params.get("acquisition.readout_time_us").is_some());
        assert!(params.get("acquisition.trigger_mode").is_some());
        assert!(params.get("info.bit_depth").is_some());
    }
}
