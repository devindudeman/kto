#!/usr/bin/env python3
"""
E2E Test Server for kto change detection validation.
Uses Python's built-in http.server - no external dependencies required.

Usage:
    python server.py [--port 8787]

Mutation API:
    POST /api/state with JSON body to change server state
    GET /api/state to view current state
    POST /api/reset to reset to defaults
"""

import argparse
import json
import time
import threading
from dataclasses import dataclass, field, asdict
from datetime import datetime
from http.server import HTTPServer, BaseHTTPRequestHandler
from typing import Optional
from urllib.parse import urlparse, parse_qs

# =============================================================================
# Server State
# =============================================================================

@dataclass
class ServerState:
    """Mutable server state for testing."""

    # Product page state
    product_price: str = "$99.99"
    product_stock: str = "SOLD OUT"
    product_name: str = "Test Widget Pro"

    # Release page state
    releases: list = field(default_factory=lambda: ["v1.0.0"])

    # News page state
    articles: list = field(default_factory=lambda: [
        {"title": "First Article", "date": "2026-01-20"},
        {"title": "Second Article", "date": "2026-01-21"},
        {"title": "Third Article", "date": "2026-01-22"},
    ])

    # Status page state
    status: str = "operational"  # operational, degraded, outage
    status_message: str = "All systems operational"

    # Noise simulation
    include_timestamp: bool = True
    include_tracking: bool = True
    include_random_id: bool = True
    ad_variant: str = "A"

    # Error simulation
    error_code: Optional[int] = None  # Set to 403, 500, etc.
    delay_seconds: float = 0.0  # Simulate slow responses
    return_empty: bool = False  # Return empty body
    return_malformed: bool = False  # Return broken HTML


# Global state instance
state = ServerState()
state_lock = threading.Lock()


def get_state():
    with state_lock:
        return asdict(state)


def update_state(**kwargs):
    with state_lock:
        for key, value in kwargs.items():
            if hasattr(state, key):
                setattr(state, key, value)


def reset_state():
    global state
    with state_lock:
        state = ServerState()


# =============================================================================
# Request Handler
# =============================================================================

class TestHandler(BaseHTTPRequestHandler):
    """HTTP request handler for test server."""

    def log_message(self, format, *args):
        """Suppress default logging."""
        pass

    def send_html(self, content: str, status: int = 200):
        """Send HTML response."""
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", len(content.encode()))
        self.end_headers()
        self.wfile.write(content.encode())

    def send_json(self, data: dict, status: int = 200):
        """Send JSON response."""
        content = json.dumps(data)
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", len(content.encode()))
        self.end_headers()
        self.wfile.write(content.encode())

    def send_rss(self, content: str):
        """Send RSS response."""
        self.send_response(200)
        self.send_header("Content-Type", "application/rss+xml")
        self.send_header("Content-Length", len(content.encode()))
        self.end_headers()
        self.wfile.write(content.encode())

    def check_error_simulation(self) -> bool:
        """Check if we should simulate an error. Returns True if error sent."""
        if state.error_code:
            self.send_response(state.error_code)
            self.end_headers()
            return True

        if state.delay_seconds > 0:
            time.sleep(state.delay_seconds)

        if state.return_empty:
            self.send_html("", 200)
            return True

        return False

    def do_GET(self):
        """Handle GET requests."""
        parsed = urlparse(self.path)
        path = parsed.path

        # API endpoints
        if path == "/api/state":
            self.send_json(get_state())
            return

        # Check error simulation for content pages
        if path not in ["/api/state", "/api/reset"]:
            if self.check_error_simulation():
                return

        # Content pages
        if path == "/product":
            self.handle_product()
        elif path == "/product-clean":
            self.handle_product_clean()
        elif path == "/releases":
            self.handle_releases()
        elif path == "/news":
            self.handle_news()
        elif path == "/status":
            self.handle_status()
        elif path == "/static":
            self.handle_static()
        elif path == "/large":
            self.handle_large()
        elif path == "/noise-only":
            self.handle_noise_only()
        elif path == "/rss":
            self.handle_rss()
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        """Handle POST requests."""
        parsed = urlparse(self.path)
        path = parsed.path

        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length).decode() if content_length > 0 else "{}"

        if path == "/api/state":
            try:
                data = json.loads(body)
                update_state(**data)
                self.send_json({"status": "ok", "state": get_state()})
            except json.JSONDecodeError:
                self.send_json({"error": "Invalid JSON"}, 400)
        elif path == "/api/reset":
            reset_state()
            self.send_json({"status": "reset", "state": get_state()})
        else:
            self.send_response(404)
            self.end_headers()

    # -------------------------------------------------------------------------
    # Page Handlers
    # -------------------------------------------------------------------------

    def handle_product(self):
        """Product page with price and stock status."""
        timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S") if state.include_timestamp else ""
        tracking = f'<input type="hidden" name="utm_source" value="test_{int(time.time())}">' if state.include_tracking else ""
        random_id = f'id="product-{hash(time.time()) % 10000}"' if state.include_random_id else 'id="product"'

        if state.product_stock.upper() == "SOLD OUT":
            button = '<button class="stock-btn" disabled>SOLD OUT</button>'
        else:
            button = '<button class="stock-btn add-to-cart">Add to Cart</button>'

        html = f'''<!DOCTYPE html>
<html>
<head>
    <title>{state.product_name} - Test Store</title>
</head>
<body>
    <div {random_id} class="product">
        <h1 class="product-title">{state.product_name}</h1>
        <div class="price-container">
            <span class="price">{state.product_price}</span>
        </div>
        <div class="stock-status">
            {button}
        </div>
        <div class="description">
            <p>This is a test product for E2E testing of kto change detection.</p>
        </div>
        {tracking}
        <div class="metadata">
            <span class="last-updated">Last updated: {timestamp}</span>
        </div>
    </div>
    <div class="ad-container" data-variant="{state.ad_variant}">
        <p>Advertisement variant {state.ad_variant}</p>
    </div>
</body>
</html>'''

        if state.return_malformed:
            html = html.replace('</div>', '').replace('</body>', '').replace('</html>', '')

        self.send_html(html)

    def handle_product_clean(self):
        """Minimal product page without noise."""
        if state.product_stock.upper() == "SOLD OUT":
            button = '<button class="stock-btn" disabled>SOLD OUT</button>'
        else:
            button = '<button class="stock-btn add-to-cart">Add to Cart</button>'

        html = f'''<!DOCTYPE html>
<html>
<head><title>{state.product_name}</title></head>
<body>
    <div class="product">
        <h1>{state.product_name}</h1>
        <span class="price">{state.product_price}</span>
        {button}
    </div>
</body>
</html>'''

        if state.return_malformed:
            html = html.replace('</div>', '').replace('</body>', '').replace('</html>', '')

        self.send_html(html)

    def handle_releases(self):
        """Release/changelog page."""
        releases_html = "\n".join([
            f'<li class="release"><span class="version">{v}</span></li>'
            for v in state.releases
        ])

        html = f'''<!DOCTYPE html>
<html>
<head><title>Releases - Test Project</title></head>
<body>
    <h1>Releases</h1>
    <ul class="release-list">
        {releases_html}
    </ul>
</body>
</html>'''
        self.send_html(html)

    def handle_news(self):
        """News feed page."""
        articles_html = "\n".join([
            f'''<article class="news-item">
            <h2 class="title">{a["title"]}</h2>
            <span class="date">{a["date"]}</span>
        </article>'''
            for a in state.articles
        ])

        html = f'''<!DOCTYPE html>
<html>
<head><title>News Feed</title></head>
<body>
    <h1>Latest News</h1>
    <div class="news-feed">
        {articles_html}
    </div>
</body>
</html>'''
        self.send_html(html)

    def handle_status(self):
        """Service status page."""
        status_class = {
            "operational": "status-ok",
            "degraded": "status-warn",
            "outage": "status-error"
        }.get(state.status, "status-unknown")

        html = f'''<!DOCTYPE html>
<html>
<head><title>System Status</title></head>
<body>
    <h1>System Status</h1>
    <div class="status-indicator {status_class}">
        <span class="status-text">{state.status.upper()}</span>
    </div>
    <p class="status-message">{state.status_message}</p>
</body>
</html>'''
        self.send_html(html)

    def handle_static(self):
        """Static page that never changes."""
        html = '''<!DOCTYPE html>
<html>
<head><title>Static Test Page</title></head>
<body>
    <h1>Static Content</h1>
    <p>This content never changes. It is used to test false positive rates.</p>
    <ul>
        <li>Item 1: Always the same</li>
        <li>Item 2: Never modified</li>
        <li>Item 3: Completely static</li>
    </ul>
</body>
</html>'''
        self.send_html(html)

    def handle_large(self):
        """Large page for stress testing.

        Note: Price is placed INSIDE the content div so that readability-js
        doesn't exclude it when extracting "main content". This mimics how
        real e-commerce sites structure their pages.
        """
        items = [f"<p>Item {i}: Lorem ipsum dolor sit amet, consectetur adipiscing elit. " * 5 + "</p>" for i in range(500)]
        content = "\n".join(items)

        html = f'''<!DOCTYPE html>
<html>
<head><title>Large Content Test</title></head>
<body>
    <h1>Large Content Page</h1>
    <div class="content">
        <div class="product-info">
            <span class="price">{state.product_price}</span>
        </div>
        {content}
    </div>
</body>
</html>'''
        self.send_html(html)

    def handle_noise_only(self):
        """Page where only noise elements change."""
        timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        random_id = hash(time.time()) % 10000

        html = f'''<!DOCTYPE html>
<html>
<head><title>Noise Test Page</title></head>
<body>
    <div id="content-{random_id}">
        <h1>Stable Content</h1>
        <p class="price">$99.99</p>
        <p>This text is always the same.</p>
    </div>
    <div class="metadata">
        <span class="timestamp">Generated: {timestamp}</span>
        <input type="hidden" value="tracking-{random_id}">
    </div>
    <div class="ad">Ad variant: {state.ad_variant}</div>
</body>
</html>'''
        self.send_html(html)

    def handle_rss(self):
        """RSS feed endpoint."""
        items = "\n".join([
            f'''<item>
            <title>{a["title"]}</title>
            <pubDate>{a["date"]}</pubDate>
            <link>http://localhost:8787/article/{i}</link>
        </item>'''
            for i, a in enumerate(state.articles)
        ])

        rss = f'''<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
    <channel>
        <title>Test News Feed</title>
        <link>http://localhost:8787/news</link>
        <description>Test RSS feed for kto E2E testing</description>
        {items}
    </channel>
</rss>'''
        self.send_rss(rss)


# =============================================================================
# Main
# =============================================================================

def run_server(host: str = "127.0.0.1", port: int = 8787):
    """Run the test server."""
    server = HTTPServer((host, port), TestHandler)
    print(f"Starting kto E2E test server on http://{host}:{port}")
    print("Endpoints:")
    print("  /product        - Product page with price/stock")
    print("  /product-clean  - Minimal product page (no noise)")
    print("  /releases       - Release list page")
    print("  /news           - News article list")
    print("  /status         - Service status page")
    print("  /static         - Static page (never changes)")
    print("  /large          - Large content (50KB+)")
    print("  /noise-only     - Only noise elements change")
    print("  /rss            - RSS feed")
    print("API:")
    print("  GET  /api/state - View current state")
    print("  POST /api/state - Update state (JSON body)")
    print("  POST /api/reset - Reset to defaults")
    print("\nPress Ctrl+C to stop.")

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down...")
        server.shutdown()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description='E2E Test Server for kto')
    parser.add_argument('--port', type=int, default=8787, help='Port to listen on')
    parser.add_argument('--host', default='127.0.0.1', help='Host to bind to')
    args = parser.parse_args()

    run_server(args.host, args.port)
