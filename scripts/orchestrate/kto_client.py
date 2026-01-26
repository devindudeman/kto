"""kto CLI wrapper with timeouts for the learning loop orchestrator.

Wraps the kto binary and provides methods for creating watches, running
checks, listing watches, and deleting watches. All subprocess calls use
explicit timeouts to prevent the orchestrator from hanging.
"""

from __future__ import annotations

import json
import logging
import os
import subprocess
from datetime import datetime
from typing import Dict, List, Optional

from .config import OrchestratorConfig
from .state import Observation

logger = logging.getLogger(__name__)


class KtoClient:
    """Thin wrapper around the kto CLI binary."""

    def __init__(self, config: OrchestratorConfig) -> None:
        self.config = config

    # =========================================================================
    # Internal helper
    # =========================================================================

    def _run_kto(
        self,
        args: List[str],
        db_path: Optional[str] = None,
        timeout: Optional[int] = None,
    ) -> subprocess.CompletedProcess:
        """Run a kto subprocess with timeout and optional DB isolation.

        Args:
            args: Arguments to pass after the kto binary name.
            db_path: If set, KTO_DB env var is used to isolate the database.
            timeout: Seconds before the subprocess is killed.
                     Defaults to config.kto_timeout_secs.

        Returns:
            The CompletedProcess result (stdout/stderr captured as text).

        Raises:
            subprocess.TimeoutExpired: If the command exceeds the timeout.
            subprocess.SubprocessError: On other subprocess failures.
        """
        if timeout is None:
            timeout = self.config.kto_timeout_secs

        cmd = [self.config.kto_binary] + args

        env = os.environ.copy()
        if db_path:
            env["KTO_DB"] = db_path

        logger.debug("Running: %s (timeout=%ds, db=%s)", " ".join(cmd), timeout, db_path)

        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )

        if result.returncode != 0:
            logger.warning(
                "kto exited with code %d: %s",
                result.returncode,
                result.stderr.strip()[:500] if result.stderr else "(no stderr)",
            )

        return result

    # =========================================================================
    # Public API
    # =========================================================================

    def create_watch(
        self,
        name: str,
        url: str,
        engine: str = "http",
        extraction: str = "auto",
        interval_secs: int = 300,
        agent_instructions: Optional[str] = None,
        selector: Optional[str] = None,
        tags: Optional[List[str]] = None,
        db_path: Optional[str] = None,
    ) -> Dict:
        """Create a new kto watch via ``kto new``.

        Args:
            name: Human-readable watch name.
            url: URL to monitor.
            engine: Fetch engine (http, playwright, rss, shell).
            extraction: Extraction strategy (auto, selector, full, meta, rss, json_ld).
            interval_secs: Check interval in seconds.
            agent_instructions: Optional AI agent instructions.
            selector: Optional CSS selector for extraction.
            tags: Optional list of tags.
            db_path: Optional database path for isolation.

        Returns:
            Dict with ``ok`` (bool), ``name``, and optionally ``error``.
        """
        args = [
            "new",
            url,
            "--name", name,
            "--yes",
            "--interval", str(interval_secs),
        ]

        # Engine flag
        if engine and engine != "http":
            if engine == "playwright":
                args.append("--js")
            elif engine == "rss":
                args.append("--rss")
            elif engine == "shell":
                args.append("--shell")

        # Extraction flag
        if extraction and extraction != "auto":
            if extraction == "selector" and selector:
                args.extend(["--selector", selector])
            elif extraction == "full":
                args.append("--full")
            elif extraction == "json_ld":
                args.append("--json-ld")
            elif extraction == "meta":
                args.append("--meta")
            elif extraction == "rss":
                args.append("--rss")
        elif selector:
            # Selector provided without explicit extraction=selector
            args.extend(["--selector", selector])

        # Agent instructions
        if agent_instructions:
            args.extend(["--agent", "--agent-instructions", agent_instructions])

        # Tags
        if tags:
            for tag in tags:
                args.extend(["--tag", tag])

        try:
            result = self._run_kto(args, db_path=db_path)
            if result.returncode == 0:
                logger.info("Created watch %r", name)
                return {"ok": True, "name": name}
            else:
                error_msg = result.stderr.strip() if result.stderr else f"exit code {result.returncode}"
                logger.error("Failed to create watch %r: %s", name, error_msg)
                return {"ok": False, "name": name, "error": error_msg}
        except subprocess.TimeoutExpired:
            logger.error("Timed out creating watch %r", name)
            return {"ok": False, "name": name, "error": "timeout"}
        except Exception as exc:
            logger.error("Exception creating watch %r: %s", name, exc)
            return {"ok": False, "name": name, "error": str(exc)}

    def run_check(
        self,
        watch_name: str,
        db_path: Optional[str] = None,
    ) -> Observation:
        """Run a check on an existing watch and return an Observation.

        Executes ``kto test <name> --json`` and parses the JSON output into
        an Observation dataclass. On any failure (timeout, bad JSON, non-zero
        exit), returns an Observation with the ``error`` field set.

        Args:
            watch_name: Name of the watch to check.
            db_path: Optional database path for isolation.

        Returns:
            An Observation populated from the check result.
        """
        timestamp = datetime.utcnow().isoformat()

        try:
            result = self._run_kto(
                ["test", watch_name, "--json"],
                db_path=db_path,
            )
        except subprocess.TimeoutExpired:
            logger.error("Timed out checking watch %r", watch_name)
            return Observation(
                timestamp=timestamp,
                error="timeout",
            )
        except Exception as exc:
            logger.error("Exception checking watch %r: %s", watch_name, exc)
            return Observation(
                timestamp=timestamp,
                error=str(exc),
            )

        # Non-zero exit without timeout
        if result.returncode != 0:
            error_msg = result.stderr.strip() if result.stderr else f"exit code {result.returncode}"
            return Observation(
                timestamp=timestamp,
                error=error_msg,
            )

        # Parse JSON output
        try:
            data = json.loads(result.stdout)
        except (json.JSONDecodeError, ValueError) as exc:
            logger.error(
                "Failed to parse JSON from kto test %r: %s\nstdout: %s",
                watch_name,
                exc,
                result.stdout[:500],
            )
            return Observation(
                timestamp=timestamp,
                error=f"json_parse_error: {exc}",
            )

        # Build Observation from parsed data
        changed = data.get("changed", False)
        content_hash = data.get("content_hash") or data.get("hash")
        diff_snippet = data.get("diff_snippet") or data.get("diff")

        # Truncate diff to a reasonable snippet length
        if diff_snippet and len(diff_snippet) > 2000:
            diff_snippet = diff_snippet[:2000] + "\n... (truncated)"

        # Agent analysis fields (may be absent if agent is disabled)
        agent_data = data.get("agent") or {}
        agent_notified = agent_data.get("notified") if agent_data else data.get("agent_notified")
        agent_title = agent_data.get("title") if agent_data else data.get("agent_title")
        agent_summary = agent_data.get("summary") if agent_data else data.get("agent_summary")

        return Observation(
            timestamp=timestamp,
            changed=changed,
            content_hash=content_hash,
            diff_snippet=diff_snippet,
            agent_notified=agent_notified,
            agent_title=agent_title,
            agent_summary=agent_summary,
            raw_json=data,
        )

    def list_watches(
        self,
        db_path: Optional[str] = None,
    ) -> List[Dict]:
        """List all watches via ``kto list --json``.

        Args:
            db_path: Optional database path for isolation.

        Returns:
            List of dicts, each representing a watch. Empty list on error.
        """
        try:
            result = self._run_kto(["list", "--json"], db_path=db_path)
        except subprocess.TimeoutExpired:
            logger.error("Timed out listing watches")
            return []
        except Exception as exc:
            logger.error("Exception listing watches: %s", exc)
            return []

        if result.returncode != 0:
            logger.warning("kto list failed: %s", result.stderr.strip()[:500] if result.stderr else "")
            return []

        try:
            data = json.loads(result.stdout)
            if isinstance(data, list):
                return data
            # Some versions may wrap in an object
            if isinstance(data, dict) and "watches" in data:
                return data["watches"]
            return []
        except (json.JSONDecodeError, ValueError) as exc:
            logger.error("Failed to parse kto list JSON: %s", exc)
            return []

    def delete_watch(
        self,
        name: str,
        db_path: Optional[str] = None,
    ) -> Dict:
        """Delete a watch via ``kto delete <name> --yes``.

        Args:
            name: Watch name to delete.
            db_path: Optional database path for isolation.

        Returns:
            Dict with ``ok`` (bool) and optionally ``error``.
        """
        try:
            result = self._run_kto(
                ["delete", name, "--yes"],
                db_path=db_path,
            )
            if result.returncode == 0:
                logger.info("Deleted watch %r", name)
                return {"ok": True}
            else:
                error_msg = result.stderr.strip() if result.stderr else f"exit code {result.returncode}"
                logger.warning("Failed to delete watch %r: %s", name, error_msg)
                return {"ok": False, "error": error_msg}
        except subprocess.TimeoutExpired:
            logger.error("Timed out deleting watch %r", name)
            return {"ok": False, "error": "timeout"}
        except Exception as exc:
            logger.error("Exception deleting watch %r: %s", name, exc)
            return {"ok": False, "error": str(exc)}
