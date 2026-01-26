"""Configuration, intent weight profiles, SLA map, and precedence rules.

This module defines all configuration constants and data models shared
across the orchestrator. Wave 0: interface contract — every subsequent
module imports from here and state.py.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional


# =============================================================================
# Orchestrator Configuration
# =============================================================================

@dataclass
class OrchestratorConfig:
    """Top-level configuration for a learning loop run."""

    intents_path: str = ""
    duration_hours: float = 12.0
    state_dir: str = "/tmp/kto-orchestrate"
    e2e_server_url: str = "http://127.0.0.1:8787"
    resume: bool = False
    dry_run: bool = False
    verbose: bool = False
    live_validate: bool = False
    kto_binary: str = "kto"
    kto_timeout_secs: int = 120
    claude_timeout_secs: int = 60
    max_evidence_per_monitor: int = 100
    log_max_bytes: int = 10 * 1024 * 1024  # 10MB


# =============================================================================
# Intent Weight Profiles (per-intent scoring weights)
# =============================================================================

# E2E mode: deterministic ground truth → agent_score is reliable
INTENT_WEIGHTS: Dict[str, Dict[str, float]] = {
    "price": {"f1": 0.35, "agent": 0.20, "latency": 0.30, "stability": 0.15},
    "stock": {"f1": 0.40, "agent": 0.25, "latency": 0.20, "stability": 0.15},
    "release": {"f1": 0.50, "agent": 0.20, "latency": 0.10, "stability": 0.20},
    "news": {"f1": 0.40, "agent": 0.25, "latency": 0.15, "stability": 0.20},
    "generic": {"f1": 0.45, "agent": 0.20, "latency": 0.15, "stability": 0.20},
}

# Live mode: agent_score is self-confirming, redistribute weight to F1
LIVE_INTENT_WEIGHTS: Dict[str, Dict[str, float]] = {
    "price": {"f1": 0.55, "agent": 0.00, "latency": 0.30, "stability": 0.15},
    "stock": {"f1": 0.65, "agent": 0.00, "latency": 0.20, "stability": 0.15},
    "release": {"f1": 0.70, "agent": 0.00, "latency": 0.10, "stability": 0.20},
    "news": {"f1": 0.65, "agent": 0.00, "latency": 0.15, "stability": 0.20},
    "generic": {"f1": 0.65, "agent": 0.00, "latency": 0.15, "stability": 0.20},
}


# =============================================================================
# SLA Map (maximum acceptable cycles before detection)
# =============================================================================

INTENT_SLA: Dict[str, int] = {
    "price": 1,
    "stock": 2,
    "release": 3,
    "news": 5,
    "generic": 3,
}


# =============================================================================
# Per-intent default check intervals (seconds)
# =============================================================================

INTENT_INTERVALS: Dict[str, int] = {
    "price": 300,      # 5 min
    "stock": 600,      # 10 min
    "release": 1800,   # 30 min
    "news": 900,       # 15 min
    "generic": 900,    # 15 min
}


# =============================================================================
# Experiment Protocol Constants
# =============================================================================

MIN_POSITIVE_EVENTS_PER_VARIANT: int = 5
MIN_BLOCKS_PER_VARIANT: int = 4
EXPERIMENT_BLOCK_SIZE: int = 3  # cycles per time block
EXPERIMENT_DELTA_THRESHOLD: float = 0.10
MAX_CONFIDENCE: float = 0.90
CONFIDENCE_MULTIPLIER: float = 2.5


# =============================================================================
# Knowledge Base Constants
# =============================================================================

KNOWLEDGE_SCHEMA_VERSION: int = 1

# Precedence chain for applying knowledge at creation time
PRECEDENCE_ORDER: List[str] = [
    "user_override",
    "discovery_result",
    "domain_scoped_rule",
    "intent_scoped_rule",
    "global_default",
]

# Decay rates (per day) for knowledge rules
DECAY_RATES: Dict[str, float] = {
    "structural": 0.05,
    "heuristic": 0.02,
    "domain": 0.01,
}

# Rule promotion thresholds
MIN_MONITORS_FOR_INTENT_SCOPE: int = 2
MIN_POSITIVE_EVENTS_FOR_PROMOTION: int = 5


# =============================================================================
# Creation Recommendation (output of knowledge lookup)
# =============================================================================

@dataclass
class CreationRecommendation:
    """Recommendation for how to create a new monitor."""

    engine: Optional[str] = None
    extraction: Optional[str] = None
    interval_secs: Optional[int] = None
    instruction_template: Optional[str] = None
    selector: Optional[str] = None

    def to_dict(self) -> dict:
        d = {}
        if self.engine is not None:
            d["engine"] = self.engine
        if self.extraction is not None:
            d["extraction"] = self.extraction
        if self.interval_secs is not None:
            d["interval_secs"] = self.interval_secs
        if self.instruction_template is not None:
            d["instruction_template"] = self.instruction_template
        if self.selector is not None:
            d["selector"] = self.selector
        return d


# =============================================================================
# Creation Rule (learned from experiments)
# =============================================================================

@dataclass
class CreationRule:
    """A rule learned from experimentation, stored in knowledge.json."""

    id: str = ""
    intent_type: str = ""
    domain_class: Optional[str] = None
    scope: str = "intent+domain"  # "intent+domain", "intent", "domain"
    rule: str = ""
    evidence: str = ""
    confidence: float = 0.0
    positive_events_observed: int = 0
    applies_to: str = "creation"
    recommendation: CreationRecommendation = field(default_factory=CreationRecommendation)
    source_domains: List[str] = field(default_factory=list)
    created_at: str = ""
    last_validated: str = ""
    rule_type: str = "heuristic"  # "structural", "heuristic", "domain"

    def to_dict(self) -> dict:
        return {
            "id": self.id,
            "intent_type": self.intent_type,
            "domain_class": self.domain_class,
            "scope": self.scope,
            "rule": self.rule,
            "evidence": self.evidence,
            "confidence": self.confidence,
            "positive_events_observed": self.positive_events_observed,
            "applies_to": self.applies_to,
            "recommendation": self.recommendation.to_dict(),
            "source_domains": self.source_domains,
            "created_at": self.created_at,
            "last_validated": self.last_validated,
            "rule_type": self.rule_type,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "CreationRule":
        rec_data = d.get("recommendation", {})
        rec = CreationRecommendation(
            engine=rec_data.get("engine"),
            extraction=rec_data.get("extraction"),
            interval_secs=rec_data.get("interval_secs"),
            instruction_template=rec_data.get("instruction_template"),
            selector=rec_data.get("selector"),
        )
        return cls(
            id=d.get("id", ""),
            intent_type=d.get("intent_type", ""),
            domain_class=d.get("domain_class"),
            scope=d.get("scope", "intent+domain"),
            rule=d.get("rule", ""),
            evidence=d.get("evidence", ""),
            confidence=d.get("confidence", 0.0),
            positive_events_observed=d.get("positive_events_observed", 0),
            applies_to=d.get("applies_to", "creation"),
            recommendation=rec,
            source_domains=d.get("source_domains", []),
            created_at=d.get("created_at", ""),
            last_validated=d.get("last_validated", ""),
            rule_type=d.get("rule_type", "heuristic"),
        )


# =============================================================================
# Efficacy Score
# =============================================================================

@dataclass
class EfficacyScore:
    """Composite efficacy score for a monitor cycle."""

    total: float = 0.0
    f1: float = 0.0
    precision: float = 0.0
    recall: float = 0.0
    agent: float = 0.0
    latency: float = 0.0
    stability: float = 0.0


# =============================================================================
# Intent Definition (loaded from TOML)
# =============================================================================

@dataclass
class IntentDefinition:
    """Definition of an intent to monitor, loaded from TOML."""

    name: str = ""
    url: str = ""
    intent_type: str = "generic"
    domain_class: str = "unknown"
    engine: str = "http"
    extraction: str = "auto"
    selector: Optional[str] = None
    interval_secs: int = 300
    agent_instructions: Optional[str] = None
    tags: List[str] = field(default_factory=list)
    mode: str = "e2e"  # "e2e" or "live"

    # E2E-specific fields
    mutations: List[MutationStep] = field(default_factory=list)
    expected_detections: int = 0

    @classmethod
    def from_dict(cls, d: dict) -> "IntentDefinition":
        mutations = [MutationStep.from_dict(m) for m in d.get("mutations", [])]
        return cls(
            name=d.get("name", ""),
            url=d.get("url", ""),
            intent_type=d.get("intent_type", "generic"),
            domain_class=d.get("domain_class", "unknown"),
            engine=d.get("engine", "http"),
            extraction=d.get("extraction", "auto"),
            selector=d.get("selector"),
            interval_secs=d.get("interval_secs", 300),
            agent_instructions=d.get("agent_instructions"),
            tags=d.get("tags", []),
            mode=d.get("mode", "e2e"),
            mutations=mutations,
            expected_detections=d.get("expected_detections", 0),
        )


@dataclass
class MutationStep:
    """A single mutation to apply during E2E testing."""

    cycle: int = 0
    field: str = ""
    value: str = ""
    expect_detection: bool = True
    description: str = ""

    @classmethod
    def from_dict(cls, d: dict) -> "MutationStep":
        return cls(
            cycle=d.get("cycle", 0),
            field=d.get("field", ""),
            value=str(d.get("value", "")),
            expect_detection=d.get("expect_detection", True),
            description=d.get("description", ""),
        )

    def to_dict(self) -> dict:
        return {
            "cycle": self.cycle,
            "field": self.field,
            "value": self.value,
            "expect_detection": self.expect_detection,
            "description": self.description,
        }
