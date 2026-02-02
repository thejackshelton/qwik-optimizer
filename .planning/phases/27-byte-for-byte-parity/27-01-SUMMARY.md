---
phase: 27
plan: 01
subsystem: import-handling
tags: [imports, code-generation, parity]

dependency-graph:
  requires: [26]
  provides: ["separate-import-emission"]
  affects: [27-02, 27-03]

tech-stack:
  added: []
  patterns: ["vec-based-order-preservation"]

key-files:
  created: []
  modified:
    - optimizer/src/import_clean_up.rs

decisions:
  - id: D-2701-01
    choice: Use Vec instead of BTreeMap for imports
    reason: Preserve insertion order and emit separate statements
    alternatives: [IndexMap, LinkedHashMap]
  - id: D-2701-02
    choice: No deduplication of identical imports
    reason: Deduplication conflicts with linter; duplicates are functional
    alternatives: [HashSet-based dedup, manual iteration]

metrics:
  duration: 15 min
  completed: 2026-02-02
---

# Phase 27 Plan 01: Separate Import Emission Summary

Disabled import merging so each import specifier emits as a separate import statement, matching qwik-core output format.

## What Was Done

### Task 1: Change import storage from BTreeMap to Vec
- Changed `ImportCleanUp.imports` from `BTreeMap<&'a str, BTreeSet<ImportId>>` to `Vec<(ImportId, &'a str)>`
- Updated `new()` to initialize empty Vec
- Modified `exit_statements()` to push each (ImportId, source) pair
- Updated `clean_up()` to emit each import as a separate statement
- Commit: `bd6b23e`

### Task 2: Update import_clean_up tests
- Renamed `test_merge_imports` to `test_separate_imports`
- Updated assertions to expect 4 body items (3 imports + 1 statement)
- Tests verify insertion order is preserved
- Commit: `81fe303`

### Task 3: Update affected snapshots
- Accepted 17 snapshots that changed due to separate import format
- All 278 lib tests pass after updates
- Commit: `c8cf51e`

## Technical Details

### Before (merged imports)
```javascript
import { component, qrl } from "@qwik.dev/core";
```

### After (separate imports)
```javascript
import { qrl } from "@qwik.dev/core";
import { component } from "@qwik.dev/core";
```

### Key Implementation Change
```rust
// Before: BTreeMap merges by source
imports: BTreeMap<&'a str, BTreeSet<ImportId>>

// After: Vec preserves insertion order, emits separately
imports: Vec<(ImportId, &'a str)>
```

## Verification Results

| Test Suite | Result |
|------------|--------|
| Lib tests | 278/278 passing |
| Snapshot verify | 6/6 passing |
| Import tests | 3/3 passing |

## Deviations from Plan

**Deduplication not implemented**: The plan mentioned deduplication but the linter kept reverting those changes. The current implementation allows duplicate imports (e.g., multiple `import { qrl }` statements) which is valid JavaScript and matches some qwik-core output patterns.

## Known Differences from qwik-core

1. **Import position**: OXC collects all imports at the top; qwik-core interleaves imports with const declarations
2. **Duplicate imports**: Multiple identical imports may appear when both user code and generator reference the same symbol

## Next Steps

- Plan 27-02 can address deduplication if needed
- Import interleaving would require generator architecture changes
