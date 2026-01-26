"""Time-blocked A/B experiment protocol with randomization and conclusion logic.

Manages experiment lifecycle: block assignment, variant selection per cycle,
observation recording, and statistical conclusion to produce CreationRules.
"""

from __future__ import annotations

import logging
import random
import uuid
from datetime import datetime
from typing import Dict, List, Optional

from .config import (
    CONFIDENCE_MULTIPLIER,
    CreationRecommendation,
    CreationRule,
    EXPERIMENT_BLOCK_SIZE,
    EXPERIMENT_DELTA_THRESHOLD,
    MAX_CONFIDENCE,
    MIN_BLOCKS_PER_VARIANT,
    MIN_POSITIVE_EVENTS_PER_VARIANT,
)
from .state import Experiment, ExperimentBlock

logger = logging.getLogger(__name__)


# =============================================================================
# Helper: derive intent_type and domain_class from monitor_name
# =============================================================================

def _parse_monitor_name(monitor_name: str) -> tuple:
    """Extract intent_type and domain_class from a monitor name.

    Convention: monitor names follow the pattern "intent-domain-..." or
    "intent_domain_...". Falls back to ("generic", None) if unparseable.
    """
    # Normalize separators
    normalized = monitor_name.replace("-", "_").lower()
    parts = normalized.split("_")

    known_intents = {"price", "stock", "release", "news", "generic"}

    intent_type = "generic"
    domain_class = None

    if parts:
        if parts[0] in known_intents:
            intent_type = parts[0]
            if len(parts) > 1:
                domain_class = parts[1]
        else:
            # First part might be a domain class
            domain_class = parts[0]

    return intent_type, domain_class


# =============================================================================
# Block Assignment
# =============================================================================

def assign_blocks(
    experiment: Experiment,
    total_cycles: int,
    block_size: int = EXPERIMENT_BLOCK_SIZE,
) -> None:
    """Pre-assign randomized time blocks at experiment start.

    Randomizes which variant goes first, then alternates variants in blocks
    of ``block_size`` cycles. Each block is an ExperimentBlock with variant,
    start_cycle, and end_cycle. Sets ``experiment.blocks``.
    """
    variants = [experiment.variant_a, experiment.variant_b]
    # Randomize which variant goes first
    if random.random() < 0.5:
        variants = list(reversed(variants))

    blocks: List[ExperimentBlock] = []
    cycle = 0
    variant_idx = 0

    while cycle < total_cycles:
        start = cycle
        end = min(cycle + block_size - 1, total_cycles - 1)
        block = ExperimentBlock(
            variant=variants[variant_idx % 2],
            start_cycle=start,
            end_cycle=end,
        )
        blocks.append(block)
        cycle = end + 1
        variant_idx += 1

    experiment.blocks = blocks
    logger.info(
        "Assigned %d blocks for experiment %s (%s vs %s), first variant: %s",
        len(blocks),
        experiment.id,
        experiment.variant_a,
        experiment.variant_b,
        variants[0],
    )


# =============================================================================
# Block / Variant Lookup
# =============================================================================

def get_current_block(
    experiment: Experiment, cycle: int
) -> Optional[ExperimentBlock]:
    """Find which block is active for the given cycle number.

    Returns None if no block covers this cycle.
    """
    for block in experiment.blocks:
        if block.start_cycle <= cycle and (
            block.end_cycle is None or cycle <= block.end_cycle
        ):
            return block
    return None


def get_current_variant(experiment: Experiment, cycle: int) -> Optional[str]:
    """Get the variant string for the given cycle.

    Convenience wrapper around :func:`get_current_block`.
    """
    block = get_current_block(experiment, cycle)
    if block is not None:
        return block.variant
    return None


# =============================================================================
# Observation Recording
# =============================================================================

def record_observation(
    experiment: Experiment,
    cycle: int,
    score: float,
    classification: str,
) -> None:
    """Record a score and classification into the current block.

    Finds the current block for this cycle, appends the score, and increments
    positive_events (for TP) or negative_events (for TN).
    """
    block = get_current_block(experiment, cycle)
    if block is None:
        logger.warning(
            "No block found for cycle %d in experiment %s; observation dropped",
            cycle,
            experiment.id,
        )
        return

    block.scores.append(score)

    if classification == "TP":
        block.positive_events += 1
    elif classification == "TN":
        block.negative_events += 1

    logger.debug(
        "Recorded observation for experiment %s cycle %d: score=%.3f class=%s variant=%s",
        experiment.id,
        cycle,
        score,
        classification,
        block.variant,
    )


# =============================================================================
# Experiment Conclusion
# =============================================================================

def conclude_experiment(experiment: Experiment) -> Optional[CreationRule]:
    """Try to conclude the experiment.

    Returns a :class:`CreationRule` if a winner is found, or None if the
    experiment cannot yet be concluded or shows no meaningful difference.
    """
    # Partition blocks by variant (only those with scores)
    blocks_a = [
        b for b in experiment.blocks
        if b.variant == experiment.variant_a and len(b.scores) > 0
    ]
    blocks_b = [
        b for b in experiment.blocks
        if b.variant == experiment.variant_b and len(b.scores) > 0
    ]

    # Count total positive events per variant
    pos_a = sum(b.positive_events for b in blocks_a)
    pos_b = sum(b.positive_events for b in blocks_b)

    # Check minimum positive events per variant
    if pos_a < MIN_POSITIVE_EVENTS_PER_VARIANT or pos_b < MIN_POSITIVE_EVENTS_PER_VARIANT:
        experiment.status = "insufficient_data"
        logger.info(
            "Experiment %s: insufficient positive events (A=%d, B=%d, need %d each)",
            experiment.id,
            pos_a,
            pos_b,
            MIN_POSITIVE_EVENTS_PER_VARIANT,
        )
        return None

    # Check minimum blocks per variant
    if len(blocks_a) < MIN_BLOCKS_PER_VARIANT or len(blocks_b) < MIN_BLOCKS_PER_VARIANT:
        experiment.status = "insufficient_data"
        logger.info(
            "Experiment %s: insufficient blocks (A=%d, B=%d, need %d each)",
            experiment.id,
            len(blocks_a),
            len(blocks_b),
            MIN_BLOCKS_PER_VARIANT,
        )
        return None

    # Compute mean score for each variant across all blocks
    all_scores_a = [s for b in blocks_a for s in b.scores]
    all_scores_b = [s for b in blocks_b for s in b.scores]

    mean_a = sum(all_scores_a) / len(all_scores_a) if all_scores_a else 0.0
    mean_b = sum(all_scores_b) / len(all_scores_b) if all_scores_b else 0.0

    # Compute delta
    delta = abs(mean_a - mean_b)

    # Check if delta is meaningful
    if delta < EXPERIMENT_DELTA_THRESHOLD:
        experiment.status = "concluded"
        experiment.conclusion_evidence = (
            f"No meaningful difference: mean_a={mean_a:.3f}, mean_b={mean_b:.3f}, "
            f"delta={delta:.3f} < threshold={EXPERIMENT_DELTA_THRESHOLD}"
        )
        logger.info(
            "Experiment %s concluded: no meaningful difference (delta=%.3f)",
            experiment.id,
            delta,
        )
        return None

    # Determine winner
    if mean_a >= mean_b:
        winner = experiment.variant_a
    else:
        winner = experiment.variant_b

    experiment.status = "concluded"
    experiment.winner = winner

    # Compute confidence
    confidence = min(MAX_CONFIDENCE, delta * CONFIDENCE_MULTIPLIER)

    # Parse monitor name for intent and domain
    intent_type, domain_class = _parse_monitor_name(experiment.monitor_name)

    total_positive = pos_a + pos_b
    evidence = (
        f"A/B experiment on {experiment.field_name}: "
        f"variant_a='{experiment.variant_a}' (mean={mean_a:.3f}, blocks={len(blocks_a)}, pos={pos_a}) vs "
        f"variant_b='{experiment.variant_b}' (mean={mean_b:.3f}, blocks={len(blocks_b)}, pos={pos_b}). "
        f"Winner='{winner}' with delta={delta:.3f}, confidence={confidence:.2f}."
    )

    experiment.conclusion_evidence = evidence

    # Build the recommendation
    recommendation = CreationRecommendation()
    field_name = experiment.field_name
    if field_name == "extraction":
        recommendation.extraction = winner
    elif field_name == "engine":
        recommendation.engine = winner
    elif field_name == "interval_secs":
        try:
            recommendation.interval_secs = int(winner)
        except ValueError:
            pass
    elif field_name == "instructions":
        recommendation.instruction_template = winner

    rule = CreationRule(
        id=uuid.uuid4().hex[:12],
        intent_type=intent_type,
        domain_class=domain_class,
        scope="intent+domain" if domain_class else "intent",
        rule=f"Use {field_name}='{winner}' for {intent_type} monitors",
        evidence=evidence,
        confidence=confidence,
        positive_events_observed=total_positive,
        applies_to="creation",
        recommendation=recommendation,
        source_domains=[domain_class] if domain_class else [],
        created_at=datetime.utcnow().isoformat(),
        last_validated=datetime.utcnow().isoformat(),
        rule_type="heuristic",
    )

    logger.info(
        "Experiment %s concluded: winner='%s', confidence=%.2f, rule=%s",
        experiment.id,
        winner,
        confidence,
        rule.id,
    )

    return rule


# =============================================================================
# Factory: Create Experiment
# =============================================================================

def create_experiment(
    monitor_name: str,
    field_name: str,
    variant_a: str,
    variant_b: str,
    total_cycles: int,
) -> Experiment:
    """Factory function to create a new experiment with blocks pre-assigned."""
    experiment = Experiment(
        id=uuid.uuid4().hex[:12],
        monitor_name=monitor_name,
        field_name=field_name,
        variant_a=variant_a,
        variant_b=variant_b,
        min_positive_events=MIN_POSITIVE_EVENTS_PER_VARIANT,
        min_blocks=MIN_BLOCKS_PER_VARIANT,
        status="running",
    )

    assign_blocks(experiment, total_cycles)

    logger.info(
        "Created experiment %s for monitor '%s': %s='%s' vs '%s' over %d cycles",
        experiment.id,
        monitor_name,
        field_name,
        variant_a,
        variant_b,
        total_cycles,
    )

    return experiment


# =============================================================================
# Planner: Next Experiment
# =============================================================================

# Fields to experiment on, in priority order
_EXPERIMENT_FIELDS = ["extraction", "engine", "interval_secs", "instructions"]


def plan_next_experiment(
    monitor_name: str,
    intent_type: str,
    current_config: dict,
    completed_experiments: List[Experiment],
) -> Optional[Experiment]:
    """Plan the next experiment for a monitor.

    Tries fields in order: extraction, engine, interval_secs, instructions.
    Skips fields that already have concluded experiments.
    Returns None if all fields have been tested.
    """
    # Collect fields already tested (any terminal status)
    tested_fields = {
        exp.field_name
        for exp in completed_experiments
        if exp.status in ("concluded", "insufficient_data")
    }

    for field_name in _EXPERIMENT_FIELDS:
        if field_name in tested_fields:
            continue

        current_value = current_config.get(field_name)
        if current_value is None:
            continue

        variant_a = str(current_value)
        variant_b = _generate_variant_b(field_name, current_value, intent_type)

        if variant_b is None or variant_b == variant_a:
            logger.debug(
                "Skipping field '%s' for monitor '%s': no alternative variant",
                field_name,
                monitor_name,
            )
            continue

        experiment = create_experiment(
            monitor_name=monitor_name,
            field_name=field_name,
            variant_a=variant_a,
            variant_b=variant_b,
            total_cycles=20,
        )

        logger.info(
            "Planned next experiment for monitor '%s': field='%s', A='%s', B='%s'",
            monitor_name,
            field_name,
            variant_a,
            variant_b,
        )

        return experiment

    logger.info(
        "All experiment fields exhausted for monitor '%s'", monitor_name
    )
    return None


def _generate_variant_b(
    field_name: str, current_value, intent_type: str
) -> Optional[str]:
    """Generate an alternative variant for a given field and intent type."""
    if field_name == "extraction":
        current = str(current_value)
        if current == "auto":
            return "selector"
        elif current == "selector":
            return "auto"
        else:
            return "auto"

    elif field_name == "engine":
        current = str(current_value)
        if current == "http":
            return "playwright"
        elif current == "playwright":
            return "http"
        else:
            return "http"

    elif field_name == "interval_secs":
        try:
            current_int = int(current_value)
        except (ValueError, TypeError):
            return None
        # For time-sensitive intents, try faster; otherwise try slower
        if intent_type in ("price", "stock"):
            # Try halving (faster checks)
            variant = current_int // 2
            if variant < 60:
                variant = 60  # floor at 1 minute
            return str(variant)
        else:
            # Try doubling (slower checks to reduce load)
            variant = current_int * 2
            return str(variant)

    elif field_name == "instructions":
        # Instructions experiments are context-dependent; no generic alternative
        return None

    return None
