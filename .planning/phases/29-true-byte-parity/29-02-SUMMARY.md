# Phase 29 Plan 02: Fix QRL Declaration Style Detection Summary

**One-liner:** Fixed false positive QRL style mismatch detection by correcting pattern comparison logic and metadata stripping

## Results

| Metric | Before | After | Status |
|--------|--------|-------|--------|
| QRL style false positive | 1 (example_functional_component_2) | 0 | FIXED |
| QRL style REAL differences | Hidden (reported as 1) | 11 (correctly detected) | REVEALED |
| Detection accuracy | False positives | Accurate | IMPROVED |

## Investigation Findings

### The "1 Mismatch" Was a False Positive

The Phase 28-02 summary correctly identified that `example_functional_component_2` was a false positive:
- Both OXC and qwik-core use the SAME QRL pattern
- Outer div handler: inline qrl() in JSX
- Inner map button handler: hoisted const qrl() before return

### Root Causes of False Positive

**Issue 1: Incorrect Mismatch Logic**
```rust
// OLD (wrong): flagged mismatch when both have BOTH types
(oxc_has_inlined_qrls && qwik_has_hoisted_qrls)
    || (oxc_has_hoisted_qrls && qwik_has_inlined_qrls)

// NEW (correct): mismatch only if pattern presence differs
(oxc_has_inlined_qrls != qwik_has_inlined_qrls)
    || (oxc_has_hoisted_qrls != qwik_has_hoisted_qrls)
```

**Issue 2: PURE Comments Treated as Metadata**
```javascript
// This line was incorrectly entering metadata-stripping mode:
/*#__PURE__*/ _jsxSorted("button", null, {
//            ^-- the { was being detected as metadata block start
```

Fix: Check for `#__PURE__` and `*/` on the same line before entering metadata mode.

## Changes Made

### optimizer/tests/snapshot_verify.rs

1. **Fixed `has_qrl_placement_mismatch()`** (lines 278-294)
   - Changed from cross-check to pattern-presence comparison
   - Both having inline AND hoisted = NO mismatch (same pattern)
   - One having inline-only vs other having hoisted-only = mismatch

2. **Fixed `strip_metadata_comments()`** (lines 182-261)
   - Added check for `#__PURE__` to avoid treating PURE comments as metadata
   - Added check for `*/` on same line (PURE comments are complete on one line)
   - Added documentation explaining the difference between PURE and metadata comments

## Verification

```bash
# Lib tests pass
cargo test --lib -p qwik-optimizer  # 278 passed

# Unit tests pass
cargo test --test snapshot_verify -- test_compare_file_structures  # ok

# example_functional_component_2 no longer flagged as QRL mismatch
SKIP_PARITY_CHECK=1 cargo test --test snapshot_verify -- --nocapture
# QRL DECLARATION STYLE (11 snapshots) - does NOT include example_functional_component_2
```

## Current State

After this fix:
- **Hoisted function placement:** 18 snapshots (unchanged)
- **QRL declaration style:** 11 snapshots (REAL differences, were hidden before)
- **Attribute quoting:** 0 snapshots (unchanged)
- **Segment count differences:** 73 snapshots (unchanged)

The 11 QRL style differences are REAL code differences that need actual code fixes:
- should_not_transform_events_on_non_elements
- example_strip_server_code
- should_merge_bind_value_and_on_input
- example_manual_chunks
- example_jsx_import_source
- example_use_server_mount
- should_merge_on_input_and_bind_checked
- should_merge_bind_checked_and_on_input
- should_merge_on_input_and_bind_value
- example_dev_mode
- example_drop_side_effects

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Fix detection logic, not code | The 1 reported mismatch was a false positive |
| Reveal hidden differences | Better to accurately report 11 REAL differences than hide them |
| Use pattern-presence comparison | Correctly identifies when BOTH have inline+hoisted vs one having only one type |

## Commits

- `3d163d7`: fix(29-02): correct QRL style mismatch false positive detection

## Duration

~45 minutes

## Next Steps

The 11 real QRL style differences may need investigation:
- Are they acceptable architectural differences?
- Do they need code fixes to match qwik-core exactly?

This should be addressed in Phase 29 subsequent plans or marked as acceptable differences.
