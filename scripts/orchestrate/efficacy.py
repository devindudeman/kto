"""F1-based per-intent efficacy scoring, mode-aware (agent_score only in E2E).

Computes composite efficacy scores from confusion matrix accumulators,
detection latency, stability, and (in E2E mode) agent decision accuracy.
"""

from __future__ import annotations

import math
from statistics import stdev
from typing import List

from .config import EfficacyScore, INTENT_WEIGHTS, LIVE_INTENT_WEIGHTS, INTENT_SLA
from .state import MonitorState


def compute_efficacy(monitor: MonitorState, mode: str) -> EfficacyScore:
    """Compute the composite efficacy score for a monitor.

    Combines F1 (from confusion matrix), latency (normalized by SLA),
    stability (variance of recent scores), and agent accuracy (E2E only)
    into a single weighted score using per-intent weight profiles.

    Args:
        monitor: The monitor state with accumulated metrics.
        mode: Either "e2e" or "live". Controls weight selection and
              whether agent_score is included.

    Returns:
        An EfficacyScore with all component scores and the weighted total.
    """
    # --- a/b/c/d: Precision, recall, F1 from confusion matrix ---
    tp = monitor.tp
    tn = monitor.tn
    fp = monitor.fp
    fn = monitor.fn

    precision = tp / (tp + fp) if (tp + fp) > 0 else 0.0
    recall = tp / (tp + fn) if (tp + fn) > 0 else 0.0
    f1 = (
        2.0 * (precision * recall) / (precision + recall)
        if (precision + recall) > 0
        else 0.0
    )

    # --- e: Latency score (normalized by SLA) ---
    sla_cycles = INTENT_SLA.get(monitor.intent_type, 3)
    if monitor.detection_latencies:
        avg_latency = sum(monitor.detection_latencies) / len(
            monitor.detection_latencies
        )
    else:
        avg_latency = float(sla_cycles)

    latency_score = 1.0 - min(avg_latency, sla_cycles) / sla_cycles
    latency_score = max(0.0, min(1.0, latency_score))

    # --- f: Stability score ---
    stability_score = compute_stability(monitor)

    # --- g: Agent score (E2E only) ---
    if mode == "e2e" and monitor.agent_total_decisions > 0:
        agent_score = monitor.agent_correct_decisions / monitor.agent_total_decisions
    else:
        agent_score = 0.0

    # --- h: Select weights based on mode ---
    if mode == "e2e":
        weights = INTENT_WEIGHTS.get(
            monitor.intent_type, INTENT_WEIGHTS["generic"]
        )
    else:
        weights = LIVE_INTENT_WEIGHTS.get(
            monitor.intent_type, LIVE_INTENT_WEIGHTS["generic"]
        )

    # --- i: Weighted total ---
    total = (
        weights["f1"] * f1
        + weights.get("agent", 0) * agent_score
        + weights["latency"] * latency_score
        + weights["stability"] * stability_score
    )

    # --- j: Return EfficacyScore ---
    return EfficacyScore(
        total=total,
        f1=f1,
        precision=precision,
        recall=recall,
        agent=agent_score,
        latency=latency_score,
        stability=stability_score,
    )


def compute_stability(monitor: MonitorState) -> float:
    """Compute intent-aware stability from recent score variance.

    Stability measures how consistent the monitor's scores are over
    recent history. Some intent types (price, stock) naturally have
    more volatile scores, so the acceptable variance threshold is
    higher for those.

    Args:
        monitor: The monitor state containing historical scores.

    Returns:
        A float in [0.0, 1.0] where 1.0 is perfectly stable.
    """
    # Insufficient data: assume stable
    if len(monitor.scores) < 3:
        return 1.0

    # Use the last 10 scores
    recent: List[float] = monitor.scores[-10:]

    # Compute standard deviation
    std_dev = stdev(recent)

    # Intent-aware volatility threshold
    if monitor.intent_type in ("price", "stock"):
        threshold = 0.3
    elif monitor.intent_type in ("release", "news"):
        threshold = 0.2
    else:
        threshold = 0.2

    stability = 1.0 - min(std_dev / threshold, 1.0)
    stability = max(0.0, min(1.0, stability))

    return stability


def classify_observation(expected_change: bool, actual_change: bool) -> str:
    """Classify a single observation into TP, TN, FP, or FN.

    Args:
        expected_change: Whether a change was expected (ground truth).
        actual_change: Whether a change was actually detected.

    Returns:
        One of "TP", "TN", "FP", or "FN".
    """
    if expected_change and actual_change:
        return "TP"
    elif not expected_change and not actual_change:
        return "TN"
    elif not expected_change and actual_change:
        return "FP"
    else:  # expected_change and not actual_change
        return "FN"
