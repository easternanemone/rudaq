//! Curated evalexpr helpers for manifest conversion formulas.
//!
//! Device manifests declare numeric conversions via `[conversions.*] formula`
//! which is evaluated with [`evalexpr`]. By default evalexpr only exposes
//! arithmetic and a small stdlib. For undergrad-authored manifests it's
//! clearer to write `to_pulses(degrees, pulses_per_degree)` than to remember
//! whether `round()` was registered or not.
//!
//! This module installs a curated palette on any [`HashMapContext`]:
//!
//! | Name                                  | Arity | Returns | Meaning                              |
//! |---------------------------------------|-------|---------|--------------------------------------|
//! | `round(v)`                            | 1     | Float   | Round to nearest integer (keeps Float) |
//! | `to_pulses(value, pulses_per_unit)`   | 2     | Int     | `round(value * pulses_per_unit)`     |
//! | `from_pulses(pulses, pulses_per_unit)`| 2     | Float   | `pulses / pulses_per_unit`           |
//! | `clamp(value, min, max)`              | 3     | Float   | Clamp to `[min, max]`                |
//! | `scale(value, k)`                     | 2     | Float   | `value * k` — cosmetic, self-docs    |
//! | `offset(value, d)`                    | 2     | Float   | `value + d` — cosmetic, self-docs    |
//!
//! Raw evalexpr still works as an escape hatch: if a manifest needs an
//! operation not on this list, a plain expression like `(v - min) / (max - min)`
//! is unchanged.

use evalexpr::{EvalexprError, Function, HashMapContext, Value};

/// Install all curated helpers on the given context.
///
/// Returns the first error encountered during registration, if any.
pub fn install_helpers(ctx: &mut HashMapContext) -> Result<(), EvalexprError> {
    use evalexpr::ContextWithMutableFunctions;

    ctx.set_function("round".to_string(), Function::new(fn_round))?;
    ctx.set_function("to_pulses".to_string(), Function::new(fn_to_pulses))?;
    ctx.set_function("from_pulses".to_string(), Function::new(fn_from_pulses))?;
    ctx.set_function("clamp".to_string(), Function::new(fn_clamp))?;
    ctx.set_function("scale".to_string(), Function::new(fn_scale))?;
    ctx.set_function("offset".to_string(), Function::new(fn_offset))?;
    Ok(())
}

/// Coerce a numeric [`Value`] to `f64`; error otherwise.
fn as_f64(v: &Value) -> Result<f64, EvalexprError> {
    match v {
        Value::Float(f) => Ok(*f),
        #[allow(clippy::cast_precision_loss)]
        Value::Int(i) => Ok(*i as f64),
        other => Err(EvalexprError::expected_number(other.clone())),
    }
}

/// Destructure a tuple arg into exactly `N` f64 values.
fn as_tuple_f64<const N: usize>(arg: &Value, fn_name: &str) -> Result<[f64; N], EvalexprError> {
    match arg {
        Value::Tuple(t) if t.len() == N => {
            let mut out = [0.0; N];
            for (i, v) in t.iter().enumerate() {
                out[i] = as_f64(v)?;
            }
            Ok(out)
        }
        _ => Err(EvalexprError::CustomMessage(format!(
            "{fn_name} expects {N} arguments"
        ))),
    }
}

fn fn_round(arg: &Value) -> Result<Value, EvalexprError> {
    match arg {
        Value::Float(f) => Ok(Value::Float(f.round())),
        Value::Int(i) => Ok(Value::Int(*i)),
        other => Err(EvalexprError::expected_number(other.clone())),
    }
}

fn fn_to_pulses(arg: &Value) -> Result<Value, EvalexprError> {
    let [v, k] = as_tuple_f64::<2>(arg, "to_pulses")?;
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: caller-supplied pulses_per_unit is bounded by mechanical reality.
    let rounded = (v * k).round() as i64;
    Ok(Value::Int(rounded))
}

fn fn_from_pulses(arg: &Value) -> Result<Value, EvalexprError> {
    let [pulses, per_unit] = as_tuple_f64::<2>(arg, "from_pulses")?;
    if per_unit == 0.0 {
        return Err(EvalexprError::CustomMessage(
            "from_pulses: pulses_per_unit must be non-zero".into(),
        ));
    }
    Ok(Value::Float(pulses / per_unit))
}

fn fn_clamp(arg: &Value) -> Result<Value, EvalexprError> {
    let [v, lo, hi] = as_tuple_f64::<3>(arg, "clamp")?;
    if lo > hi {
        return Err(EvalexprError::CustomMessage(format!(
            "clamp: min ({lo}) must be <= max ({hi})"
        )));
    }
    Ok(Value::Float(v.clamp(lo, hi)))
}

fn fn_scale(arg: &Value) -> Result<Value, EvalexprError> {
    let [v, k] = as_tuple_f64::<2>(arg, "scale")?;
    Ok(Value::Float(v * k))
}

fn fn_offset(arg: &Value) -> Result<Value, EvalexprError> {
    let [v, d] = as_tuple_f64::<2>(arg, "offset")?;
    Ok(Value::Float(v + d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evalexpr::{ContextWithMutableVariables, eval_with_context_mut};

    fn ctx_with_helpers() -> HashMapContext {
        let mut ctx = HashMapContext::new();
        install_helpers(&mut ctx).expect("helpers install");
        ctx
    }

    #[test]
    fn round_passes_through_int() {
        let mut ctx = ctx_with_helpers();
        let v = eval_with_context_mut("round(3)", &mut ctx).unwrap();
        assert_eq!(v, Value::Int(3));
    }

    #[test]
    fn round_rounds_float() {
        let mut ctx = ctx_with_helpers();
        let v = eval_with_context_mut("round(3.6)", &mut ctx).unwrap();
        assert_eq!(v, Value::Float(4.0));
    }

    #[test]
    fn to_pulses_matches_round_multiply() {
        let mut ctx = ctx_with_helpers();
        ctx.set_value("degrees".into(), Value::Float(30.0)).unwrap();
        ctx.set_value("pulses_per_degree".into(), Value::Float(398.22))
            .unwrap();
        let curated =
            eval_with_context_mut("to_pulses(degrees, pulses_per_degree)", &mut ctx).unwrap();
        let raw = eval_with_context_mut("round(degrees * pulses_per_degree)", &mut ctx).unwrap();
        let Value::Int(a_i) = curated else {
            panic!("expected int, got {curated:?}");
        };
        let Value::Float(b) = raw else {
            panic!("expected float, got {raw:?}");
        };
        #[allow(clippy::cast_precision_loss)]
        let a = a_i as f64;
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn from_pulses_inverts_to_pulses() {
        let mut ctx = ctx_with_helpers();
        ctx.set_value("pulses".into(), Value::Int(11946)).unwrap();
        ctx.set_value("ppd".into(), Value::Float(398.22)).unwrap();
        let v = eval_with_context_mut("from_pulses(pulses, ppd)", &mut ctx).unwrap();
        let Value::Float(f) = v else {
            panic!("expected float, got {v:?}");
        };
        assert!((f - 30.0).abs() < 0.01);
    }

    #[test]
    fn from_pulses_rejects_zero_divisor() {
        let mut ctx = ctx_with_helpers();
        let err = eval_with_context_mut("from_pulses(5.0, 0.0)", &mut ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non-zero"), "got: {msg}");
    }

    #[test]
    fn clamp_inside_range_unchanged() {
        let mut ctx = ctx_with_helpers();
        let v = eval_with_context_mut("clamp(5.0, 0.0, 10.0)", &mut ctx).unwrap();
        assert_eq!(v, Value::Float(5.0));
    }

    #[test]
    fn clamp_below_min_clamps_up() {
        let mut ctx = ctx_with_helpers();
        let v = eval_with_context_mut("clamp(-3.0, 0.0, 10.0)", &mut ctx).unwrap();
        assert_eq!(v, Value::Float(0.0));
    }

    #[test]
    fn clamp_above_max_clamps_down() {
        let mut ctx = ctx_with_helpers();
        let v = eval_with_context_mut("clamp(100.0, 0.0, 10.0)", &mut ctx).unwrap();
        assert_eq!(v, Value::Float(10.0));
    }

    #[test]
    fn clamp_rejects_inverted_bounds() {
        let mut ctx = ctx_with_helpers();
        let err = eval_with_context_mut("clamp(5.0, 10.0, 0.0)", &mut ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be <="), "got: {msg}");
    }

    #[test]
    fn scale_is_multiply() {
        let mut ctx = ctx_with_helpers();
        let v = eval_with_context_mut("scale(2.0, 3.0)", &mut ctx).unwrap();
        assert_eq!(v, Value::Float(6.0));
    }

    #[test]
    fn offset_is_add() {
        let mut ctx = ctx_with_helpers();
        let v = eval_with_context_mut("offset(2.0, 3.0)", &mut ctx).unwrap();
        assert_eq!(v, Value::Float(5.0));
    }

    #[test]
    fn helpers_compose_in_a_single_expression() {
        let mut ctx = ctx_with_helpers();
        // clamp a scaled value, then offset it.
        let v = eval_with_context_mut("offset(clamp(scale(4.0, 3.0), 0.0, 10.0), 1.0)", &mut ctx)
            .unwrap();
        // scale(4,3)=12 -> clamp(12,0,10)=10 -> offset(10,1)=11
        assert_eq!(v, Value::Float(11.0));
    }

    #[test]
    fn wrong_arity_gives_clear_error() {
        let mut ctx = ctx_with_helpers();
        let err = eval_with_context_mut("to_pulses(1.0)", &mut ctx).unwrap_err();
        let msg = err.to_string();
        // evalexpr passes single args as the value itself, not a 1-tuple;
        // our helper detects the shape mismatch.
        assert!(msg.contains("to_pulses"), "got: {msg}");
    }

    #[test]
    fn mixed_int_and_float_args_accepted() {
        let mut ctx = ctx_with_helpers();
        // Int `*` Float must work the same as Float `*` Float.
        let v = eval_with_context_mut("to_pulses(30, 398.22)", &mut ctx).unwrap();
        let Value::Int(i) = v else {
            panic!("expected int, got {v:?}");
        };
        // 30 * 398.22 = 11946.6 → round = 11947
        assert_eq!(i, 11947);
    }
}
