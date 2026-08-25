"""Pytest bootstrap for the WAN suite: env must exist BEFORE app import."""

import os
import sys

os.environ.setdefault("LEGION_TELEMETRY_KEY", "test-key-123")
os.environ.setdefault("LEGION_TELEMETRY_RATE_PER_MIN", "30")

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
