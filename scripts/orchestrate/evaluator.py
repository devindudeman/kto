"""Evaluator module: deterministic E2E evaluation + Claude live validation.

Classifies each observation against ground truth (E2E) or heuristics (live),
updates monitor confusion matrix and tracking statistics.
"""

from __future__ import annotations

import logging
from typing import List, Optional

from .config import IntentDefinition, MutationStep
from .state import Evaluation, MonitorState, Observation

logger = logging.getLogger(__name__)


# =============================================================================
# E2E Evaluation (deterministic, ground truth from test server)
# =============================================================================


def evaluate_e2e(
    monitor: MonitorState,
    observation: Observation,
    intent: IntentDefinition,
    applied_mutations: List[MutationStep],
) -> Evaluation:
    """Deterministic E2E evaluation.

    We control the test server, so we know ground truth: which mutations
    were applied and whether they should trigger detection.

    Args:
        monitor: Current monitor state (includes cycle_count).
        observation: The observation from this check cycle.
        intent: The intent definition (includes mutation schedule).
        applied_mutations: Mutations that have been applied so far.

    Returns:
        Evaluation with classification (TP/TN/FP/FN) and agent correctness.
    """
    current_cycle = monitor.cycle_count

    # -------------------------------------------------------------------------
    # a. Determine if a change-triggering mutation was applied before this cycle
    # -------------------------------------------------------------------------
    # Find mutations with expect_detection=True that were applied at or before
    # the current cycle. We consider a change "expected" if the most recent
    # such mutation was applied in the previous cycle or this cycle (i.e., it's
    # fresh enough that kto should detect it now).
    expected_change = False
    triggering_mutations = [
        m for m in applied_mutations
        if m.expect_detection and m.cycle <= current_cycle
    ]

    if triggering_mutations:
        most_recent = max(triggering_mutations, key=lambda m: m.cycle)
        # Expected if the mutation was applied this cycle or the previous one.
        # Beyond that, kto should have already detected it.
        if most_recent.cycle >= current_cycle - 1:
            expected_change = True

    # -------------------------------------------------------------------------
    # b. Determine actual change
    # -------------------------------------------------------------------------
    actual_change = observation.changed and observation.error is None

    # -------------------------------------------------------------------------
    # c. Classify: TP, TN, FP, FN
    # -------------------------------------------------------------------------
    if expected_change and actual_change:
        classification = "TP"
    elif not expected_change and not actual_change:
        classification = "TN"
    elif not expected_change and actual_change:
        classification = "FP"
    else:  # expected_change and not actual_change
        classification = "FN"

    # -------------------------------------------------------------------------
    # d. Determine agent_correct
    # -------------------------------------------------------------------------
    agent_correct: Optional[bool] = None

    if observation.agent_notified is not None:
        if classification == "TP":
            # Agent should notify on true positives
            agent_correct = observation.agent_notified is True
        elif classification == "TN":
            # Agent should NOT notify when nothing changed
            agent_correct = observation.agent_notified is False
        elif classification == "FP":
            # Change was detected but shouldn't have been.
            # Agent is correct if it suppressed the false change.
            agent_correct = observation.agent_notified is False
        elif classification == "FN":
            # Change was missed entirely, agent wasn't called with a change.
            # Not applicable -- leave as None.
            agent_correct = None

    # -------------------------------------------------------------------------
    # e. Build reason string
    # -------------------------------------------------------------------------
    reason_parts = []

    if expected_change:
        if triggering_mutations:
            most_recent = max(triggering_mutations, key=lambda m: m.cycle)
            reason_parts.append(
                f"Mutation '{most_recent.description or most_recent.field}' "
                f"applied at cycle {most_recent.cycle}"
            )
        reason_parts.append("change expected")
    else:
        reason_parts.append("no change expected")

    if actual_change:
        reason_parts.append("change detected")
    elif observation.error:
        reason_parts.append(f"error: {observation.error}")
    else:
        reason_parts.append("no change detected")

    reason_parts.append(f"classification={classification}")

    if agent_correct is not None:
        notified_str = "notified" if observation.agent_notified else "suppressed"
        correct_str = "correct" if agent_correct else "incorrect"
        reason_parts.append(f"agent {notified_str} ({correct_str})")

    reason = "; ".join(reason_parts)

    # -------------------------------------------------------------------------
    # f. Return Evaluation
    # -------------------------------------------------------------------------
    evaluation = Evaluation(
        classification=classification,
        expected_change=expected_change,
        actual_change=actual_change,
        agent_correct=agent_correct,
        reason=reason,
    )

    logger.debug(
        "E2E evaluation for %s cycle %d: %s — %s",
        monitor.name, current_cycle, classification, reason,
    )

    return evaluation


# =============================================================================
# Live Evaluation (no ground truth)
# =============================================================================


def evaluate_live(
    monitor: MonitorState,
    observation: Observation,
) -> Evaluation:
    """Live mode evaluation -- no ground truth, classification is approximate.

    In live mode we don't control the server, so we can't know whether a
    change was expected.  We classify optimistically: detected changes are
    assumed true positives, no-change cycles are assumed true negatives.

    Args:
        monitor: Current monitor state.
        observation: The observation from this check cycle.

    Returns:
        Evaluation with approximate classification.
    """
    actual_change = observation.changed

    if observation.error:
        # Errors don't count as changes
        classification = "TN"
        reason = f"Error during check ({observation.error}); treated as TN"
    elif actual_change:
        classification = "TP"
        reason = "Change detected in live mode; assumed TP (no ground truth)"
    else:
        classification = "TN"
        reason = "No change detected in live mode; assumed TN"

    evaluation = Evaluation(
        classification=classification,
        expected_change=False,  # unknown in live mode
        actual_change=actual_change and observation.error is None,
        agent_correct=None,  # self-confirming in live, don't track
        reason=reason,
    )

    logger.debug(
        "Live evaluation for %s cycle %d: %s — %s",
        monitor.name, monitor.cycle_count, classification, reason,
    )

    return evaluation


# =============================================================================
# Update Monitor Stats
# =============================================================================


def update_monitor_stats(
    monitor: MonitorState,
    evaluation: Evaluation,
    score: float,
) -> None:
    """Update the monitor's confusion matrix and tracking stats.

    Args:
        monitor: Monitor state to update in place.
        evaluation: The evaluation for this cycle.
        score: The computed efficacy score for this cycle.
    """
    # Increment confusion matrix
    if evaluation.classification == "TP":
        monitor.tp += 1
    elif evaluation.classification == "TN":
        monitor.tn += 1
    elif evaluation.classification == "FP":
        monitor.fp += 1
    elif evaluation.classification == "FN":
        monitor.fn += 1

    # Track agent decision accuracy
    if evaluation.agent_correct is not None:
        monitor.agent_total_decisions += 1
        if evaluation.agent_correct:
            monitor.agent_correct_decisions += 1

    # Append score
    monitor.scores.append(score)

    # Track detection latency for TPs
    if evaluation.classification == "TP":
        latency = compute_detection_latency(monitor, evaluation)
        if latency is not None:
            monitor.detection_latencies.append(latency)

    # Cap lists at 100 entries (evict oldest)
    if len(monitor.observations) > 100:
        monitor.observations = monitor.observations[-100:]
    if len(monitor.evaluations) > 100:
        monitor.evaluations = monitor.evaluations[-100:]
    if len(monitor.scores) > 100:
        monitor.scores = monitor.scores[-100:]
    # detection_latencies capped at 50 (per state.py serialization)
    if len(monitor.detection_latencies) > 50:
        monitor.detection_latencies = monitor.detection_latencies[-50:]


# =============================================================================
# Detection Latency
# =============================================================================


def compute_detection_latency(
    monitor: MonitorState,
    evaluation: Evaluation,
) -> Optional[int]:
    """For TP classifications, estimate cycles since the change happened.

    Looks at recent evaluations to find the last TN before this TP.
    The latency is the number of cycles between that last TN and this TP
    (i.e., the current cycle).

    Args:
        monitor: Monitor state with evaluation history.
        evaluation: The current evaluation (should be TP).

    Returns:
        Number of cycles between last TN and this detection, or None
        if not a TP or cannot be determined.
    """
    if evaluation.classification != "TP":
        return None

    current_cycle = monitor.cycle_count

    # Walk evaluations backwards to find the most recent TN
    for prev_eval in reversed(monitor.evaluations):
        if prev_eval.classification == "TN":
            # Find the observation that corresponds to this evaluation
            # The evaluations list is parallel to observations, so
            # we use the index to find the cycle number.
            idx = None
            for i, e in enumerate(monitor.evaluations):
                if e is prev_eval:
                    idx = i
                    break

            if idx is not None and idx < len(monitor.observations):
                tn_cycle = monitor.observations[idx].cycle
                latency = current_cycle - tn_cycle
                if latency >= 0:
                    logger.debug(
                        "Detection latency for %s: %d cycles "
                        "(last TN at cycle %d, detected at cycle %d)",
                        monitor.name, latency, tn_cycle, current_cycle,
                    )
                    return latency

            # Fallback: can't find matching observation, estimate as 1
            return 1

    # No previous TN found -- this is the first evaluation or all prior were
    # non-TN.  Return 1 as a conservative estimate (detected same cycle).
    return 1
