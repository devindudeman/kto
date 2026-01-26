"""RunState, MonitorState, and atomic persistence.

All shared mutable state lives here. Wave 0: interface contract.
"""

from __future__ import annotations

import json
import os
import time
import uuid
from dataclasses import dataclass, field
from datetime import datetime
from typing import Dict, List, Optional

from .config import EfficacyScore


# =============================================================================
# Observation (single check result)
# =============================================================================

@dataclass
class Observation:
    """Result of a single kto check cycle."""

    cycle: int = 0
    timestamp: str = ""
    changed: bool = False
    error: Optional[str] = None
    content_hash: Optional[str] = None
    diff_snippet: Optional[str] = None
    agent_notified: Optional[bool] = None
    agent_title: Optional[str] = None
    agent_summary: Optional[str] = None
    raw_json: Optional[dict] = None

    def to_dict(self) -> dict:
        return {
            "cycle": self.cycle,
            "timestamp": self.timestamp,
            "changed": self.changed,
            "error": self.error,
            "content_hash": self.content_hash,
            "diff_snippet": self.diff_snippet,
            "agent_notified": self.agent_notified,
            "agent_title": self.agent_title,
            "agent_summary": self.agent_summary,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "Observation":
        return cls(
            cycle=d.get("cycle", 0),
            timestamp=d.get("timestamp", ""),
            changed=d.get("changed", False),
            error=d.get("error"),
            content_hash=d.get("content_hash"),
            diff_snippet=d.get("diff_snippet"),
            agent_notified=d.get("agent_notified"),
            agent_title=d.get("agent_title"),
            agent_summary=d.get("agent_summary"),
        )


# =============================================================================
# Evaluation (classification of a check result)
# =============================================================================

@dataclass
class Evaluation:
    """Classification of an observation against ground truth."""

    classification: str = ""  # "TP", "TN", "FP", "FN"
    expected_change: bool = False
    actual_change: bool = False
    agent_correct: Optional[bool] = None
    reason: str = ""

    def to_dict(self) -> dict:
        return {
            "classification": self.classification,
            "expected_change": self.expected_change,
            "actual_change": self.actual_change,
            "agent_correct": self.agent_correct,
            "reason": self.reason,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "Evaluation":
        return cls(
            classification=d.get("classification", ""),
            expected_change=d.get("expected_change", False),
            actual_change=d.get("actual_change", False),
            agent_correct=d.get("agent_correct"),
            reason=d.get("reason", ""),
        )


# =============================================================================
# Experiment Block (time-blocked A/B)
# =============================================================================

@dataclass
class ExperimentBlock:
    """A contiguous period where one variant is active."""

    variant: str = ""
    start_cycle: int = 0
    end_cycle: Optional[int] = None
    scores: List[float] = field(default_factory=list)
    positive_events: int = 0  # TP count
    negative_events: int = 0  # TN count

    def to_dict(self) -> dict:
        return {
            "variant": self.variant,
            "start_cycle": self.start_cycle,
            "end_cycle": self.end_cycle,
            "scores": self.scores,
            "positive_events": self.positive_events,
            "negative_events": self.negative_events,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "ExperimentBlock":
        return cls(
            variant=d.get("variant", ""),
            start_cycle=d.get("start_cycle", 0),
            end_cycle=d.get("end_cycle"),
            scores=d.get("scores", []),
            positive_events=d.get("positive_events", 0),
            negative_events=d.get("negative_events", 0),
        )


# =============================================================================
# Experiment
# =============================================================================

@dataclass
class Experiment:
    """A/B experiment comparing two configurations."""

    id: str = ""
    monitor_name: str = ""
    field_name: str = ""  # "engine", "extraction", "instructions", "interval"
    variant_a: str = ""   # current value
    variant_b: str = ""   # test value
    blocks: List[ExperimentBlock] = field(default_factory=list)
    min_positive_events: int = 5
    min_blocks: int = 4
    status: str = "running"  # "running", "concluded", "insufficient_data"
    winner: Optional[str] = None
    conclusion_evidence: str = ""

    def to_dict(self) -> dict:
        return {
            "id": self.id,
            "monitor_name": self.monitor_name,
            "field_name": self.field_name,
            "variant_a": self.variant_a,
            "variant_b": self.variant_b,
            "blocks": [b.to_dict() for b in self.blocks],
            "min_positive_events": self.min_positive_events,
            "min_blocks": self.min_blocks,
            "status": self.status,
            "winner": self.winner,
            "conclusion_evidence": self.conclusion_evidence,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "Experiment":
        blocks = [ExperimentBlock.from_dict(b) for b in d.get("blocks", [])]
        return cls(
            id=d.get("id", ""),
            monitor_name=d.get("monitor_name", ""),
            field_name=d.get("field_name", ""),
            variant_a=d.get("variant_a", ""),
            variant_b=d.get("variant_b", ""),
            blocks=blocks,
            min_positive_events=d.get("min_positive_events", 5),
            min_blocks=d.get("min_blocks", 4),
            status=d.get("status", "running"),
            winner=d.get("winner"),
            conclusion_evidence=d.get("conclusion_evidence", ""),
        )


# =============================================================================
# Monitor State
# =============================================================================

@dataclass
class MonitorState:
    """State for a single monitored watch."""

    name: str = ""
    watch_name: str = ""
    intent_type: str = "generic"
    domain_class: str = "unknown"
    mode: str = "e2e"
    cycle_count: int = 0
    interval_secs: int = 300
    observations: List[Observation] = field(default_factory=list)
    evaluations: List[Evaluation] = field(default_factory=list)
    scores: List[float] = field(default_factory=list)
    current_config: Dict[str, str] = field(default_factory=dict)
    active_experiment_id: Optional[str] = None

    # Confusion matrix accumulators
    tp: int = 0
    tn: int = 0
    fp: int = 0
    fn: int = 0

    # Agent decision tracking (E2E only)
    agent_correct_decisions: int = 0
    agent_total_decisions: int = 0

    # Latency tracking (cycles to first detection after change)
    detection_latencies: List[int] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "name": self.name,
            "watch_name": self.watch_name,
            "intent_type": self.intent_type,
            "domain_class": self.domain_class,
            "mode": self.mode,
            "cycle_count": self.cycle_count,
            "interval_secs": self.interval_secs,
            "observations": [o.to_dict() for o in self.observations[-100:]],  # cap at 100
            "evaluations": [e.to_dict() for e in self.evaluations[-100:]],
            "scores": self.scores[-100:],
            "current_config": self.current_config,
            "active_experiment_id": self.active_experiment_id,
            "tp": self.tp,
            "tn": self.tn,
            "fp": self.fp,
            "fn": self.fn,
            "agent_correct_decisions": self.agent_correct_decisions,
            "agent_total_decisions": self.agent_total_decisions,
            "detection_latencies": self.detection_latencies[-50:],
        }

    @classmethod
    def from_dict(cls, d: dict) -> "MonitorState":
        observations = [Observation.from_dict(o) for o in d.get("observations", [])]
        evaluations = [Evaluation.from_dict(e) for e in d.get("evaluations", [])]
        return cls(
            name=d.get("name", ""),
            watch_name=d.get("watch_name", ""),
            intent_type=d.get("intent_type", "generic"),
            domain_class=d.get("domain_class", "unknown"),
            mode=d.get("mode", "e2e"),
            cycle_count=d.get("cycle_count", 0),
            interval_secs=d.get("interval_secs", 300),
            observations=observations,
            evaluations=evaluations,
            scores=d.get("scores", []),
            current_config=d.get("current_config", {}),
            active_experiment_id=d.get("active_experiment_id"),
            tp=d.get("tp", 0),
            tn=d.get("tn", 0),
            fp=d.get("fp", 0),
            fn=d.get("fn", 0),
            agent_correct_decisions=d.get("agent_correct_decisions", 0),
            agent_total_decisions=d.get("agent_total_decisions", 0),
            detection_latencies=d.get("detection_latencies", []),
        )


# =============================================================================
# Run State (top-level)
# =============================================================================

@dataclass
class RunState:
    """Top-level state for the entire orchestration run."""

    run_id: str = ""
    started_at: str = ""
    mode: str = "e2e"
    monitors: Dict[str, MonitorState] = field(default_factory=dict)
    experiments: Dict[str, Experiment] = field(default_factory=dict)
    total_cycles: int = 0
    last_save_time: float = 0.0

    def __post_init__(self):
        if not self.run_id:
            self.run_id = uuid.uuid4().hex[:8]
        if not self.started_at:
            self.started_at = datetime.utcnow().isoformat()

    def to_dict(self) -> dict:
        return {
            "run_id": self.run_id,
            "started_at": self.started_at,
            "mode": self.mode,
            "monitors": {k: v.to_dict() for k, v in self.monitors.items()},
            "experiments": {k: v.to_dict() for k, v in self.experiments.items()},
            "total_cycles": self.total_cycles,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "RunState":
        monitors = {k: MonitorState.from_dict(v) for k, v in d.get("monitors", {}).items()}
        experiments = {k: Experiment.from_dict(v) for k, v in d.get("experiments", {}).items()}
        return cls(
            run_id=d.get("run_id", ""),
            started_at=d.get("started_at", ""),
            mode=d.get("mode", "e2e"),
            monitors=monitors,
            experiments=experiments,
            total_cycles=d.get("total_cycles", 0),
        )


# =============================================================================
# Atomic Persistence
# =============================================================================

def save_state_atomic(state: RunState, path: str) -> None:
    """Write state to disk atomically (write to .tmp then rename)."""
    tmp_path = path + ".tmp"
    data = state.to_dict()
    with open(tmp_path, "w") as f:
        json.dump(data, f, indent=2)
    os.replace(tmp_path, path)
    state.last_save_time = time.time()


def load_state(path: str) -> Optional[RunState]:
    """Load state from disk. Returns None if file doesn't exist."""
    if not os.path.exists(path):
        return None
    with open(path, "r") as f:
        data = json.load(f)
    return RunState.from_dict(data)
