#!/usr/bin/env python3
"""Main entry point for the kto learning loop orchestrator.

Handles CLI argument parsing, initialization, and the main loop.
Coordinates the observe -> evaluate -> experiment -> learn cycle
across all configured intents, producing a final report with
learned creation rules.

Usage:
    python scripts/orchestrate.py --intents intents.toml --duration 12
    python scripts/orchestrate.py --intents intents.toml --resume
    python scripts/orchestrate.py --intents intents.toml --dry-run
"""

from __future__ import annotations

import argparse
import os
import signal
import sys
import time

# Allow imports from the scripts/ directory so the orchestrate package is found
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from orchestrate.config import OrchestratorConfig
from orchestrate.state import RunState, MonitorState, load_state, save_state_atomic
from orchestrate.intents import load_intents, validate_intents
from orchestrate.kto_client import KtoClient
from orchestrate.server_bridge import ServerBridge
from orchestrate.knowledge import KnowledgeBase
from orchestrate.cycle import CycleRunner
from orchestrate.report import generate_report
from orchestrate.log import OrchestrationLogger


# ---------------------------------------------------------------------------
# Signal handling
# ---------------------------------------------------------------------------

_shutdown_requested = False


def _handle_signal(signum: int, frame) -> None:
    """Set the global shutdown flag on SIGINT/SIGTERM."""
    global _shutdown_requested
    _shutdown_requested = True


# ---------------------------------------------------------------------------
# CLI argument parsing
# ---------------------------------------------------------------------------


def parse_args(argv: list | None = None) -> argparse.Namespace:
    """Parse command-line arguments.

    Args:
        argv: Argument list (defaults to sys.argv[1:]).

    Returns:
        Parsed argument namespace.
    """
    parser = argparse.ArgumentParser(
        prog="orchestrate",
        description="kto learning loop orchestrator -- discover what makes monitors effective",
    )

    parser.add_argument(
        "--intents",
        required=True,
        metavar="PATH",
        help="Path to intent TOML file defining monitors and mutations",
    )
    parser.add_argument(
        "--duration",
        type=float,
        default=12.0,
        metavar="HOURS",
        help="Total run duration in hours (default: 12.0)",
    )
    parser.add_argument(
        "--state-dir",
        default="/tmp/kto-orchestrate",
        metavar="PATH",
        help="Directory for state, knowledge, and log files (default: /tmp/kto-orchestrate)",
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="Resume from existing state file instead of starting fresh",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show what would happen without executing any kto commands",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Enable verbose logging",
    )
    parser.add_argument(
        "--e2e-server",
        default="http://127.0.0.1:8787",
        metavar="URL",
        help="E2E test server URL (default: http://127.0.0.1:8787)",
    )
    parser.add_argument(
        "--live-validate",
        action="store_true",
        help="Run live mode as validation-only (no experiments)",
    )
    parser.add_argument(
        "--kto-binary",
        default="kto",
        metavar="PATH",
        help="Path to kto binary (default: kto)",
    )

    return parser.parse_args(argv)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _build_config(args: argparse.Namespace) -> OrchestratorConfig:
    """Build an OrchestratorConfig from parsed CLI arguments."""
    return OrchestratorConfig(
        intents_path=args.intents,
        duration_hours=args.duration,
        state_dir=args.state_dir,
        e2e_server_url=args.e2e_server,
        resume=args.resume,
        dry_run=args.dry_run,
        verbose=args.verbose,
        live_validate=args.live_validate,
        kto_binary=args.kto_binary,
    )


def _state_file_path(state_dir: str) -> str:
    """Return the canonical path for the run state file."""
    return os.path.join(state_dir, "state.json")


def _knowledge_file_path(state_dir: str) -> str:
    """Return the canonical path for the knowledge base file."""
    return os.path.join(state_dir, "knowledge.json")


def _min_interval(state: RunState) -> int:
    """Return the minimum check interval across all monitors.

    This determines how long the main loop sleeps between rounds.
    Defaults to 60 seconds if no monitors have an interval set.
    """
    intervals = [
        mon.interval_secs
        for mon in state.monitors.values()
        if mon.interval_secs > 0
    ]
    return min(intervals) if intervals else 60


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main(argv: list | None = None) -> None:
    """Entry point for the learning loop orchestrator."""
    global _shutdown_requested

    args = parse_args(argv)
    config = _build_config(args)

    # Ensure state directory exists
    os.makedirs(config.state_dir, exist_ok=True)

    # ------------------------------------------------------------------
    # 1. Set up logging
    # ------------------------------------------------------------------
    log = OrchestrationLogger(config.state_dir, max_bytes=config.log_max_bytes)
    log.info(
        "Orchestrator starting",
        intents_path=config.intents_path,
        duration_hours=config.duration_hours,
        state_dir=config.state_dir,
        resume=config.resume,
        dry_run=config.dry_run,
    )

    # ------------------------------------------------------------------
    # 2. Load and validate intents
    # ------------------------------------------------------------------
    print(f"Loading intents from {config.intents_path} ...")
    try:
        intents = load_intents(config.intents_path)
    except (FileNotFoundError, ValueError) as exc:
        log.error(f"Failed to load intents: {exc}")
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(1)

    errors = validate_intents(intents)
    if errors:
        log.error(f"Intent validation failed with {len(errors)} error(s)")
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        sys.exit(1)

    print(f"Loaded {len(intents)} intent(s)")
    for intent in intents:
        print(f"  - {intent.name} ({intent.intent_type}, {intent.mode})")
    log.info(f"Validated {len(intents)} intent(s)")

    # Determine run mode from intents (use first intent's mode, or "e2e")
    run_mode = intents[0].mode if intents else "e2e"

    # ------------------------------------------------------------------
    # 3. Dry-run: show what would happen and exit
    # ------------------------------------------------------------------
    if config.dry_run:
        print("\n--- DRY RUN ---")
        print(f"Duration:     {config.duration_hours}h")
        print(f"State dir:    {config.state_dir}")
        print(f"E2E server:   {config.e2e_server_url}")
        print(f"kto binary:   {config.kto_binary}")
        print(f"Run mode:     {run_mode}")
        print(f"\nWould create {len(intents)} watch(es):")
        for intent in intents:
            watch_name = f"orch_{intent.name}"
            print(f"  - {watch_name}: {intent.url}")
            print(f"    engine={intent.engine}, extraction={intent.extraction}, "
                  f"interval={intent.interval_secs}s")
            if intent.agent_instructions:
                instr_preview = intent.agent_instructions[:80]
                if len(intent.agent_instructions) > 80:
                    instr_preview += "..."
                print(f"    agent_instructions: {instr_preview}")
            if intent.mutations:
                print(f"    mutations: {len(intent.mutations)} step(s)")
                for mut in intent.mutations:
                    print(f"      cycle {mut.cycle}: {mut.field}={mut.value} "
                          f"({mut.description or 'no description'})")
        print("\n--- END DRY RUN ---")
        sys.exit(0)

    # ------------------------------------------------------------------
    # 4. Initialize or resume state
    # ------------------------------------------------------------------
    state_path = _state_file_path(config.state_dir)

    if config.resume:
        state = load_state(state_path)
        if state is not None:
            print(f"Resumed state from {state_path} (run_id={state.run_id}, "
                  f"cycles={state.total_cycles})")
            log.info(
                f"Resumed run {state.run_id} with {state.total_cycles} prior cycles",
                run_id=state.run_id,
                total_cycles=state.total_cycles,
            )
        else:
            print("No existing state file found, starting fresh")
            state = RunState(mode=run_mode)
            log.info(f"No state to resume, created new run {state.run_id}")
    else:
        state = RunState(mode=run_mode)
        log.info(f"Created new run {state.run_id}", run_id=state.run_id)

    print(f"Run ID: {state.run_id}")

    # ------------------------------------------------------------------
    # 5. Set up knowledge base
    # ------------------------------------------------------------------
    kb_path = _knowledge_file_path(config.state_dir)
    knowledge = KnowledgeBase(kb_path)
    if os.path.exists(kb_path):
        loaded = knowledge.load()
        if loaded:
            removed = knowledge.apply_decay()
            print(f"Loaded knowledge base: {knowledge.rule_count()} rule(s) "
                  f"({removed} decayed)")
            log.info(
                f"Loaded knowledge base with {knowledge.rule_count()} rules, "
                f"{removed} removed by decay",
                rules=knowledge.rule_count(),
                decayed=removed,
            )
        else:
            print("Knowledge base exists but could not be loaded, starting fresh")
            log.warn("Failed to load knowledge base, starting fresh")
    else:
        print("No existing knowledge base, starting fresh")

    # ------------------------------------------------------------------
    # 6. Set up kto client and server bridge
    # ------------------------------------------------------------------
    kto = KtoClient(config)

    server: ServerBridge | None = None
    has_e2e = any(i.mode == "e2e" for i in intents)

    if has_e2e:
        server = ServerBridge(config.e2e_server_url)
        if server.is_available():
            print(f"E2E test server available at {config.e2e_server_url}")
            log.info(f"E2E server reachable at {config.e2e_server_url}")
            # Reset server state to a clean baseline
            if server.reset():
                log.info("E2E server state reset to defaults")
            else:
                log.warn("Failed to reset E2E server state")
        else:
            log.error(
                f"E2E test server not reachable at {config.e2e_server_url}. "
                f"Start it with: python3 tests/e2e/harness/server.py"
            )
            print(
                f"ERROR: E2E test server not reachable at {config.e2e_server_url}\n"
                f"Start it with: python3 tests/e2e/harness/server.py",
                file=sys.stderr,
            )
            sys.exit(1)

    # ------------------------------------------------------------------
    # 7. Create monitors in kto for each intent (if not resuming)
    # ------------------------------------------------------------------
    db_path = os.path.join(config.state_dir, "test.db")

    if not config.resume or not state.monitors:
        print(f"\nCreating {len(intents)} watch(es) in kto ...")
        for intent in intents:
            watch_name = f"{state.run_id}_{intent.name}"

            result = kto.create_watch(
                name=watch_name,
                url=intent.url,
                engine=intent.engine,
                extraction=intent.extraction,
                interval_secs=intent.interval_secs,
                agent_instructions=intent.agent_instructions,
                selector=intent.selector,
                tags=intent.tags,
                db_path=db_path,
            )

            if result.get("ok"):
                print(f"  Created: {watch_name}")
                log.info(f"Created watch '{watch_name}' for intent '{intent.name}'",
                         watch_name=watch_name, intent=intent.name)
            else:
                error_msg = result.get("error", "unknown error")
                print(f"  FAILED: {watch_name} -- {error_msg}", file=sys.stderr)
                log.error(
                    f"Failed to create watch '{watch_name}': {error_msg}",
                    watch_name=watch_name,
                    error=error_msg,
                )
                # Continue with remaining intents; the monitor will be missing
                # from state and skipped during cycles
                continue

            # Initialize MonitorState
            monitor = MonitorState(
                name=intent.name,
                watch_name=watch_name,
                intent_type=intent.intent_type,
                domain_class=intent.domain_class,
                mode=intent.mode,
                interval_secs=intent.interval_secs,
                current_config={
                    "engine": intent.engine,
                    "extraction": intent.extraction,
                    "interval_secs": str(intent.interval_secs),
                },
            )
            if intent.selector:
                monitor.current_config["selector"] = intent.selector
            if intent.agent_instructions:
                monitor.current_config["agent_instructions"] = intent.agent_instructions

            state.monitors[intent.name] = monitor

        if not state.monitors:
            log.error("No monitors were created successfully, aborting")
            print("ERROR: No monitors were created. Check logs for details.",
                  file=sys.stderr)
            sys.exit(1)

        # Save initial state
        save_state_atomic(state, state_path)
        print(f"\nInitialized {len(state.monitors)} monitor(s)")
    else:
        print(f"Resuming with {len(state.monitors)} existing monitor(s)")

    # ------------------------------------------------------------------
    # 8. Create CycleRunner
    # ------------------------------------------------------------------
    cycle_runner = CycleRunner(
        config=config,
        state=state,
        knowledge=knowledge,
        kto=kto,
        server=server,
        logger_=log,
        intents=intents,
    )

    # ------------------------------------------------------------------
    # 9. Install signal handlers for graceful shutdown
    # ------------------------------------------------------------------
    signal.signal(signal.SIGINT, _handle_signal)
    signal.signal(signal.SIGTERM, _handle_signal)

    # ------------------------------------------------------------------
    # 10. Main loop
    # ------------------------------------------------------------------
    end_time = time.time() + config.duration_hours * 3600
    sleep_interval = _min_interval(state)
    last_save_time = time.time()

    print(f"\nStarting main loop (duration={config.duration_hours}h, "
          f"sleep={sleep_interval}s between rounds)")
    print("Press Ctrl+C to stop gracefully\n")
    log.info(
        f"Main loop started: end_time in {config.duration_hours}h, "
        f"sleep_interval={sleep_interval}s",
        duration_hours=config.duration_hours,
        sleep_interval=sleep_interval,
    )

    try:
        while time.time() < end_time and not _shutdown_requested:
            round_start = time.time()

            # Run one cycle across all due monitors
            results = cycle_runner.run_all_monitors()

            # Print a brief progress line
            if results:
                scores_str = ", ".join(
                    f"{name}={score.total:.3f}"
                    for name, score in results.items()
                )
                print(f"[cycle {state.total_cycles}] Checked {len(results)} monitor(s): "
                      f"{scores_str}")
            else:
                print(f"[cycle {state.total_cycles}] No monitors due for check")

            # Save state periodically (every 60 seconds)
            now = time.time()
            if now - last_save_time >= 60:
                save_state_atomic(state, state_path)
                last_save_time = now

            # Sleep until next round, checking for shutdown every second
            elapsed = time.time() - round_start
            remaining_sleep = max(0, sleep_interval - elapsed)
            sleep_end = time.time() + remaining_sleep
            while time.time() < sleep_end and not _shutdown_requested:
                time.sleep(min(1.0, sleep_end - time.time()))

    except KeyboardInterrupt:
        # Belt-and-suspenders: handle KeyboardInterrupt even if signal
        # handler didn't fire (e.g., during sleep on some platforms)
        _shutdown_requested = True

    if _shutdown_requested:
        print("\nShutdown requested, saving state ...")
        log.info("Graceful shutdown requested")

    # ------------------------------------------------------------------
    # 11. Finalize: save state, generate report, save knowledge
    # ------------------------------------------------------------------
    print("\nFinalizing run ...")

    save_state_atomic(state, state_path)
    log.info(f"Final state saved to {state_path}", total_cycles=state.total_cycles)
    print(f"State saved ({state.total_cycles} total cycles)")

    knowledge.save()
    log.info(
        f"Knowledge base saved with {knowledge.rule_count()} rule(s)",
        rules=knowledge.rule_count(),
    )
    print(f"Knowledge base saved ({knowledge.rule_count()} rules)")

    print("Generating report ...")
    report_text = generate_report(state, knowledge, config.state_dir)
    print(report_text)

    # ------------------------------------------------------------------
    # 12. Clean up: delete test watches (unless resuming)
    # ------------------------------------------------------------------
    if not config.resume:
        print("\nCleaning up test watches ...")
        for monitor_name, monitor in state.monitors.items():
            result = kto.delete_watch(monitor.watch_name, db_path=db_path)
            if result.get("ok"):
                log.info(f"Deleted watch '{monitor.watch_name}'",
                         watch_name=monitor.watch_name)
            else:
                error_msg = result.get("error", "unknown")
                log.warn(
                    f"Failed to delete watch '{monitor.watch_name}': {error_msg}",
                    watch_name=monitor.watch_name,
                    error=error_msg,
                )
        print("Cleanup complete")
    else:
        print("\nSkipping cleanup (--resume mode, watches preserved for inspection)")

    # ------------------------------------------------------------------
    # Summary
    # ------------------------------------------------------------------
    print(f"\nRun {state.run_id} complete")
    print(f"  Total cycles:   {state.total_cycles}")
    print(f"  Monitors:       {len(state.monitors)}")
    print(f"  Rules learned:  {knowledge.rule_count()}")
    print(f"  State:          {state_path}")
    print(f"  Knowledge:      {kb_path}")
    print(f"  Report:         {os.path.join(config.state_dir, 'report.txt')}")
    print(f"  Logs:           {os.path.join(config.state_dir, 'orchestrate.log')}")

    log.info(
        f"Run {state.run_id} complete: {state.total_cycles} cycles, "
        f"{knowledge.rule_count()} rules learned",
        run_id=state.run_id,
        total_cycles=state.total_cycles,
        rules_learned=knowledge.rule_count(),
    )


if __name__ == "__main__":
    main()
