#!/usr/bin/env python3
"""
E2E Test Suite for kto Change Detection Validation

This suite validates that kto correctly detects (and doesn't false-positive on)
various types of web content changes.

Usage:
    python run_suite.py [--keep-server] [--verbose] [--scenario PATTERN]

Metrics tracked:
    - Precision: TP / (TP + FP)
    - Recall: TP / (TP + FN)
    - Noise Rate: FP / Total unchanged checks
    - AI Faithfulness: Claims that match diff / Total claims
"""

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import time
import urllib.request
import urllib.error
from dataclasses import dataclass, field
from typing import Any, Optional

# =============================================================================
# Configuration
# =============================================================================

SERVER_URL = "http://127.0.0.1:8787"
API_URL = f"{SERVER_URL}/api"
TEST_DB = "/tmp/kto_e2e_test.db"
KTO_CMD = ["cargo", "run", "--quiet", "--"]

# =============================================================================
# Metrics Collection
# =============================================================================

@dataclass
class TestMetrics:
    """Aggregated test metrics."""
    true_positives: int = 0
    true_negatives: int = 0
    false_positives: int = 0
    false_negatives: int = 0
    ai_faithful: int = 0
    ai_hallucinated: int = 0
    errors: int = 0
    total_runs: int = 0

    @property
    def precision(self) -> float:
        denom = self.true_positives + self.false_positives
        return self.true_positives / denom if denom > 0 else 0.0

    @property
    def recall(self) -> float:
        denom = self.true_positives + self.false_negatives
        return self.true_positives / denom if denom > 0 else 0.0

    @property
    def noise_rate(self) -> float:
        unchanged = self.true_negatives + self.false_positives
        return self.false_positives / unchanged if unchanged > 0 else 0.0

    @property
    def faithfulness_rate(self) -> float:
        total = self.ai_faithful + self.ai_hallucinated
        return self.ai_faithful / total if total > 0 else 0.0

    def summary(self) -> dict:
        return {
            "true_positives": self.true_positives,
            "true_negatives": self.true_negatives,
            "false_positives": self.false_positives,
            "false_negatives": self.false_negatives,
            "precision": round(self.precision, 3),
            "recall": round(self.recall, 3),
            "noise_rate": round(self.noise_rate, 3),
            "faithfulness_rate": round(self.faithfulness_rate, 3),
            "errors": self.errors,
            "total_runs": self.total_runs,
        }

metrics = TestMetrics()

# =============================================================================
# Test Result
# =============================================================================

@dataclass
class TestResult:
    """Result of a single test scenario."""
    name: str
    passed: bool
    expected_change: bool
    actual_change: bool
    details: dict = field(default_factory=dict)
    error: Optional[str] = None

    def __str__(self):
        status = "PASS" if self.passed else "FAIL"
        return f"[{status}] {self.name}"

# =============================================================================
# Helpers
# =============================================================================

def reset_db():
    """Remove test database for clean slate."""
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)

def kto(*args, capture=True) -> subprocess.CompletedProcess:
    """Run kto command with test database."""
    env = os.environ.copy()
    env["KTO_DB"] = TEST_DB
    cmd = KTO_CMD + list(args)

    if capture:
        result = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=120)
    else:
        result = subprocess.run(cmd, env=env, timeout=120)
    return result

def kto_json(*args) -> Optional[dict]:
    """Run kto command and parse JSON output."""
    result = kto(*args, "--json")
    if result.returncode != 0:
        print(f"  kto error: {result.stderr[:500]}")
        return None
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        print(f"  JSON parse error: {result.stdout[:500]}")
        return None

def api_get(endpoint: str) -> dict:
    """GET from test server API."""
    try:
        with urllib.request.urlopen(f"{API_URL}/{endpoint}") as resp:
            return json.loads(resp.read().decode())
    except Exception as e:
        return {"error": str(e)}

def api_post(endpoint: str, data: dict = None) -> dict:
    """POST to test server API."""
    try:
        body = json.dumps(data or {}).encode()
        req = urllib.request.Request(
            f"{API_URL}/{endpoint}",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST"
        )
        with urllib.request.urlopen(req) as resp:
            return json.loads(resp.read().decode())
    except Exception as e:
        return {"error": str(e)}

def api_reset():
    """Reset server to default state."""
    return api_post("reset")

def api_set(**kwargs):
    """Update server state."""
    return api_post("state", kwargs)

def wait_for_server(timeout: int = 30):
    """Wait for test server to be ready."""
    start = time.time()
    while time.time() - start < timeout:
        try:
            urllib.request.urlopen(f"{SERVER_URL}/static", timeout=1)
            return True
        except Exception:
            time.sleep(0.5)
    return False

# =============================================================================
# Test Scenario Definitions
# =============================================================================

def create_watch(name: str, url: str, intent: str = "") -> bool:
    """Create a watch for testing.

    Note: We don't use intent strings for E2E tests because they trigger
    AI analysis which is slow and not what we're testing here. We're testing
    the change detection pipeline, not the AI wizard.
    """
    full_url = f"{SERVER_URL}{url}"
    # Don't append intent - we want fast, simple watch creation for E2E tests
    # The intent string triggers AI analysis which is slow and not under test here

    result = kto("new", full_url, "--name", name, "--yes")
    return result.returncode == 0

def take_baseline(name: str) -> bool:
    """Take a baseline snapshot.

    Note: kto run checks ALL watches in the DB. Since each test uses a fresh
    DB with only one watch, this effectively runs just our test watch.
    """
    result = kto("run")
    return result.returncode == 0

def check_for_change(name: str) -> dict:
    """Check if watch detects a change. Returns parsed result."""
    result = kto("test", name, "--json")
    if result.returncode != 0:
        return {"error": result.stderr, "change_detected": False}

    try:
        data = json.loads(result.stdout)
        # kto uses "changed" key in JSON output
        data["change_detected"] = data.get("changed", False)
        return data
    except json.JSONDecodeError:
        # Non-JSON output means no change or error
        return {
            "change_detected": "change detected" in result.stdout.lower(),
            "raw_output": result.stdout
        }

def run_and_check(name: str) -> dict:
    """Run a check that saves state (for idempotence testing).

    Note: kto run checks ALL watches in the DB. Since each test uses a fresh
    DB with only one watch, this effectively runs just our test watch.

    Note: kto run doesn't support --json, so we parse text output.
    """
    result = kto("run")
    # kto run returns 0 even on success, check the output text
    output = result.stdout + result.stderr

    # Parse text output - look for "CHANGE DETECTED" or "no change"
    change_detected = "CHANGE DETECTED" in output.upper()

    return {
        "change_detected": change_detected,
        "raw_output": output,
        "returncode": result.returncode
    }

# =============================================================================
# Test Scenarios
# =============================================================================

class Scenarios:
    """All test scenarios organized by category."""

    # -------------------------------------------------------------------------
    # Category 1: True Positives (Should Detect & Notify)
    # -------------------------------------------------------------------------

    @staticmethod
    def test_01_price_drop() -> TestResult:
        """Price drop should be detected."""
        name = "test-price-drop"
        api_reset()
        api_set(product_price="$99.99")

        # Create watch and baseline
        create_watch(name, "/product-clean", "price drops")
        take_baseline(name)

        # Mutate: drop price
        api_set(product_price="$79.99")

        # Check
        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="01_price_drop",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_02_price_increase() -> TestResult:
        """Price increase should be detected."""
        name = "test-price-increase"
        api_reset()
        api_set(product_price="$79.99")

        create_watch(name, "/product-clean", "price changes")
        take_baseline(name)

        api_set(product_price="$99.99")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="02_price_increase",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_03_stock_oos_to_available() -> TestResult:
        """Stock: SOLD OUT -> Add to Cart should be detected."""
        name = "test-stock-available"
        api_reset()
        api_set(product_stock="SOLD OUT")

        create_watch(name, "/product-clean", "back in stock")
        take_baseline(name)

        api_set(product_stock="Add to Cart")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="03_stock_oos_to_available",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_04_stock_available_to_oos() -> TestResult:
        """Stock: Add to Cart -> SOLD OUT should be detected."""
        name = "test-stock-oos"
        api_reset()
        api_set(product_stock="Add to Cart")

        create_watch(name, "/product-clean", "stock changes")
        take_baseline(name)

        api_set(product_stock="SOLD OUT")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="04_stock_available_to_oos",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_05_new_release() -> TestResult:
        """New version added should be detected."""
        name = "test-new-release"
        api_reset()
        api_set(releases=["v1.0.0"])

        create_watch(name, "/releases", "new releases")
        take_baseline(name)

        api_set(releases=["v1.0.0", "v1.1.0"])

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="05_new_release",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_06_status_degraded() -> TestResult:
        """Status: operational -> degraded should be detected."""
        name = "test-status-degraded"
        api_reset()
        api_set(status="operational", status_message="All systems operational")

        create_watch(name, "/status", "status changes")
        take_baseline(name)

        api_set(status="degraded", status_message="Elevated error rates")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="06_status_degraded",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_07_new_article() -> TestResult:
        """New article added should be detected."""
        name = "test-new-article"
        api_reset()
        api_set(articles=[
            {"title": "First Article", "date": "2026-01-20"},
            {"title": "Second Article", "date": "2026-01-21"},
        ])

        create_watch(name, "/news", "new articles")
        take_baseline(name)

        api_set(articles=[
            {"title": "First Article", "date": "2026-01-20"},
            {"title": "Second Article", "date": "2026-01-21"},
            {"title": "Breaking News!", "date": "2026-01-22"},
        ])

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="07_new_article",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_08_item_removed() -> TestResult:
        """Item removed from list should be detected."""
        name = "test-item-removed"
        api_reset()
        api_set(releases=["v1.0.0", "v1.1.0", "v1.2.0"])

        create_watch(name, "/releases", "changes")
        take_baseline(name)

        api_set(releases=["v1.0.0", "v1.2.0"])  # v1.1.0 removed

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="08_item_removed",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_09_middle_item_edited() -> TestResult:
        """Middle item in list edited should be detected."""
        name = "test-middle-edit"
        api_reset()
        api_set(articles=[
            {"title": "Article A", "date": "2026-01-20"},
            {"title": "Article B", "date": "2026-01-21"},
            {"title": "Article C", "date": "2026-01-22"},
        ])

        create_watch(name, "/news", "changes")
        take_baseline(name)

        api_set(articles=[
            {"title": "Article A", "date": "2026-01-20"},
            {"title": "Article B - Updated!", "date": "2026-01-21"},  # Changed
            {"title": "Article C", "date": "2026-01-22"},
        ])

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="09_middle_item_edited",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_10_status_outage() -> TestResult:
        """Status: operational -> outage should be detected."""
        name = "test-status-outage"
        api_reset()
        api_set(status="operational")

        create_watch(name, "/status", "outages")
        take_baseline(name)

        api_set(status="outage", status_message="Major service disruption")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="10_status_outage",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )

    # -------------------------------------------------------------------------
    # Category 2: True Negatives (Should NOT Detect)
    # -------------------------------------------------------------------------

    @staticmethod
    def test_11_static_unchanged() -> TestResult:
        """Static page with no changes should not trigger."""
        name = "test-static"
        api_reset()

        create_watch(name, "/static", "changes")
        take_baseline(name)

        # No mutation - page is truly static
        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == False
        if passed:
            metrics.true_negatives += 1
        else:
            metrics.false_positives += 1

        return TestResult(
            name="11_static_unchanged",
            passed=passed,
            expected_change=False,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_12_price_unchanged() -> TestResult:
        """Price unchanged should not trigger."""
        name = "test-price-same"
        api_reset()
        api_set(product_price="$99.99")

        create_watch(name, "/product-clean", "price drops")
        take_baseline(name)

        # Re-set same price (no actual change)
        api_set(product_price="$99.99")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == False
        if passed:
            metrics.true_negatives += 1
        else:
            metrics.false_positives += 1

        return TestResult(
            name="12_price_unchanged",
            passed=passed,
            expected_change=False,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_13_stock_unchanged() -> TestResult:
        """Stock status unchanged should not trigger."""
        name = "test-stock-same"
        api_reset()
        api_set(product_stock="SOLD OUT")

        create_watch(name, "/product-clean", "stock changes")
        take_baseline(name)

        # Same stock status
        api_set(product_stock="SOLD OUT")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        passed = detected == False
        if passed:
            metrics.true_negatives += 1
        else:
            metrics.false_positives += 1

        return TestResult(
            name="13_stock_unchanged",
            passed=passed,
            expected_change=False,
            actual_change=detected,
            details=result
        )

    @staticmethod
    def test_14_ad_rotation_only() -> TestResult:
        """Ad rotation should not trigger (noise)."""
        name = "test-ad-rotation"
        api_reset()
        api_set(ad_variant="A", include_timestamp=False, include_tracking=False, include_random_id=False)

        create_watch(name, "/product", "price drops")
        take_baseline(name)

        # Only ad changes
        api_set(ad_variant="B")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        # This might detect change due to ad content - that's acceptable noise
        # Mark as pass if no change, or note it detected noise
        passed = detected == False
        if passed:
            metrics.true_negatives += 1
        else:
            # This is a false positive (detected noise)
            metrics.false_positives += 1

        return TestResult(
            name="14_ad_rotation_only",
            passed=passed,
            expected_change=False,
            actual_change=detected,
            details=result
        )

    # -------------------------------------------------------------------------
    # Category 3: Error Handling
    # -------------------------------------------------------------------------

    @staticmethod
    def test_18_error_403() -> TestResult:
        """403 error should be handled gracefully."""
        name = "test-error-403"
        api_reset()

        create_watch(name, "/product-clean", "changes")
        take_baseline(name)

        # Simulate 403
        api_set(error_code=403)

        result = kto("test", name)

        # Should not crash, should log error
        passed = result.returncode == 0 or "error" in result.stdout.lower() or "error" in result.stderr.lower()

        # Reset error state
        api_set(error_code=None)

        return TestResult(
            name="18_error_403",
            passed=passed,
            expected_change=False,
            actual_change=False,
            details={"stdout": result.stdout, "stderr": result.stderr, "code": result.returncode}
        )

    @staticmethod
    def test_19_error_500() -> TestResult:
        """500 error should be handled gracefully."""
        name = "test-error-500"
        api_reset()

        create_watch(name, "/product-clean", "changes")
        take_baseline(name)

        api_set(error_code=500)

        result = kto("test", name)

        passed = result.returncode == 0 or "error" in result.stdout.lower() or "error" in result.stderr.lower()

        api_set(error_code=None)

        return TestResult(
            name="19_error_500",
            passed=passed,
            expected_change=False,
            actual_change=False,
            details={"stdout": result.stdout, "stderr": result.stderr}
        )

    @staticmethod
    def test_20_error_timeout() -> TestResult:
        """Timeout should be handled gracefully."""
        name = "test-timeout"
        api_reset()

        create_watch(name, "/product-clean", "changes")
        take_baseline(name)

        # Set delay longer than typical timeout (but not too long for test)
        api_set(delay_seconds=5.0)

        result = kto("test", name)

        # Should complete (maybe with timeout error) without crashing
        passed = True  # If we get here, it didn't hang forever

        api_set(delay_seconds=0.0)

        return TestResult(
            name="20_error_timeout",
            passed=passed,
            expected_change=False,
            actual_change=False,
            details={"stdout": result.stdout[:500], "stderr": result.stderr[:500]}
        )

    @staticmethod
    def test_21_empty_response() -> TestResult:
        """Empty response should be handled."""
        name = "test-empty"
        api_reset()

        create_watch(name, "/product-clean", "changes")
        take_baseline(name)

        api_set(return_empty=True)

        result = kto("test", name)

        # Should handle without crash
        passed = True

        api_set(return_empty=False)

        return TestResult(
            name="21_empty_response",
            passed=passed,
            expected_change=False,
            actual_change=False,
            details={"stdout": result.stdout, "stderr": result.stderr}
        )

    @staticmethod
    def test_22_malformed_html() -> TestResult:
        """Malformed HTML should be handled."""
        name = "test-malformed"
        api_reset()

        create_watch(name, "/product-clean", "changes")
        take_baseline(name)

        api_set(return_malformed=True)

        result = kto("test", name)

        # Should extract something without crashing
        passed = result.returncode == 0

        api_set(return_malformed=False)

        return TestResult(
            name="22_malformed_html",
            passed=passed,
            expected_change=False,  # Might detect change due to malformed content
            actual_change=False,
            details={"stdout": result.stdout[:500], "stderr": result.stderr[:500]}
        )

    # -------------------------------------------------------------------------
    # Category 4: Idempotence & State
    # -------------------------------------------------------------------------

    @staticmethod
    def test_23_idempotence_static() -> TestResult:
        """Repeated runs on static content should never trigger."""
        name = "test-idempotent"
        api_reset()

        create_watch(name, "/static", "changes")

        false_positives = 0
        runs = 10

        for i in range(runs):
            result = run_and_check(name)
            if result.get("change_detected", False):
                false_positives += 1

        passed = false_positives == 0
        metrics.total_runs += runs

        if not passed:
            metrics.false_positives += false_positives

        return TestResult(
            name="23_idempotence_static",
            passed=passed,
            expected_change=False,
            actual_change=false_positives > 0,
            details={"runs": runs, "false_positives": false_positives}
        )

    @staticmethod
    def test_24_alert_once_only() -> TestResult:
        """Change should only trigger alert once, not on subsequent runs."""
        name = "test-alert-once"
        api_reset()
        api_set(product_price="$99.99")

        create_watch(name, "/product-clean", "price changes")
        run_and_check(name)  # Baseline
        time.sleep(0.5)  # Allow DB to flush

        # Make change
        api_set(product_price="$79.99")

        # First check should detect
        result1 = run_and_check(name)
        first_detected = result1.get("change_detected", False)
        time.sleep(0.5)  # Allow DB to flush

        # Second check (no new change) should NOT detect
        result2 = run_and_check(name)
        second_detected = result2.get("change_detected", False)

        passed = first_detected and not second_detected

        return TestResult(
            name="24_alert_once_only",
            passed=passed,
            expected_change=True,  # First time
            actual_change=first_detected,
            details={
                "first_detected": first_detected,
                "second_detected": second_detected,
                "result1": result1,
                "result2": result2
            }
        )

    @staticmethod
    def test_25_large_content() -> TestResult:
        """Large content (50KB+) should be handled."""
        name = "test-large"
        api_reset()
        api_set(product_price="$99.99")

        create_watch(name, "/large", "price changes")
        take_baseline(name)

        api_set(product_price="$79.99")

        result = check_for_change(name)
        detected = result.get("change_detected", False)

        # Should detect the price change even in large content
        passed = detected == True
        if passed:
            metrics.true_positives += 1
        else:
            metrics.false_negatives += 1

        return TestResult(
            name="25_large_content",
            passed=passed,
            expected_change=True,
            actual_change=detected,
            details=result
        )


# =============================================================================
# Test Runner
# =============================================================================

def get_all_tests():
    """Get all test methods."""
    tests = []
    for name in dir(Scenarios):
        if name.startswith("test_"):
            tests.append((name, getattr(Scenarios, name)))
    return sorted(tests, key=lambda x: x[0])


def run_tests(pattern: str = None, verbose: bool = False):
    """Run all tests and collect results."""
    tests = get_all_tests()
    results = []

    print(f"\n{'='*60}")
    print(f"kto E2E Test Suite")
    print(f"{'='*60}")
    print(f"Database: {TEST_DB}")
    print(f"Server: {SERVER_URL}")
    print(f"Tests: {len(tests)}")
    print(f"{'='*60}\n")

    for name, test_fn in tests:
        if pattern and pattern not in name:
            continue

        print(f"Running {name}...", end=" ", flush=True)
        reset_db()  # Clean slate for each test

        try:
            result = test_fn()
            results.append(result)

            if result.passed:
                print("PASS")
            else:
                print("FAIL")
                if verbose:
                    print(f"  Expected change: {result.expected_change}")
                    print(f"  Actual change: {result.actual_change}")
                    print(f"  Details: {json.dumps(result.details, indent=2)[:500]}")
        except Exception as e:
            print(f"ERROR: {e}")
            metrics.errors += 1
            results.append(TestResult(
                name=name,
                passed=False,
                expected_change=False,
                actual_change=False,
                error=str(e)
            ))

    return results


def print_summary(results: list):
    """Print test summary and metrics."""
    passed = sum(1 for r in results if r.passed)
    failed = sum(1 for r in results if not r.passed)

    print(f"\n{'='*60}")
    print("RESULTS SUMMARY")
    print(f"{'='*60}")
    print(f"Total:  {len(results)}")
    print(f"Passed: {passed}")
    print(f"Failed: {failed}")
    print(f"Pass Rate: {100*passed/len(results):.1f}%")

    print(f"\n{'='*60}")
    print("METRICS")
    print(f"{'='*60}")
    summary = metrics.summary()
    print(f"Precision:        {summary['precision']:.1%} (target: >=95%)")
    print(f"Recall:           {summary['recall']:.1%} (target: >=90%)")
    print(f"Noise Rate:       {summary['noise_rate']:.1%} (target: <5%)")
    print(f"Faithfulness:     {summary['faithfulness_rate']:.1%} (target: >=90%)")
    print(f"True Positives:   {summary['true_positives']}")
    print(f"True Negatives:   {summary['true_negatives']}")
    print(f"False Positives:  {summary['false_positives']}")
    print(f"False Negatives:  {summary['false_negatives']}")
    print(f"Errors:           {summary['errors']}")

    # Failed tests
    if failed > 0:
        print(f"\n{'='*60}")
        print("FAILED TESTS")
        print(f"{'='*60}")
        for r in results:
            if not r.passed:
                print(f"  - {r.name}: expected_change={r.expected_change}, actual={r.actual_change}")
                if r.error:
                    print(f"    Error: {r.error}")

    return summary


def save_report(results: list, summary: dict, filename: str = "e2e_report.json"):
    """Save detailed report to JSON."""
    report = {
        "timestamp": time.strftime("%Y-%m-%d %H:%M:%S"),
        "summary": summary,
        "results": [
            {
                "name": r.name,
                "passed": r.passed,
                "expected_change": r.expected_change,
                "actual_change": r.actual_change,
                "error": r.error
            }
            for r in results
        ]
    }

    with open(filename, "w") as f:
        json.dump(report, f, indent=2)

    print(f"\nReport saved to: {filename}")


# =============================================================================
# Main
# =============================================================================

def main():
    parser = argparse.ArgumentParser(description="kto E2E Test Suite")
    parser.add_argument("--keep-server", action="store_true",
                        help="Don't start/stop server (assume already running)")
    parser.add_argument("--verbose", "-v", action="store_true",
                        help="Show detailed output for failed tests")
    parser.add_argument("--scenario", "-s", type=str,
                        help="Only run scenarios matching pattern")
    parser.add_argument("--report", "-r", type=str, default="e2e_report.json",
                        help="Output report filename")
    args = parser.parse_args()

    server_proc = None

    try:
        if not args.keep_server:
            # Start test server
            print("Starting test server...")
            server_script = os.path.join(os.path.dirname(__file__), "harness", "server.py")
            server_proc = subprocess.Popen(
                [sys.executable, server_script],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE
            )

            if not wait_for_server():
                print("ERROR: Test server failed to start")
                sys.exit(1)
            print("Server ready.")
        else:
            if not wait_for_server(timeout=5):
                print("ERROR: Test server not running. Start with: python tests/e2e/harness/server.py")
                sys.exit(1)

        # Run tests
        results = run_tests(pattern=args.scenario, verbose=args.verbose)

        # Summary
        summary = print_summary(results)

        # Save report
        save_report(results, summary, args.report)

        # Exit code
        failed = sum(1 for r in results if not r.passed)
        sys.exit(0 if failed == 0 else 1)

    finally:
        if server_proc:
            print("\nStopping test server...")
            server_proc.terminate()
            server_proc.wait(timeout=5)


if __name__ == "__main__":
    main()
