# kto E2E Test Suite

End-to-end tests that validate kto's change detection accuracy using a local deterministic test server.

## Quick Start

```bash
# Run all tests
python3 tests/e2e/run_suite.py

# Run specific scenario
python3 tests/e2e/run_suite.py --scenario price

# Verbose output for failures
python3 tests/e2e/run_suite.py --verbose

# Keep server running (for debugging)
python3 tests/e2e/run_suite.py --keep-server
```

## What This Tests

Unlike smoke tests that only verify "watch created successfully", this suite validates:

1. **True Positives** - Real changes are detected (price drops, stock changes, new releases)
2. **True Negatives** - Non-changes don't trigger (static content, noise)
3. **Error Handling** - Graceful behavior on 403, 500, timeouts
4. **Idempotence** - Repeated runs don't cause false positives
5. **State Management** - Changes alert once, not repeatedly

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Test Server (localhost:8787)                               │
│  ├── /product-clean  → Price + stock (minimal)              │
│  ├── /product        → With noise (timestamps, tracking)    │
│  ├── /releases       → Version list                         │
│  ├── /news           → Article list                         │
│  ├── /status         → Service status                       │
│  ├── /static         → Never changes (false positive test)  │
│  └── /api/state      → Mutation API                         │
├─────────────────────────────────────────────────────────────┤
│  Test Flow:                                                  │
│  1. Reset server state                                       │
│  2. Create watch with `kto new`                              │
│  3. Take baseline with `kto run`                             │
│  4. Mutate content via API                                   │
│  5. Check with `kto test`                                    │
│  6. Assert: change detected correctly                        │
└─────────────────────────────────────────────────────────────┘
```

## Metrics

| Metric | Definition | Target |
|--------|------------|--------|
| Precision | TP / (TP + FP) | ≥95% |
| Recall | TP / (TP + FN) | ≥90% |
| Noise Rate | FP / Unchanged checks | <5% |

## Test Scenarios (22 total)

### True Positives (should detect)
- `test_01` - Price drop ($99 → $79)
- `test_02` - Price increase
- `test_03` - Stock: SOLD OUT → Add to Cart
- `test_04` - Stock: Add to Cart → SOLD OUT
- `test_05` - New release added
- `test_06` - Status degraded
- `test_07` - New article
- `test_08` - Item removed from list
- `test_09` - Middle item edited
- `test_10` - Status outage

### True Negatives (should NOT detect)
- `test_11` - Static page unchanged
- `test_12` - Price unchanged
- `test_13` - Stock unchanged
- `test_14` - Ad rotation only (noise)

### Error Handling
- `test_18` - HTTP 403
- `test_19` - HTTP 500
- `test_20` - Timeout
- `test_21` - Empty response
- `test_22` - Malformed HTML

### Idempotence & State
- `test_23` - 10 runs on static = 0 false positives
- `test_24` - Alert fires once, not repeatedly
- `test_25` - Large content (167KB)

## Files

```
tests/e2e/
├── README.md           # This file
├── run_suite.py        # Test runner (no dependencies)
├── run.sh              # Convenience script
└── harness/
    └── server.py       # Test server (stdlib only)
```

## Requirements

- Python 3.8+
- No external dependencies (uses stdlib only)
- kto must be buildable (`cargo build`)

## Output

```
============================================================
kto E2E Test Suite
============================================================
Running test_01_price_drop... PASS
Running test_02_price_increase... PASS
...

============================================================
METRICS
============================================================
Precision:        100.0% (target: >=95%)
Recall:           72.7% (target: >=90%)
Noise Rate:       0.0% (target: <5%)
```

Reports saved to `e2e_report.json`.

## When to Run

- **Before releases** - Must pass
- **After changing extraction/detection logic** - Catches regressions
- **In CI** - Add to GitHub Actions

## vs. Other Testing

| Test Type | Purpose | This Suite |
|-----------|---------|------------|
| `cargo test` | Unit tests | No |
| E2E Suite | Change detection accuracy | **Yes** |
| Orchestration | Live site exploration | No |

The orchestration cycles test "can we create watches on real sites?" which is useful for discovering edge cases but NOT a quality gate. This E2E suite is the quality gate.
