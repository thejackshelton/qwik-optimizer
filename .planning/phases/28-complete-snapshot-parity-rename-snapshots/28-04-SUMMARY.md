---
phase: 28
plan: 04
subsystem: testing-infrastructure
tags: [insta, snapshots, naming, macros]

dependency-graph:
  requires: ["28-01", "28-02", "28-03"]
  provides: ["short snapshot naming", "readable snapshot files"]
  affects: []

tech-stack:
  added: []
  patterns: ["insta with_settings! macro", "explicit snapshot naming"]

key-files:
  created: []
  modified:
    - optimizer/src/macros.rs
    - optimizer/src/spec_parity_tests.rs
    - optimizer/tests/snapshot_verify.rs
    - optimizer/src/snapshots/* (163 files renamed)

decisions:
  - id: use-with-settings
    summary: "Use insta::with_settings! macro instead of insta.toml"
    rationale: "insta.toml setting was not working with assert_snapshot!(name, value) form"
  - id: explicit-snapshot-names
    summary: "Pass explicit snapshot name (func_name) to snapshot_res! macro"
    rationale: "Required for insta to use short names without module path prefix"

metrics:
  duration: ~25 min
  completed: 2026-02-01
---

# Phase 28 Plan 04: Rename Snapshots Summary

Renamed all 163 snapshots from verbose module path format to short readable format.

## What Changed

### 1. Macro Updates (macros.rs)

Added three-parameter variant of `snapshot_res!` macro with explicit name support:

```rust
#[macro_export]
macro_rules! snapshot_res {
    ($res: expr, $prefix: expr, $name: expr) => {
        match $res {
            Ok(v) => {
                // ... build output ...
                insta::with_settings!({prepend_module_to_snapshot => false}, {
                    insta::assert_snapshot!($name, output);
                });
            }
            Err(err) => {
                insta::with_settings!({prepend_module_to_snapshot => false}, {
                    insta::assert_snapshot!($name, err);
                });
            }
        }
    };
    // ... two-parameter variant for backward compatibility ...
}
```

### 2. Test Updates (spec_parity_tests.rs)

Updated `spec_test!` macro to pass explicit snapshot name:

```rust
crate::snapshot_res!(result, format!("==INPUT==\n\n{}", code.to_string()), func_name);
```

### 3. Verification Test Updates (snapshot_verify.rs)

Updated `extract_oxc_test_name` function:
- Old prefix: `qwik_optimizer__spec_parity_tests__tests__spec_`
- New prefix: `spec_`

### 4. Snapshot Renaming

All 163 snapshot files renamed:
- Old: `qwik_optimizer__spec_parity_tests__tests__spec_example_1.snap` (67+ chars)
- New: `spec_example_1.snap` (~20 chars)

## Results

| Metric | Before | After |
|--------|--------|-------|
| Filename length (avg) | 67 chars | 20 chars |
| Module prefix | Yes | No |
| Lib tests | 278 pass | 278 pass |
| Verification tests | 6 pass | 6 pass |

## Verification

- `cargo test --lib` - All 278 tests pass
- `cargo test --test snapshot_verify` - All 6 tests pass
- `ls optimizer/src/snapshots/ | head -5` shows short names

## Technical Notes

The `insta.toml` configuration file with `prepend_module_to_snapshot = false` did NOT work for the `assert_snapshot!(name, value)` form. The solution was to use `insta::with_settings!` directly in the macro.

## Commits

- 8d827d8: chore(28-04): add insta.toml to disable module prefix in snapshot names
- b43d87e: refactor(28-04): rename snapshots to short format (spec_*.snap)
- 0ea7a4b: (amended) removed insta.toml, using macro settings instead
- 204c2bd: fix(28-04): update snapshot_verify for short naming format

## Deviations from Plan

1. **insta.toml approach failed** - The configuration file setting was not being respected by insta. Had to use `insta::with_settings!` macro instead.

2. **Macro modification required** - The original plan expected insta.toml to work. Had to modify both `macros.rs` and `spec_parity_tests.rs` to pass explicit snapshot names with settings.

## Next Phase Readiness

Plan 28-05 (Final Verification) can proceed to document final parity status.
