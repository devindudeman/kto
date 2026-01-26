"""E2E test server mutation API client.

Communicates with the test server (tests/e2e/harness/server.py) to control
test mutations. Uses only Python standard library (urllib, json, logging).
"""

from __future__ import annotations

import json
import logging
from typing import Any, Dict, Optional
from urllib.error import URLError
from urllib.request import Request, urlopen

from .config import MutationStep

logger = logging.getLogger(__name__)

# Fields whose server-side types are lists
_LIST_FIELDS = {"releases", "articles"}

# Fields whose server-side types are bools
_BOOL_FIELDS = {
    "include_timestamp",
    "include_tracking",
    "include_random_id",
    "return_empty",
    "return_malformed",
}

# Fields whose server-side types are optional ints
_OPTIONAL_INT_FIELDS = {"error_code"}

# Fields whose server-side types are floats
_FLOAT_FIELDS = {"delay_seconds"}


def _coerce_value(field_name: str, raw_value: str) -> Any:
    """Coerce a string value to the type expected by the test server.

    The MutationStep dataclass stores all values as strings. The test server
    expects native JSON types, so we convert based on the field name.

    Args:
        field_name: The server state field being set.
        raw_value: The string representation of the value.

    Returns:
        The value coerced to the appropriate Python type.
    """
    if field_name in _LIST_FIELDS:
        # Try parsing as JSON first (handles lists of dicts for articles)
        try:
            parsed = json.loads(raw_value)
            if isinstance(parsed, list):
                return parsed
        except (json.JSONDecodeError, ValueError):
            pass
        # Fallback: comma-separated strings (for releases like "v1.0.0,v2.0.0")
        return [item.strip() for item in raw_value.split(",") if item.strip()]

    if field_name in _BOOL_FIELDS:
        return raw_value.lower() in ("true", "1", "yes")

    if field_name in _OPTIONAL_INT_FIELDS:
        # Support clearing error_code with empty/none/null
        if raw_value.lower() in ("", "none", "null"):
            return None
        try:
            return int(raw_value)
        except ValueError:
            logger.warning("Cannot convert %r to int for field %r, using None", raw_value, field_name)
            return None

    if field_name in _FLOAT_FIELDS:
        try:
            return float(raw_value)
        except ValueError:
            logger.warning("Cannot convert %r to float for field %r, using 0.0", raw_value, field_name)
            return 0.0

    # Default: keep as string (product_price, product_stock, product_name,
    # status, status_message, ad_variant)
    return raw_value


class ServerBridge:
    """Client for the E2E test server mutation API.

    Communicates with the test server at the given base URL to get/set server
    state and apply mutation steps during orchestrated E2E test runs.

    Example::

        bridge = ServerBridge("http://127.0.0.1:8787")
        if bridge.is_available():
            bridge.reset()
            bridge.update_state(product_price="$49.99", product_stock="IN STOCK")
            state = bridge.get_state()
            print(state["product_price"])  # "$49.99"
    """

    def __init__(self, base_url: str = "http://127.0.0.1:8787") -> None:
        self.base_url = base_url.rstrip("/")

    # =========================================================================
    # Internal helpers
    # =========================================================================

    def _request(
        self,
        method: str,
        path: str,
        body: Optional[dict] = None,
        timeout: float = 10.0,
    ) -> Optional[dict]:
        """Send an HTTP request to the test server and return parsed JSON.

        Args:
            method: HTTP method ("GET" or "POST").
            path: URL path (e.g. "/api/state").
            body: Optional JSON body for POST requests.
            timeout: Request timeout in seconds.

        Returns:
            Parsed JSON response as a dict, or None on failure.
        """
        url = f"{self.base_url}{path}"
        data = None
        headers = {}

        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"

        req = Request(url, data=data, headers=headers, method=method)

        try:
            with urlopen(req, timeout=timeout) as resp:
                response_body = resp.read().decode("utf-8")
                return json.loads(response_body)
        except URLError as exc:
            logger.debug("Request to %s failed: %s", url, exc)
            return None
        except json.JSONDecodeError as exc:
            logger.warning("Invalid JSON from %s: %s", url, exc)
            return None
        except Exception as exc:
            logger.debug("Unexpected error requesting %s: %s", url, exc)
            return None

    # =========================================================================
    # Public API
    # =========================================================================

    def get_state(self) -> dict:
        """Get the current server state.

        Returns:
            Dict of all server state fields, or empty dict on failure.
        """
        result = self._request("GET", "/api/state")
        if result is None:
            logger.error("Failed to get server state from %s", self.base_url)
            return {}
        return result

    def update_state(self, **kwargs: Any) -> bool:
        """Update server state fields.

        Args:
            **kwargs: Field names and values to update. Values should already
                      be the correct Python types (use apply_mutation for
                      automatic type coercion from strings).

        Returns:
            True if the update succeeded, False otherwise.
        """
        if not kwargs:
            logger.warning("update_state called with no arguments")
            return True

        result = self._request("POST", "/api/state", body=kwargs)
        if result is None:
            logger.error("Failed to update server state: %s", kwargs)
            return False

        if result.get("status") == "ok":
            logger.debug("Server state updated: %s", list(kwargs.keys()))
            return True

        logger.warning("Server returned unexpected response: %s", result)
        return False

    def reset(self) -> bool:
        """Reset server state to defaults.

        Returns:
            True if the reset succeeded, False otherwise.
        """
        result = self._request("POST", "/api/reset")
        if result is None:
            logger.error("Failed to reset server state")
            return False

        if result.get("status") == "reset":
            logger.debug("Server state reset to defaults")
            return True

        logger.warning("Server returned unexpected reset response: %s", result)
        return False

    def apply_mutation(self, mutation: MutationStep) -> bool:
        """Apply a MutationStep by updating the appropriate server field.

        Handles type coercion from the MutationStep's string value to the
        type expected by the server (lists for releases/articles, bools for
        flags, ints for error_code, floats for delay_seconds).

        Args:
            mutation: The MutationStep to apply.

        Returns:
            True if the mutation was applied successfully, False otherwise.
        """
        if not mutation.field:
            logger.warning("MutationStep has empty field, skipping: %s", mutation.description)
            return False

        coerced_value = _coerce_value(mutation.field, mutation.value)

        logger.info(
            "Applying mutation [cycle %d]: %s = %r (%s)",
            mutation.cycle,
            mutation.field,
            coerced_value,
            mutation.description or "no description",
        )

        return self.update_state(**{mutation.field: coerced_value})

    def is_available(self) -> bool:
        """Check if the test server is reachable.

        Attempts a GET /api/state request. Returns True if the server
        responds successfully, False otherwise.
        """
        result = self._request("GET", "/api/state", timeout=5.0)
        return result is not None
