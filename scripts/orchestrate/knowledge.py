"""KnowledgeBase module with schema versioning, scoping, decay, precedence, and export.

Manages the knowledge.json file where learned creation rules are persisted.
Rules are scoped by intent_type and optionally by domain_class, with
time-based decay and promotion from domain-scoped to intent-scoped rules.
"""

from __future__ import annotations

import json
import logging
import os
import uuid
from datetime import datetime, timezone
from typing import Dict, List, Optional

from .config import (
    KNOWLEDGE_SCHEMA_VERSION,
    PRECEDENCE_ORDER,
    DECAY_RATES,
    MIN_MONITORS_FOR_INTENT_SCOPE,
    MIN_POSITIVE_EVENTS_FOR_PROMOTION,
    CreationRule,
    CreationRecommendation,
)

logger = logging.getLogger(__name__)


class KnowledgeBase:
    """Persistent store of learned creation rules with versioning and decay.

    The knowledge base is stored as a JSON file with the following structure::

        {
            "schema_version": 1,
            "rules": [...],
            "precedence": ["user_override", "discovery_result", ...]
        }

    Rules are matched by intent_type and optionally domain_class, sorted by
    precedence (domain-scoped before intent-only) then by confidence descending.
    """

    def __init__(self, path: str) -> None:
        self.path: str = path
        self.rules: List[CreationRule] = []
        self.schema_version: int = KNOWLEDGE_SCHEMA_VERSION

    def load(self) -> bool:
        """Load knowledge base from disk.

        Returns:
            True if loaded successfully, False if file doesn't exist or has
            an unknown schema_version (safe fallback).
        """
        if not os.path.exists(self.path):
            logger.debug("Knowledge file does not exist: %s", self.path)
            return False

        try:
            with open(self.path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except (json.JSONDecodeError, OSError) as exc:
            logger.warning("Failed to read knowledge file %s: %s", self.path, exc)
            return False

        file_version = data.get("schema_version", 0)
        if file_version != KNOWLEDGE_SCHEMA_VERSION:
            logger.warning(
                "Unknown knowledge schema version %s (expected %s), "
                "falling back to empty knowledge base",
                file_version,
                KNOWLEDGE_SCHEMA_VERSION,
            )
            return False

        self.schema_version = file_version
        self.rules = [CreationRule.from_dict(r) for r in data.get("rules", [])]
        logger.info("Loaded %d rules from %s", len(self.rules), self.path)
        return True

    def save(self) -> None:
        """Atomic write: write to .tmp then os.replace to final path."""
        data = {
            "schema_version": self.schema_version,
            "rules": [r.to_dict() for r in self.rules],
            "precedence": PRECEDENCE_ORDER,
        }

        dir_path = os.path.dirname(self.path)
        if dir_path:
            os.makedirs(dir_path, exist_ok=True)

        tmp_path = self.path + ".tmp"
        try:
            with open(tmp_path, "w", encoding="utf-8") as f:
                json.dump(data, f, indent=2)
                f.write("\n")
            os.replace(tmp_path, self.path)
            logger.debug("Saved %d rules to %s", len(self.rules), self.path)
        except OSError as exc:
            logger.error("Failed to save knowledge file %s: %s", self.path, exc)
            # Clean up tmp file if replace failed
            if os.path.exists(tmp_path):
                try:
                    os.remove(tmp_path)
                except OSError:
                    pass
            raise

    def add_rule(self, rule: CreationRule) -> None:
        """Add or update a rule.

        If a rule with the same intent_type + domain_class + rule text already
        exists, update it only if the new confidence is higher. Generates a
        rule ID if the incoming rule has an empty id.
        """
        if not rule.id:
            rule.id = str(uuid.uuid4())

        if not rule.created_at:
            rule.created_at = datetime.now(timezone.utc).isoformat()

        if not rule.last_validated:
            rule.last_validated = rule.created_at

        # Look for existing rule with same key triple
        for i, existing in enumerate(self.rules):
            if (
                existing.intent_type == rule.intent_type
                and existing.domain_class == rule.domain_class
                and existing.rule == rule.rule
            ):
                if rule.confidence > existing.confidence:
                    # Preserve the original ID and created_at
                    rule.id = existing.id
                    rule.created_at = existing.created_at
                    self.rules[i] = rule
                    logger.debug(
                        "Updated rule %s: confidence %.2f -> %.2f",
                        rule.id,
                        existing.confidence,
                        rule.confidence,
                    )
                else:
                    logger.debug(
                        "Skipped rule update %s: existing confidence %.2f >= new %.2f",
                        existing.id,
                        existing.confidence,
                        rule.confidence,
                    )
                return

        self.rules.append(rule)
        logger.debug("Added new rule %s (intent=%s, domain=%s)", rule.id, rule.intent_type, rule.domain_class)

    def get_rules(
        self, intent_type: str, domain_class: Optional[str] = None
    ) -> List[CreationRule]:
        """Get matching rules sorted by precedence then confidence.

        Args:
            intent_type: The intent type to match (e.g., "price", "stock").
            domain_class: Optional domain class. If provided, matches rules
                with the same domain_class OR rules with no domain_class.
                If None, only matches rules with no domain_class.

        Returns:
            List of matching rules sorted by scope precedence (intent+domain
            first, then intent-only) then by confidence descending.
        """
        matched: List[CreationRule] = []

        for rule in self.rules:
            if rule.intent_type != intent_type:
                continue

            if domain_class is not None:
                # Match rules with same domain_class OR no domain_class
                if rule.domain_class is not None and rule.domain_class != domain_class:
                    continue
            else:
                # Only match rules with no domain_class
                if rule.domain_class is not None:
                    continue

            matched.append(rule)

        # Sort: domain-scoped (intent+domain) first, then intent-only, then by confidence desc
        def sort_key(r: CreationRule) -> tuple:
            # Lower sort value = higher priority
            has_domain = 0 if r.domain_class is not None else 1
            return (has_domain, -r.confidence)

        matched.sort(key=sort_key)
        return matched

    def get_recommendation(
        self, intent_type: str, domain_class: Optional[str] = None
    ) -> Optional[CreationRecommendation]:
        """Get the best recommendation by merging matching rules.

        Higher confidence rules win per field. Returns None if no rules match.

        Args:
            intent_type: The intent type to match.
            domain_class: Optional domain class for scoping.

        Returns:
            A merged CreationRecommendation, or None if no matching rules.
        """
        rules = self.get_rules(intent_type, domain_class)
        if not rules:
            return None

        # Merge: first rule to set a field wins (rules are sorted by precedence)
        rec = CreationRecommendation()
        field_sources: Dict[str, float] = {}  # field -> confidence of rule that set it

        for rule in rules:
            r = rule.recommendation
            if r.engine is not None and "engine" not in field_sources:
                rec.engine = r.engine
                field_sources["engine"] = rule.confidence
            elif r.engine is not None and rule.confidence > field_sources.get("engine", 0.0):
                rec.engine = r.engine
                field_sources["engine"] = rule.confidence

            if r.extraction is not None and "extraction" not in field_sources:
                rec.extraction = r.extraction
                field_sources["extraction"] = rule.confidence
            elif r.extraction is not None and rule.confidence > field_sources.get("extraction", 0.0):
                rec.extraction = r.extraction
                field_sources["extraction"] = rule.confidence

            if r.interval_secs is not None and "interval_secs" not in field_sources:
                rec.interval_secs = r.interval_secs
                field_sources["interval_secs"] = rule.confidence
            elif r.interval_secs is not None and rule.confidence > field_sources.get("interval_secs", 0.0):
                rec.interval_secs = r.interval_secs
                field_sources["interval_secs"] = rule.confidence

            if r.instruction_template is not None and "instruction_template" not in field_sources:
                rec.instruction_template = r.instruction_template
                field_sources["instruction_template"] = rule.confidence
            elif r.instruction_template is not None and rule.confidence > field_sources.get("instruction_template", 0.0):
                rec.instruction_template = r.instruction_template
                field_sources["instruction_template"] = rule.confidence

            if r.selector is not None and "selector" not in field_sources:
                rec.selector = r.selector
                field_sources["selector"] = rule.confidence
            elif r.selector is not None and rule.confidence > field_sources.get("selector", 0.0):
                rec.selector = r.selector
                field_sources["selector"] = rule.confidence

        return rec

    def apply_decay(self) -> int:
        """Apply time-based decay to all rules based on rule_type.

        Reduces confidence by (days_since_last_validated * decay_rate).
        Removes rules whose confidence drops below 0.1.

        Returns:
            Number of rules removed due to decay.
        """
        now = datetime.now(timezone.utc)
        surviving: List[CreationRule] = []
        removed_count = 0

        for rule in self.rules:
            if rule.last_validated:
                try:
                    last_validated = datetime.fromisoformat(rule.last_validated)
                    # Ensure timezone-aware comparison
                    if last_validated.tzinfo is None:
                        last_validated = last_validated.replace(tzinfo=timezone.utc)
                    days_elapsed = (now - last_validated).total_seconds() / 86400.0
                except (ValueError, TypeError):
                    days_elapsed = 0.0
            else:
                days_elapsed = 0.0

            rate = DECAY_RATES.get(rule.rule_type, DECAY_RATES.get("heuristic", 0.02))
            decay_amount = days_elapsed * rate
            rule.confidence = max(0.0, rule.confidence - decay_amount)

            if rule.confidence < 0.1:
                logger.debug(
                    "Removing decayed rule %s (confidence %.4f, type=%s, days=%.1f)",
                    rule.id,
                    rule.confidence,
                    rule.rule_type,
                    days_elapsed,
                )
                removed_count += 1
            else:
                surviving.append(rule)

        self.rules = surviving
        if removed_count > 0:
            logger.info("Decay removed %d rules, %d remaining", removed_count, len(self.rules))
        return removed_count

    def try_promote_rule(self, rule: CreationRule) -> Optional[CreationRule]:
        """Attempt to promote a domain-scoped rule to intent-scoped.

        If the rule has evidence from >= MIN_MONITORS_FOR_INTENT_SCOPE source
        domains and >= MIN_POSITIVE_EVENTS_FOR_PROMOTION positive events,
        create a new intent-scoped rule (without domain_class).

        Args:
            rule: The rule to potentially promote.

        Returns:
            A new promoted intent-scoped CreationRule, or None if promotion
            criteria are not met.
        """
        unique_domains = set(rule.source_domains)

        if len(unique_domains) < MIN_MONITORS_FOR_INTENT_SCOPE:
            logger.debug(
                "Rule %s: %d source domains < %d required for promotion",
                rule.id,
                len(unique_domains),
                MIN_MONITORS_FOR_INTENT_SCOPE,
            )
            return None

        if rule.positive_events_observed < MIN_POSITIVE_EVENTS_FOR_PROMOTION:
            logger.debug(
                "Rule %s: %d positive events < %d required for promotion",
                rule.id,
                rule.positive_events_observed,
                MIN_POSITIVE_EVENTS_FOR_PROMOTION,
            )
            return None

        # Create promoted rule
        now_iso = datetime.now(timezone.utc).isoformat()
        promoted = CreationRule(
            id=str(uuid.uuid4()),
            intent_type=rule.intent_type,
            domain_class=None,
            scope="intent",
            rule=rule.rule,
            evidence=f"Promoted from domain-scoped rule {rule.id} with "
                     f"{len(unique_domains)} domains and "
                     f"{rule.positive_events_observed} positive events",
            confidence=rule.confidence * 0.8,  # Slight confidence discount for generalization
            positive_events_observed=rule.positive_events_observed,
            applies_to=rule.applies_to,
            recommendation=CreationRecommendation(
                engine=rule.recommendation.engine,
                extraction=rule.recommendation.extraction,
                interval_secs=rule.recommendation.interval_secs,
                instruction_template=rule.recommendation.instruction_template,
                selector=rule.recommendation.selector,
            ),
            source_domains=list(unique_domains),
            created_at=now_iso,
            last_validated=now_iso,
            rule_type=rule.rule_type,
        )

        self.add_rule(promoted)
        logger.info(
            "Promoted rule %s -> %s (intent=%s, confidence=%.2f)",
            rule.id,
            promoted.id,
            promoted.intent_type,
            promoted.confidence,
        )
        return promoted

    def export(self) -> dict:
        """Export the full knowledge base as a dict for reporting.

        Returns:
            Dictionary with schema_version, precedence, rules, and summary
            statistics.
        """
        rules_by_intent: Dict[str, int] = {}
        rules_by_type: Dict[str, int] = {}
        rules_by_scope: Dict[str, int] = {}

        for rule in self.rules:
            rules_by_intent[rule.intent_type] = rules_by_intent.get(rule.intent_type, 0) + 1
            rules_by_type[rule.rule_type] = rules_by_type.get(rule.rule_type, 0) + 1
            scope = "domain-scoped" if rule.domain_class is not None else "intent-scoped"
            rules_by_scope[scope] = rules_by_scope.get(scope, 0) + 1

        return {
            "schema_version": self.schema_version,
            "precedence": PRECEDENCE_ORDER,
            "rules": [r.to_dict() for r in self.rules],
            "summary": {
                "total_rules": len(self.rules),
                "by_intent": rules_by_intent,
                "by_type": rules_by_type,
                "by_scope": rules_by_scope,
            },
        }

    def rule_count(self) -> int:
        """Return the number of rules in the knowledge base."""
        return len(self.rules)
