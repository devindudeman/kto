"""Structured JSONL + human-readable logging with rotation.

Provides dual-output logging: a human-readable log file for quick
inspection and a structured JSONL file for machine analysis. Both
files support automatic rotation when they exceed a configurable
size limit.
"""

from __future__ import annotations

import json
import logging
import os
import time
from typing import Any, Dict, Optional


class OrchestrationLogger:
    """Dual-output logger for the kto learning loop orchestrator.

    Writes every log entry to two files:
      - ``{state_dir}/orchestrate.log``   -- human-readable text
      - ``{state_dir}/orchestrate.jsonl`` -- structured JSON Lines

    When either file exceeds *max_bytes*, it is rotated by renaming
    it to ``.1`` (any previous ``.1`` is overwritten).
    """

    # Level names consistent with standard logging + custom "learning"
    LEVELS = ("debug", "info", "warn", "error", "learning")

    def __init__(self, state_dir: str, max_bytes: int = 10 * 1024 * 1024) -> None:
        self._state_dir = state_dir
        self._max_bytes = max_bytes

        os.makedirs(state_dir, exist_ok=True)

        self._human_path = os.path.join(state_dir, "orchestrate.log")
        self._jsonl_path = os.path.join(state_dir, "orchestrate.jsonl")

        # Also set up a Python stdlib logger for console output
        self._console = logging.getLogger("kto.orchestrate")
        if not self._console.handlers:
            handler = logging.StreamHandler()
            handler.setFormatter(
                logging.Formatter("%(asctime)s [%(levelname)s] %(message)s")
            )
            self._console.addHandler(handler)
            self._console.setLevel(logging.DEBUG)

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def info(self, msg: str, **fields: Any) -> None:
        """Log an informational message."""
        self._log("info", msg, fields)

    def warn(self, msg: str, **fields: Any) -> None:
        """Log a warning message."""
        self._log("warn", msg, fields)

    def error(self, msg: str, **fields: Any) -> None:
        """Log an error message."""
        self._log("error", msg, fields)

    def learning(self, msg: str, **fields: Any) -> None:
        """Log a learning event.

        Learning events are tagged with level ``learning`` in the JSONL
        output so that downstream analysis pipelines can filter for them
        easily.
        """
        self._log("learning", msg, fields)

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _log(self, level: str, msg: str, fields: Dict[str, Any]) -> None:
        """Route a log entry to both outputs and the console."""
        self._write_jsonl(level, msg, fields)
        self._write_human(level, msg)

        # Mirror to Python's logging for console visibility
        py_level = {
            "debug": logging.DEBUG,
            "info": logging.INFO,
            "warn": logging.WARNING,
            "error": logging.ERROR,
            "learning": logging.INFO,
        }.get(level, logging.INFO)
        self._console.log(py_level, msg)

    def _write_jsonl(self, level: str, msg: str, fields: Dict[str, Any]) -> None:
        """Append a single JSON object to the JSONL log file."""
        self._rotate_if_needed(self._jsonl_path)

        record: Dict[str, Any] = {
            "ts": time.time(),
            "time": time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime()),
            "level": level,
            "msg": msg,
        }
        if fields:
            record["fields"] = fields

        try:
            with open(self._jsonl_path, "a", encoding="utf-8") as f:
                f.write(json.dumps(record, default=str) + "\n")
        except OSError:
            # If we cannot write, swallow silently — logging should never crash
            pass

    def _write_human(self, level: str, msg: str) -> None:
        """Append a human-readable line to the text log file."""
        self._rotate_if_needed(self._human_path)

        timestamp = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime())
        tag = level.upper().ljust(8)
        line = f"{timestamp} [{tag}] {msg}\n"

        try:
            with open(self._human_path, "a", encoding="utf-8") as f:
                f.write(line)
        except OSError:
            pass

    def _rotate_if_needed(self, path: str) -> None:
        """Rotate *path* to ``path.1`` if it exceeds the size limit."""
        try:
            size = os.path.getsize(path)
        except OSError:
            return

        if size < self._max_bytes:
            return

        rotated = path + ".1"
        try:
            # On POSIX this atomically replaces rotated if it exists.
            # On Windows os.replace also works (Python 3.3+).
            os.replace(path, rotated)
        except OSError:
            pass
