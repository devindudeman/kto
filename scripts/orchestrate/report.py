"""Learning-focused report generation for orchestration runs.

Generates reports about what was learned during the orchestration run,
focusing on creation rules discovered through experimentation, not just
raw metrics. Produces both a structured report.json and a human-readable
report.txt.
"""

from __future__ import annotations

import json
import logging
import os
from datetime import datetime
from typing import Dict, List, Optional

from .config import CreationRule, MIN_POSITIVE_EVENTS_PER_VARIANT, MIN_BLOCKS_PER_VARIANT
from .state import RunState, MonitorState, Experiment
from .knowledge import KnowledgeBase

logger = logging.getLogger(__name__)


# =============================================================================
# Public API
# =============================================================================


def generate_report(
    state: RunState, knowledge: KnowledgeBase, output_dir: str
) -> str:
    """Generate the full report for an orchestration run.

    Writes both ``report.json`` (structured) and ``report.txt``
    (human-readable) to *output_dir*.

    Args:
        state: The run state containing all monitor and experiment data.
        knowledge: The knowledge base with learned creation rules.
        output_dir: Directory to write report files into.

    Returns:
        The human-readable report text.
    """
    report_data = build_report_data(state, knowledge)

    os.makedirs(output_dir, exist_ok=True)

    # Write structured JSON report
    json_path = os.path.join(output_dir, "report.json")
    tmp_path = json_path + ".tmp"
    try:
        with open(tmp_path, "w", encoding="utf-8") as f:
            json.dump(report_data, f, indent=2)
            f.write("\n")
        os.replace(tmp_path, json_path)
        logger.info("Wrote structured report to %s", json_path)
    except OSError as exc:
        logger.error("Failed to write report.json: %s", exc)
        if os.path.exists(tmp_path):
            try:
                os.remove(tmp_path)
            except OSError:
                pass

    # Write human-readable text report
    human_text = format_human_report(report_data)
    txt_path = os.path.join(output_dir, "report.txt")
    tmp_path = txt_path + ".tmp"
    try:
        with open(tmp_path, "w", encoding="utf-8") as f:
            f.write(human_text)
        os.replace(tmp_path, txt_path)
        logger.info("Wrote human-readable report to %s", txt_path)
    except OSError as exc:
        logger.error("Failed to write report.txt: %s", exc)
        if os.path.exists(tmp_path):
            try:
                os.remove(tmp_path)
            except OSError:
                pass

    return human_text


# =============================================================================
# Report Data Builder
# =============================================================================


def build_report_data(state: RunState, knowledge: KnowledgeBase) -> dict:
    """Build the structured report data dictionary.

    This is the canonical data structure that gets saved to report.json.
    All sections of the report are derived from this dict.

    Args:
        state: The run state containing all monitor and experiment data.
        knowledge: The knowledge base with learned creation rules.

    Returns:
        A dictionary with all report sections.
    """
    now = datetime.utcnow()

    # --- Header ---
    duration_str = _compute_duration(state.started_at, now)

    header = {
        "run_id": state.run_id,
        "mode": state.mode,
        "started_at": state.started_at,
        "report_generated_at": now.isoformat(),
        "duration": duration_str,
        "monitor_count": len(state.monitors),
        "total_cycles": state.total_cycles,
    }

    # --- Creation Rules Learned ---
    rules_learned = []
    for rule in knowledge.rules:
        rec_parts = _format_recommendation_parts(rule)
        rules_learned.append({
            "id": rule.id,
            "description": rule.rule,
            "evidence": rule.evidence,
            "scope": rule.scope,
            "intent_type": rule.intent_type,
            "domain_class": rule.domain_class,
            "confidence": rule.confidence,
            "positive_events": rule.positive_events_observed,
            "recommendation": rule.recommendation.to_dict(),
            "impact": _format_impact(rule, rec_parts),
        })

    # --- Experiments Concluded ---
    concluded = []
    inconclusive = []

    for exp in state.experiments.values():
        if exp.status == "concluded" and exp.winner is not None:
            # Compute delta and positive events for the report
            delta, total_positive = _experiment_stats(exp)
            concluded.append({
                "id": exp.id,
                "monitor_name": exp.monitor_name,
                "field": exp.field_name,
                "variant_a": exp.variant_a,
                "variant_b": exp.variant_b,
                "winner": exp.winner,
                "delta": round(delta, 4),
                "positive_events": total_positive,
                "evidence": exp.conclusion_evidence,
            })
        elif exp.status == "concluded" and exp.winner is None:
            # Concluded but no meaningful difference
            delta, total_positive = _experiment_stats(exp)
            inconclusive.append({
                "id": exp.id,
                "monitor_name": exp.monitor_name,
                "field": exp.field_name,
                "variant_a": exp.variant_a,
                "variant_b": exp.variant_b,
                "reason": "no_meaningful_difference",
                "delta": round(delta, 4),
                "positive_events": total_positive,
                "evidence": exp.conclusion_evidence,
                "needed": _compute_needed(exp, "no_meaningful_difference"),
            })
        elif exp.status == "insufficient_data":
            delta, total_positive = _experiment_stats(exp)
            inconclusive.append({
                "id": exp.id,
                "monitor_name": exp.monitor_name,
                "field": exp.field_name,
                "variant_a": exp.variant_a,
                "variant_b": exp.variant_b,
                "reason": "insufficient_data",
                "delta": round(delta, 4),
                "positive_events": total_positive,
                "evidence": exp.conclusion_evidence,
                "needed": _compute_needed(exp, "insufficient_data"),
            })
        # Running experiments are omitted (not yet concluded)

    # --- Monitor Summary ---
    monitor_summaries = []
    for name, mon in sorted(state.monitors.items()):
        f1 = _compute_f1(mon)
        agent_accuracy = _compute_agent_accuracy(mon)
        current_score = mon.scores[-1] if mon.scores else 0.0

        monitor_summaries.append({
            "name": name,
            "intent_type": mon.intent_type,
            "domain_class": mon.domain_class,
            "mode": mon.mode,
            "cycle_count": mon.cycle_count,
            "confusion_matrix": {
                "tp": mon.tp,
                "tn": mon.tn,
                "fp": mon.fp,
                "fn": mon.fn,
            },
            "f1_score": round(f1, 4),
            "agent_accuracy": round(agent_accuracy, 4) if agent_accuracy is not None else None,
            "current_efficacy_score": round(current_score, 4),
        })

    # --- Recommendations for Next Run ---
    recommendations = _build_recommendations(state, inconclusive, knowledge)

    return {
        "header": header,
        "creation_rules_learned": rules_learned,
        "experiments_concluded": concluded,
        "experiments_inconclusive": inconclusive,
        "monitor_summary": monitor_summaries,
        "recommendations": recommendations,
    }


# =============================================================================
# Human-Readable Formatter
# =============================================================================


def format_human_report(report_data: dict) -> str:
    """Format report_data into a human-readable string.

    Uses box-drawing characters and indentation for readability.

    Args:
        report_data: The structured report dict from build_report_data().

    Returns:
        A formatted multi-line string.
    """
    lines: List[str] = []

    # === Header ===
    header = report_data["header"]
    lines.append("=" * 72)
    lines.append("  ORCHESTRATION RUN REPORT")
    lines.append("=" * 72)
    lines.append("")
    lines.append(f"  Run ID:       {header['run_id']}")
    lines.append(f"  Mode:         {header['mode']}")
    lines.append(f"  Duration:     {header['duration']}")
    lines.append(f"  Monitors:     {header['monitor_count']}")
    lines.append(f"  Total Cycles: {header['total_cycles']}")
    lines.append(f"  Started:      {header['started_at']}")
    lines.append(f"  Generated:    {header['report_generated_at']}")
    lines.append("")

    # === Creation Rules Learned ===
    rules = report_data["creation_rules_learned"]
    lines.append("-" * 72)
    lines.append("  CREATION RULES LEARNED")
    lines.append("-" * 72)
    lines.append("")

    if rules:
        for i, rule in enumerate(rules, 1):
            lines.append(f"  [{i}] {rule['description']}")
            lines.append(f"      Scope:      {rule['scope']}")
            if rule.get("domain_class"):
                lines.append(f"      Domain:     {rule['domain_class']}")
            lines.append(f"      Intent:     {rule['intent_type']}")
            lines.append(f"      Confidence: {rule['confidence']:.2f}")
            lines.append(f"      Evidence:   {rule['evidence']}")
            lines.append(f"      Impact:     {rule['impact']}")
            lines.append("")
    else:
        lines.append("  No creation rules were learned during this run.")
        lines.append("")

    # === Experiments Concluded ===
    concluded = report_data["experiments_concluded"]
    lines.append("-" * 72)
    lines.append("  EXPERIMENTS CONCLUDED")
    lines.append("-" * 72)
    lines.append("")

    if concluded:
        for i, exp in enumerate(concluded, 1):
            lines.append(f"  [{i}] {exp['monitor_name']}")
            lines.append(f"      Field tested:     {exp['field']}")
            lines.append(f"      Variant A:        {exp['variant_a']}")
            lines.append(f"      Variant B:        {exp['variant_b']}")
            lines.append(f"      Winner:           {exp['winner']}")
            lines.append(f"      Delta:            {exp['delta']:.4f}")
            lines.append(f"      Positive events:  {exp['positive_events']}")
            lines.append("")
    else:
        lines.append("  No experiments reached a conclusive winner.")
        lines.append("")

    # === Experiments Inconclusive ===
    inconclusive = report_data["experiments_inconclusive"]
    lines.append("-" * 72)
    lines.append("  EXPERIMENTS INCONCLUSIVE")
    lines.append("-" * 72)
    lines.append("")

    if inconclusive:
        for i, exp in enumerate(inconclusive, 1):
            reason_display = (
                "Insufficient data"
                if exp["reason"] == "insufficient_data"
                else "No meaningful difference"
            )
            lines.append(f"  [{i}] {exp['monitor_name']}")
            lines.append(f"      Field tested:     {exp['field']}")
            lines.append(f"      Variant A:        {exp['variant_a']}")
            lines.append(f"      Variant B:        {exp['variant_b']}")
            lines.append(f"      Reason:           {reason_display}")
            lines.append(f"      Positive events:  {exp['positive_events']}")
            if exp.get("needed"):
                lines.append(f"      Needed:           {exp['needed']}")
            lines.append("")
    else:
        lines.append("  All experiments reached conclusions.")
        lines.append("")

    # === Monitor Summary ===
    monitors = report_data["monitor_summary"]
    lines.append("-" * 72)
    lines.append("  MONITOR SUMMARY")
    lines.append("-" * 72)
    lines.append("")

    if monitors:
        for mon in monitors:
            cm = mon["confusion_matrix"]
            lines.append(f"  {mon['name']}")
            lines.append(f"      Intent:   {mon['intent_type']}  |  Mode: {mon['mode']}")
            lines.append(f"      Cycles:   {mon['cycle_count']}")
            lines.append(
                f"      Matrix:   TP={cm['tp']}  TN={cm['tn']}  "
                f"FP={cm['fp']}  FN={cm['fn']}"
            )
            lines.append(f"      F1 Score: {mon['f1_score']:.4f}")
            if mon["agent_accuracy"] is not None:
                lines.append(f"      Agent Accuracy: {mon['agent_accuracy']:.4f}")
            lines.append(f"      Efficacy: {mon['current_efficacy_score']:.4f}")
            lines.append("")
    else:
        lines.append("  No monitors were active during this run.")
        lines.append("")

    # === Recommendations for Next Run ===
    recommendations = report_data["recommendations"]
    lines.append("-" * 72)
    lines.append("  RECOMMENDATIONS FOR NEXT RUN")
    lines.append("-" * 72)
    lines.append("")

    if recommendations:
        for i, rec in enumerate(recommendations, 1):
            lines.append(f"  [{i}] {rec}")
        lines.append("")
    else:
        lines.append("  No specific recommendations. All experiments concluded successfully.")
        lines.append("")

    lines.append("=" * 72)
    lines.append("")

    return "\n".join(lines)


# =============================================================================
# Internal Helpers
# =============================================================================


def _compute_duration(started_at: str, now: datetime) -> str:
    """Compute human-readable duration from started_at ISO string to now."""
    try:
        start = datetime.fromisoformat(started_at)
    except (ValueError, TypeError):
        return "unknown"

    delta = now - start
    total_seconds = int(delta.total_seconds())

    if total_seconds < 0:
        return "unknown"

    hours, remainder = divmod(total_seconds, 3600)
    minutes, seconds = divmod(remainder, 60)

    if hours > 0:
        return f"{hours}h {minutes}m {seconds}s"
    elif minutes > 0:
        return f"{minutes}m {seconds}s"
    else:
        return f"{seconds}s"


def _format_recommendation_parts(rule: CreationRule) -> List[str]:
    """Extract human-readable recommendation parts from a rule."""
    parts: List[str] = []
    rec = rule.recommendation
    if rec.engine is not None:
        parts.append(f"engine={rec.engine}")
    if rec.extraction is not None:
        parts.append(f"extraction={rec.extraction}")
    if rec.interval_secs is not None:
        parts.append(f"interval={rec.interval_secs}s")
    if rec.instruction_template is not None:
        # Truncate long instruction templates for display
        tmpl = rec.instruction_template
        if len(tmpl) > 60:
            tmpl = tmpl[:57] + "..."
        parts.append(f"instructions='{tmpl}'")
    if rec.selector is not None:
        parts.append(f"selector='{rec.selector}'")
    return parts


def _format_impact(rule: CreationRule, rec_parts: List[str]) -> str:
    """Format the impact statement for a creation rule."""
    if not rec_parts:
        return f"Future {rule.intent_type} monitors will use this rule"

    rec_str = ", ".join(rec_parts)
    scope_label = rule.intent_type
    if rule.domain_class:
        scope_label = f"{rule.intent_type}+{rule.domain_class}"

    return f"Future {scope_label} monitors will default to {rec_str}"


def _experiment_stats(exp: Experiment) -> tuple:
    """Compute delta and total positive events for an experiment.

    Returns:
        A tuple of (delta, total_positive_events).
    """
    blocks_a = [
        b for b in exp.blocks
        if b.variant == exp.variant_a and len(b.scores) > 0
    ]
    blocks_b = [
        b for b in exp.blocks
        if b.variant == exp.variant_b and len(b.scores) > 0
    ]

    all_scores_a = [s for b in blocks_a for s in b.scores]
    all_scores_b = [s for b in blocks_b for s in b.scores]

    mean_a = sum(all_scores_a) / len(all_scores_a) if all_scores_a else 0.0
    mean_b = sum(all_scores_b) / len(all_scores_b) if all_scores_b else 0.0

    delta = abs(mean_a - mean_b)

    pos_a = sum(b.positive_events for b in blocks_a)
    pos_b = sum(b.positive_events for b in blocks_b)
    total_positive = pos_a + pos_b

    return delta, total_positive


def _compute_needed(exp: Experiment, reason: str) -> str:
    """Describe what would be needed for this experiment to conclude.

    Args:
        exp: The experiment that was inconclusive.
        reason: Either "insufficient_data" or "no_meaningful_difference".

    Returns:
        A human-readable string describing what is needed.
    """
    if reason == "no_meaningful_difference":
        return (
            "Variants performed similarly. Consider testing a more "
            "divergent alternative or running with more mutation variety."
        )

    # insufficient_data: figure out what is missing
    blocks_a = [b for b in exp.blocks if b.variant == exp.variant_a and len(b.scores) > 0]
    blocks_b = [b for b in exp.blocks if b.variant == exp.variant_b and len(b.scores) > 0]

    pos_a = sum(b.positive_events for b in blocks_a)
    pos_b = sum(b.positive_events for b in blocks_b)

    needed_parts: List[str] = []

    # Check positive events
    min_pos = MIN_POSITIVE_EVENTS_PER_VARIANT
    if pos_a < min_pos:
        needed_parts.append(
            f"{min_pos - pos_a} more positive events for variant A ('{exp.variant_a}')"
        )
    if pos_b < min_pos:
        needed_parts.append(
            f"{min_pos - pos_b} more positive events for variant B ('{exp.variant_b}')"
        )

    # Check blocks
    min_blk = MIN_BLOCKS_PER_VARIANT
    if len(blocks_a) < min_blk:
        needed_parts.append(
            f"{min_blk - len(blocks_a)} more blocks for variant A ('{exp.variant_a}')"
        )
    if len(blocks_b) < min_blk:
        needed_parts.append(
            f"{min_blk - len(blocks_b)} more blocks for variant B ('{exp.variant_b}')"
        )

    if needed_parts:
        return "; ".join(needed_parts)
    return "More cycles needed to accumulate sufficient data"


def _compute_f1(mon: MonitorState) -> float:
    """Compute F1 score from the monitor's confusion matrix."""
    tp = mon.tp
    fp = mon.fp
    fn = mon.fn

    precision = tp / (tp + fp) if (tp + fp) > 0 else 0.0
    recall = tp / (tp + fn) if (tp + fn) > 0 else 0.0

    if (precision + recall) > 0:
        return 2.0 * (precision * recall) / (precision + recall)
    return 0.0


def _compute_agent_accuracy(mon: MonitorState) -> Optional[float]:
    """Compute agent decision accuracy (E2E mode only).

    Returns:
        Accuracy as a float, or None if no agent decisions were recorded.
    """
    if mon.agent_total_decisions > 0:
        return mon.agent_correct_decisions / mon.agent_total_decisions
    return None


def _build_recommendations(
    state: RunState,
    inconclusive: List[dict],
    knowledge: KnowledgeBase,
) -> List[str]:
    """Build actionable recommendations for the next orchestration run.

    Based on inconclusive experiments, missing coverage, and the current
    state of the knowledge base.

    Args:
        state: The run state.
        inconclusive: List of inconclusive experiment dicts.
        knowledge: The knowledge base.

    Returns:
        A list of recommendation strings.
    """
    recommendations: List[str] = []

    # Recommendations from inconclusive experiments
    for exp in inconclusive:
        monitor_name = exp["monitor_name"]
        field = exp["field"]
        reason = exp["reason"]

        # Find the intent type for this monitor
        intent_type = "generic"
        if monitor_name in state.monitors:
            intent_type = state.monitors[monitor_name].intent_type

        if reason == "insufficient_data":
            recommendations.append(
                f"Add more {intent_type} mutations to E2E harness for "
                f"{field} comparison (monitor: {monitor_name})"
            )
        elif reason == "no_meaningful_difference":
            recommendations.append(
                f"Consider a more divergent {field} variant for "
                f"{intent_type} monitors (monitor: {monitor_name})"
            )

    # If we have E2E-only rules, recommend live validation
    e2e_rule_intents = set()
    for rule in knowledge.rules:
        e2e_rule_intents.add(rule.intent_type)

    if state.mode == "e2e" and e2e_rule_intents:
        intent_list = ", ".join(sorted(e2e_rule_intents))
        recommendations.append(
            f"Run live validation to confirm E2E-learned rules on real sites "
            f"(intents with rules: {intent_list})"
        )

    # Identify intents with no monitors or low cycle counts
    intent_cycle_counts: Dict[str, int] = {}
    for mon in state.monitors.values():
        intent_cycle_counts[mon.intent_type] = (
            intent_cycle_counts.get(mon.intent_type, 0) + mon.cycle_count
        )

    for intent, cycles in sorted(intent_cycle_counts.items()):
        if cycles < 10:
            recommendations.append(
                f"Intent '{intent}' has only {cycles} total cycles -- "
                f"consider running longer or adding more monitors"
            )

    # Identify monitors with high FP or FN rates
    for name, mon in sorted(state.monitors.items()):
        total_classified = mon.tp + mon.tn + mon.fp + mon.fn
        if total_classified < 5:
            continue

        fp_rate = mon.fp / total_classified
        fn_rate = mon.fn / total_classified

        if fp_rate > 0.2:
            recommendations.append(
                f"Monitor '{name}' has a high false positive rate "
                f"({mon.fp}/{total_classified} = {fp_rate:.0%}) -- "
                f"consider stricter normalization or filtering"
            )
        if fn_rate > 0.2:
            recommendations.append(
                f"Monitor '{name}' has a high false negative rate "
                f"({mon.fn}/{total_classified} = {fn_rate:.0%}) -- "
                f"consider more sensitive extraction or shorter intervals"
            )

    return recommendations
