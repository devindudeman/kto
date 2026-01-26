"""TOML intent loader and validation.

Loads intent definitions from TOML files and validates them against
the expected schema. Supports Python 3.11+ tomllib, the tomli
backport, or a minimal built-in TOML parser as a last resort.
"""

from __future__ import annotations

from typing import Any, Dict, List

from .config import IntentDefinition, MutationStep


# ---------------------------------------------------------------------------
# TOML parsing — prefer stdlib tomllib, fall back to tomli, then built-in
# ---------------------------------------------------------------------------

_toml_loads = None

try:
    import tomllib  # Python 3.11+

    def _stdlib_loads(text: str) -> dict:
        return tomllib.loads(text)

    _toml_loads = _stdlib_loads
except ModuleNotFoundError:
    try:
        import tomli  # type: ignore[import-untyped]

        def _tomli_loads(text: str) -> dict:
            return tomli.loads(text)

        _toml_loads = _tomli_loads
    except ModuleNotFoundError:
        pass


# ---------------------------------------------------------------------------
# Minimal TOML parser (fallback for environments without tomllib/tomli)
# ---------------------------------------------------------------------------

def _minimal_toml_parse(text: str) -> dict:
    """Parse the subset of TOML used by intent definition files.

    Supports:
      - [table] headers
      - [[array_of_tables]] headers
      - key = "string" (basic strings, including multi-line triple-quoted)
      - key = integer
      - key = true / false
      - key = ["string", ...] (inline string arrays)
      - # line comments
    """
    root: Dict[str, Any] = {}
    current: Dict[str, Any] = root
    current_path: List[str] = []
    array_tables: Dict[str, list] = {}
    lines = text.splitlines()
    idx = 0

    def _set_nested(d: dict, keys: List[str], value: Any) -> None:
        for k in keys[:-1]:
            d = d.setdefault(k, {})
        d[keys[-1]] = value

    def _get_nested(d: dict, keys: List[str]) -> dict:
        for k in keys:
            d = d.setdefault(k, {})
        return d

    def _parse_value(raw: str) -> Any:
        raw = raw.strip()
        if not raw:
            return ""
        # Boolean
        if raw == "true":
            return True
        if raw == "false":
            return False
        # Integer
        try:
            return int(raw)
        except ValueError:
            pass
        # Float
        try:
            return float(raw)
        except ValueError:
            pass
        # Quoted string
        if (raw.startswith('"') and raw.endswith('"')) or (
            raw.startswith("'") and raw.endswith("'")
        ):
            return raw[1:-1]
        # Inline array
        if raw.startswith("[") and raw.endswith("]"):
            inner = raw[1:-1].strip()
            if not inner:
                return []
            items = []
            for item in inner.split(","):
                item = item.strip()
                if not item:
                    continue
                items.append(_parse_value(item))
            return items
        # Bare string (shouldn't happen in valid TOML, but be lenient)
        return raw

    def _parse_multiline_string(start_idx: int, line_remainder: str) -> tuple:
        """Parse a triple-quoted multi-line basic string.

        Returns (parsed_string, next_line_index).
        """
        # Determine quote style
        if line_remainder.startswith('"""'):
            quote = '"""'
            content_start = line_remainder[3:]
        elif line_remainder.startswith("'''"):
            quote = "'''"
            content_start = line_remainder[3:]
        else:
            raise ValueError("Not a multi-line string")

        # Check if closing quotes are on the same line
        close_pos = content_start.find(quote)
        if close_pos != -1:
            return content_start[:close_pos], start_idx + 1

        parts = [content_start]
        i = start_idx + 1
        while i < len(lines):
            ln = lines[i]
            close_pos = ln.find(quote)
            if close_pos != -1:
                parts.append(ln[:close_pos])
                return "\n".join(parts), i + 1
            parts.append(ln)
            i += 1
        # Unterminated — return what we have
        return "\n".join(parts), i

    while idx < len(lines):
        line = lines[idx]
        stripped = line.strip()

        # Skip empty lines and comments
        if not stripped or stripped.startswith("#"):
            idx += 1
            continue

        # Array of tables: [[section.path]]
        if stripped.startswith("[[") and stripped.endswith("]]"):
            path_str = stripped[2:-2].strip()
            parts = [p.strip() for p in path_str.split(".")]
            table_key = ".".join(parts)

            if table_key not in array_tables:
                array_tables[table_key] = []
                # Ensure parent path exists and set the array
                if len(parts) > 1:
                    parent = _get_nested(root, parts[:-1])
                    parent[parts[-1]] = array_tables[table_key]
                else:
                    root[parts[0]] = array_tables[table_key]

            new_table: Dict[str, Any] = {}
            array_tables[table_key].append(new_table)
            current = new_table
            current_path = parts
            idx += 1
            continue

        # Table header: [section.path]
        if stripped.startswith("[") and stripped.endswith("]"):
            path_str = stripped[1:-1].strip()
            parts = [p.strip() for p in path_str.split(".")]

            # Check if this is a sub-table of an array entry
            # e.g., [[intents]] then [[intents.mutations]] means mutations
            # is an array of tables under the last intents entry.
            # But for [intents.mutations], it's a regular table.
            current = _get_nested(root, parts)
            current_path = parts
            idx += 1
            continue

        # Key = Value
        if "=" in stripped:
            eq_pos = stripped.index("=")
            key = stripped[:eq_pos].strip()
            val_raw = stripped[eq_pos + 1:].strip()

            # Strip inline comments (not inside strings)
            if val_raw and not val_raw.startswith('"') and not val_raw.startswith("'"):
                comment_pos = val_raw.find("#")
                if comment_pos != -1:
                    val_raw = val_raw[:comment_pos].strip()

            # Multi-line strings
            if val_raw.startswith('"""') or val_raw.startswith("'''"):
                value, idx = _parse_multiline_string(idx, val_raw)
                current[key] = value
                continue

            current[key] = _parse_value(val_raw)
            idx += 1
            continue

        idx += 1

    return root


if _toml_loads is None:
    _toml_loads = _minimal_toml_parse


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def load_intents(path: str) -> List[IntentDefinition]:
    """Load intent definitions from a TOML file.

    Args:
        path: Filesystem path to the TOML intent definitions file.

    Returns:
        A list of IntentDefinition objects parsed from the file.

    Raises:
        FileNotFoundError: If the file does not exist.
        ValueError: If the TOML cannot be parsed.
    """
    with open(path, "r", encoding="utf-8") as f:
        text = f.read()

    try:
        data = _toml_loads(text)
    except Exception as exc:
        raise ValueError(f"Failed to parse TOML from {path}: {exc}") from exc

    # Extract mode from [meta] if present, used as default for intents
    meta = data.get("meta", {})
    default_mode = meta.get("mode", "e2e")

    raw_intents = data.get("intents", [])
    if not isinstance(raw_intents, list):
        raise ValueError(
            f"Expected 'intents' to be an array of tables, got {type(raw_intents).__name__}"
        )

    result: List[IntentDefinition] = []
    for raw in raw_intents:
        if not isinstance(raw, dict):
            raise ValueError(
                f"Expected each intent to be a table, got {type(raw).__name__}"
            )
        # Apply default mode from [meta] if not set on the intent itself
        if "mode" not in raw:
            raw["mode"] = default_mode
        result.append(IntentDefinition.from_dict(raw))

    return result


def validate_intents(intents: List[IntentDefinition]) -> List[str]:
    """Validate a list of intent definitions.

    Returns a list of human-readable error messages. An empty list
    means all intents are valid.

    Validation rules:
      - Each intent must have a non-empty name, url, and intent_type.
      - E2E intents (mode == "e2e") should have at least one mutation.
      - Mutation cycles must be positive integers.
      - expected_detections should not be negative.
      - Intent names must be unique.
    """
    errors: List[str] = []
    seen_names: set = set()

    for i, intent in enumerate(intents):
        prefix = f"intent[{i}]"
        if intent.name:
            prefix = f"intent '{intent.name}'"

        # Required fields
        if not intent.name:
            errors.append(f"{prefix}: missing required field 'name'")
        if not intent.url:
            errors.append(f"{prefix}: missing required field 'url'")
        if not intent.intent_type:
            errors.append(f"{prefix}: missing required field 'intent_type'")

        # Duplicate names
        if intent.name:
            if intent.name in seen_names:
                errors.append(f"{prefix}: duplicate intent name '{intent.name}'")
            seen_names.add(intent.name)

        # E2E intents should have mutations
        if intent.mode == "e2e" and not intent.mutations:
            errors.append(
                f"{prefix}: E2E intent should have at least one mutation defined"
            )

        # Validate mutations
        for j, mut in enumerate(intent.mutations):
            mut_prefix = f"{prefix} mutation[{j}]"

            if mut.cycle <= 0:
                errors.append(
                    f"{mut_prefix}: cycle must be a positive integer, got {mut.cycle}"
                )

            if not mut.field:
                errors.append(f"{mut_prefix}: missing required field 'field'")

        # expected_detections should be non-negative
        if intent.expected_detections < 0:
            errors.append(
                f"{prefix}: expected_detections must be non-negative, "
                f"got {intent.expected_detections}"
            )

    return errors
