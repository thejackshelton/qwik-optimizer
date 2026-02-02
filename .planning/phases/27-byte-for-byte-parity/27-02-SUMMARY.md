---
phase: 27-byte-for-byte-parity
plan: 02
subsystem: optimizer
tags: [_fnSignal, inlined_fn, jsx, reactivity]

# Dependency graph
requires:
  - phase: 26-exact-snapshot-parity
    provides: QRL hoisting, inlinedQrl generation, _fnSignal inside loops
provides:
  - _fnSignal generation outside of loops
  - Improved hoisted function capture filtering
affects: [27-03, 27-04, 27-05]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "get_used_as_object_idents - collect only identifiers used as member expression objects"
    - "scoped_idents collection from decl_stack for non-loop contexts"

key-files:
  created: []
  modified:
    - optimizer/src/transform/jsx/attribute.rs
    - optimizer/src/inlined_fn.rs

key-decisions:
  - "Use decl_stack to collect scoped variables when not inside loops"
  - "Filter captures to only include identifiers used as member expression objects"
  - "Remaining 23 hoisted function mismatches due to props destructuring complexity"

patterns-established:
  - "ObjectUsageCollector - visitor pattern to collect used-as-object identifiers"
  - "actual_captures filtering in convert_inlined_fn"

# Metrics
duration: 15min
completed: 2026-02-02
---

# Phase 27 Plan 02: _fnSignal Expansion Summary

**Expanded _fnSignal generation to cover signal/store property access outside of loops, reducing hoisted function mismatches from 24 to 23**

## Performance

- **Duration:** 15 min
- **Started:** 2026-02-02T00:58:05Z
- **Completed:** 2026-02-02T01:12:52Z
- **Tasks:** 3
- **Files modified:** 4

## Accomplishments
- Removed `loop_depth > 0` restriction for _fnSignal wrapping
- Added `get_used_as_object_idents` function to filter captures to only variables used as member expression objects
- Hoisted function placement mismatches reduced from 24 to 23
- All 278 lib tests and 6 snapshot verification tests pass

## Task Commits

Each task was committed atomically:

1. **Task 1: Analyze the 24 affected snapshots** - Analysis only, no commit needed
2. **Task 2: Remove loop_depth restriction** - `9b5ea3c` (feat)
3. **Task 3: Update snapshots** - `8607bfb` (test)

## Files Created/Modified
- `optimizer/src/transform/jsx/attribute.rs` - Expanded _fnSignal logic to work outside loops
- `optimizer/src/inlined_fn.rs` - Added get_used_as_object_idents for capture filtering
- `optimizer/src/snapshots/*` - 2 snapshots updated

## Decisions Made
- **Scoped idents collection:** When not in a loop, collect all declared variables from `decl_stack` with `IdentType::Var(_)`
- **Capture filtering:** Only include identifiers actually used as objects of member expressions in the hoisted function parameters
- **Partial completion accepted:** 23 mismatches remain due to props destructuring complexity (see below)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Capture array included all scoped variables**
- **Found during:** Task 2 (Testing _fnSignal expansion)
- **Issue:** Initial implementation captured ALL scoped variables as parameters, producing incorrect output like `_hf0 = (p0, p1, p2, p3)=>p0.id`
- **Fix:** Added `get_used_as_object_idents` to collect only identifiers used as member expression objects
- **Files modified:** optimizer/src/inlined_fn.rs
- **Verification:** Output now shows `_hf0 = (p0)=>p0.id` with only necessary captures
- **Committed in:** 9b5ea3c (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Essential fix for correct capture array generation. No scope creep.

## Issues Encountered

**23 remaining hoisted function mismatches:**
The plan expected to eliminate all 24 mismatches, but 23 remain. Root cause analysis:

1. **Props destructuring transformation:** qwik-core converts destructured props like `fromProps` to `_rawProps.fromProps` access BEFORE checking for _fnSignal wrapping
2. **Current OXC approach:** Does not transform props identifiers to member access patterns
3. **Example difference:**
   - Input: `computed={fromLocal + fromProps}`
   - qwik-core: `computed: _fnSignal(_hf0, [_rawProps, fromLocal], _hf0_str)` with `_hf0 = (p0, p1)=>p1 + p0.fromProps`
   - OXC: `computed: fromLocal + fromProps` (no _fnSignal because `fromProps` is not detected as member access)

**Resolution:** This is a more complex architectural issue requiring props awareness in the _fnSignal generation logic. The remaining mismatches are documented for future work.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- _fnSignal now triggers outside of loops for member access patterns
- Props-related _fnSignal wrapping deferred - requires props identifier transformation
- 23 hoisted function placement differences documented

---
*Phase: 27-byte-for-byte-parity*
*Completed: 2026-02-02*
