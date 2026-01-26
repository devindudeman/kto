"""CycleRunner: wires observe -> evaluate -> experiment -> learn.

Orchestrates a single check cycle for each monitor, coordinating
mutation application (E2E), observation via kto CLI, evaluation
against ground truth, efficacy scoring, experiment recording, and
knowledge base updates.
"""

from __future__ import annotations

import logging
import os
import time
from datetime import datetime
from typing import Dict, List, Optional

from .config import (
    OrchestratorConfig,
    IntentDefinition,
    CreationRule,
    EfficacyScore,
    INTENT_INTERVALS,
    MutationStep,
)
from .state import (
    RunState,
    MonitorState,
    Observation,
    Evaluation,
    Experiment,
    save_state_atomic,
)
from .kto_client import KtoClient
from .server_bridge import ServerBridge
from .evaluator import evaluate_e2e, evaluate_live, update_monitor_stats
from .efficacy import compute_efficacy, classify_observation
from .experimenter import (
    get_current_variant,
    record_observation,
    conclude_experiment,
    plan_next_experiment,
    create_experiment,
)
from .knowledge import KnowledgeBase
from .log import OrchestrationLogger

logger = logging.getLogger(__name__)


class CycleRunner:
    """Runs observe -> evaluate -> experiment -> learn cycles for monitors.

    Coordinates the full lifecycle of a single check cycle: applying
    mutations (E2E mode), running kto checks, evaluating results against
    ground truth, computing efficacy scores, recording experiment
    observations, concluding experiments, and planning new ones.
    """

    def __init__(
        self,
        config: OrchestratorConfig,
        state: RunState,
        knowledge: KnowledgeBase,
        kto: KtoClient,
        server: Optional[ServerBridge],
        logger_: OrchestrationLogger,
        intents: List[IntentDefinition],
    ) -> None:
        self.config = config
        self.state = state
        self.knowledge = knowledge
        self.kto = kto
        self.server = server
        self.logger = logger_
        self.intents = intents

        # Map monitor names to their IntentDefinition for fast lookup
        self.intent_map: Dict[str, IntentDefinition] = {
            intent.name: intent for intent in intents
        }

        # Track which mutations have been applied per monitor
        self.applied_mutations: Dict[str, List[MutationStep]] = {}

    # =========================================================================
    # Public API
    # =========================================================================

    def run_cycle(self, monitor_name: str) -> Optional[EfficacyScore]:
        """Run a single observe -> evaluate -> experiment -> learn cycle.

        Args:
            monitor_name: Name of the monitor to run a cycle for.

        Returns:
            The computed EfficacyScore, or None if the monitor or intent
            is not found.
        """
        # -----------------------------------------------------------------
        # a. Resolve monitor and intent
        # -----------------------------------------------------------------
        monitor = self.state.monitors.get(monitor_name)
        if monitor is None:
            self.logger.error(
                f"Monitor '{monitor_name}' not found in state",
                monitor=monitor_name,
            )
            return None

        intent = self.intent_map.get(monitor_name)
        if intent is None:
            self.logger.error(
                f"IntentDefinition not found for monitor '{monitor_name}'",
                monitor=monitor_name,
            )
            return None

        # -----------------------------------------------------------------
        # b. Increment cycle count
        # -----------------------------------------------------------------
        monitor.cycle_count += 1
        cycle = monitor.cycle_count

        # Ensure applied_mutations list exists for this monitor
        if monitor_name not in self.applied_mutations:
            self.applied_mutations[monitor_name] = []

        # -----------------------------------------------------------------
        # c. Apply mutation (E2E only)
        # -----------------------------------------------------------------
        if intent.mode == "e2e" and self.server is not None:
            for mutation in intent.mutations:
                if mutation.cycle == cycle:
                    ok = self.server.apply_mutation(mutation)
                    if ok:
                        self.applied_mutations[monitor_name].append(mutation)
                        self.logger.info(
                            f"[{monitor_name}] Applied mutation at cycle {cycle}: "
                            f"{mutation.description or mutation.field}={mutation.value}",
                            monitor=monitor_name,
                            cycle=cycle,
                            mutation_field=mutation.field,
                            mutation_value=mutation.value,
                        )
                    else:
                        self.logger.error(
                            f"[{monitor_name}] Failed to apply mutation at cycle {cycle}: "
                            f"{mutation.field}={mutation.value}",
                            monitor=monitor_name,
                            cycle=cycle,
                            mutation_field=mutation.field,
                        )

        # -----------------------------------------------------------------
        # d. Determine active experiment variant
        # -----------------------------------------------------------------
        active_experiment: Optional[Experiment] = None
        active_variant: Optional[str] = None

        if monitor.active_experiment_id:
            active_experiment = self.state.experiments.get(
                monitor.active_experiment_id
            )
            if active_experiment is not None:
                active_variant = get_current_variant(active_experiment, cycle)
                if active_variant is not None:
                    logger.debug(
                        "Monitor '%s' cycle %d: experiment %s variant '%s' active",
                        monitor_name,
                        cycle,
                        active_experiment.id,
                        active_variant,
                    )

        # -----------------------------------------------------------------
        # e. Observe: run kto check
        # -----------------------------------------------------------------
        db_path = self.get_db_path()
        observation = self.kto.run_check(monitor.watch_name, db_path=db_path)
        observation.cycle = cycle
        observation.timestamp = datetime.utcnow().isoformat()

        # Append to monitor observations (capped at 100 by state serialization)
        monitor.observations.append(observation)
        if len(monitor.observations) > 100:
            monitor.observations = monitor.observations[-100:]

        # -----------------------------------------------------------------
        # f. Evaluate
        # -----------------------------------------------------------------
        if intent.mode == "e2e":
            evaluation = evaluate_e2e(
                monitor,
                observation,
                intent,
                self.applied_mutations.get(monitor_name, []),
            )
        else:
            evaluation = evaluate_live(monitor, observation)

        # Append to monitor evaluations (capped at 100)
        monitor.evaluations.append(evaluation)
        if len(monitor.evaluations) > 100:
            monitor.evaluations = monitor.evaluations[-100:]

        # -----------------------------------------------------------------
        # g. Score
        # -----------------------------------------------------------------
        score = compute_efficacy(monitor, self.state.mode)

        # -----------------------------------------------------------------
        # h. Update stats
        # -----------------------------------------------------------------
        update_monitor_stats(monitor, evaluation, score.total)

        # -----------------------------------------------------------------
        # i. Record to experiment (if active)
        # -----------------------------------------------------------------
        if active_experiment is not None and active_experiment.status == "running":
            record_observation(
                active_experiment,
                cycle,
                score.total,
                evaluation.classification,
            )

            rule = conclude_experiment(active_experiment)

            if rule is not None:
                # Experiment concluded with a winner -- add rule to knowledge base
                self.knowledge.add_rule(rule)
                self.knowledge.save()
                self.logger.learning(
                    f"[{monitor_name}] Experiment {active_experiment.id} concluded: "
                    f"winner='{active_experiment.winner}', "
                    f"rule={rule.id} (confidence={rule.confidence:.2f})",
                    monitor=monitor_name,
                    experiment_id=active_experiment.id,
                    winner=active_experiment.winner,
                    rule_id=rule.id,
                    confidence=rule.confidence,
                )
                # Clear the active experiment
                monitor.active_experiment_id = None

            elif active_experiment.status in ("concluded", "insufficient_data"):
                # Experiment concluded without a usable rule, or insufficient data
                self.logger.info(
                    f"[{monitor_name}] Experiment {active_experiment.id} ended: "
                    f"status={active_experiment.status}",
                    monitor=monitor_name,
                    experiment_id=active_experiment.id,
                    status=active_experiment.status,
                )
                monitor.active_experiment_id = None

        # -----------------------------------------------------------------
        # j. Plan next experiment (if no active experiment)
        # -----------------------------------------------------------------
        if monitor.active_experiment_id is None:
            # Gather concluded experiments for this monitor
            concluded = [
                exp
                for exp in self.state.experiments.values()
                if exp.monitor_name == monitor_name
                and exp.status in ("concluded", "insufficient_data")
            ]

            next_experiment = plan_next_experiment(
                monitor_name,
                intent.intent_type,
                monitor.current_config,
                concluded,
            )

            if next_experiment is not None:
                self.state.experiments[next_experiment.id] = next_experiment
                monitor.active_experiment_id = next_experiment.id
                self.logger.info(
                    f"[{monitor_name}] Planned experiment {next_experiment.id}: "
                    f"{next_experiment.field_name} "
                    f"'{next_experiment.variant_a}' vs '{next_experiment.variant_b}'",
                    monitor=monitor_name,
                    experiment_id=next_experiment.id,
                    field=next_experiment.field_name,
                    variant_a=next_experiment.variant_a,
                    variant_b=next_experiment.variant_b,
                )

        # -----------------------------------------------------------------
        # k. Log cycle result
        # -----------------------------------------------------------------
        self.logger.info(
            f"[{monitor_name}] Cycle {cycle}: "
            f"changed={observation.changed}, "
            f"class={evaluation.classification}, "
            f"score={score.total:.3f} "
            f"(f1={score.f1:.2f}, latency={score.latency:.2f}, "
            f"stability={score.stability:.2f}, agent={score.agent:.2f})",
            monitor=monitor_name,
            cycle=cycle,
            changed=observation.changed,
            classification=evaluation.classification,
            score_total=score.total,
            score_f1=score.f1,
            score_latency=score.latency,
            score_stability=score.stability,
            score_agent=score.agent,
        )

        # Increment global cycle counter
        self.state.total_cycles += 1

        # -----------------------------------------------------------------
        # l. Return score
        # -----------------------------------------------------------------
        return score

    def run_all_monitors(self) -> Dict[str, EfficacyScore]:
        """Run one cycle for each monitor that is due for a check.

        Respects per-intent intervals: a monitor is only checked if enough
        time has elapsed since its last observation.

        Returns:
            Dict mapping monitor_name to its EfficacyScore for monitors
            that were checked this round. Monitors that were skipped
            (not yet due) are omitted.
        """
        results: Dict[str, EfficacyScore] = {}

        for monitor_name, monitor in self.state.monitors.items():
            if not self._should_run_monitor(monitor):
                logger.debug(
                    "Skipping monitor '%s': not yet due for check",
                    monitor_name,
                )
                continue

            score = self.run_cycle(monitor_name)
            if score is not None:
                results[monitor_name] = score

        return results

    def get_db_path(self) -> str:
        """Return the path to the isolated test database.

        Returns:
            Absolute path to ``{config.state_dir}/test.db``.
        """
        return os.path.join(self.config.state_dir, "test.db")

    # =========================================================================
    # Internal helpers
    # =========================================================================

    def _should_run_monitor(self, monitor: MonitorState) -> bool:
        """Check if enough time has elapsed since the monitor's last observation.

        Uses the monitor's interval_secs to determine the minimum gap
        between checks. If the monitor has no observations yet, it is
        always due.

        Args:
            monitor: The monitor state to check.

        Returns:
            True if the monitor should be checked now, False otherwise.
        """
        if not monitor.observations:
            return True

        last_obs = monitor.observations[-1]
        if not last_obs.timestamp:
            return True

        try:
            last_time = datetime.fromisoformat(last_obs.timestamp)
            now = datetime.utcnow()
            elapsed_secs = (now - last_time).total_seconds()
        except (ValueError, TypeError):
            # If we cannot parse the timestamp, run the check
            return True

        interval = monitor.interval_secs
        return elapsed_secs >= interval
