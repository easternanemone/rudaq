## Summary

<!-- Brief description of what this PR does and why -->

## Debt Prevention Checklist

<!-- Check all that apply. PRs that introduce new debt must justify each item. -->

- [ ] No new `.unwrap()` / `.expect()` in library code (use `?` with `.context()`)
- [ ] No `panic!()` / `unreachable!()` in library code (return `Err(...)` instead)
- [ ] No `std::thread::sleep()` in async code (use `tokio::time::sleep()`)
- [ ] No `#[allow(...)]` without rationale comment
- [ ] No `TODO` / `FIXME` without beads issue reference (e.g., `TODO(bd-XXXX)`)
- [ ] No calls to `#[deprecated]` items (use replacement from deprecation note)
- [ ] No `unsafe {}` without `// SAFETY:` comment

## Test Plan

<!-- How was this tested? -->

- [ ] `cargo nextest run` passes
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --all-targets` clean (no new warnings)
