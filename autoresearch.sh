#!/usr/bin/env bash
#
# Autoresearch benchmark: telemetry collector pipeline performance.
#
# Ingests 500 deterministic gzipped Schema v4 telemetry reports (fixed seed,
# realistic payload matching diagnostics.rs Schema v4 output, cf. the
# `test_schema_v4_full_telemetry_payload_accepted` shape in
# server/test_collector.py) through server/collector.py's FastAPI app via the
# ASGI TestClient into an isolated temporary SQLite database.
#
# Metrics (stdout):
#   METRIC latency_us=<per-report mean latency in microseconds>   (primary)
#   METRIC throughput_rps=<reports per second>
#   METRIC duration_ms=<total wall time in milliseconds>
#
# Exit 0 on success (all 500 reports accepted and stored), non-zero otherwise.
# Temporary files/databases are removed on exit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/legion-autoresearch-XXXXXX")"
cleanup() {
    rm -rf "$TMPDIR"
}
trap cleanup EXIT INT TERM

export LEGION_TELEMETRY_DB="$TMPDIR/bench.db"
export LEGION_TELEMETRY_RATE_PER_MIN=0
export LEGION_TELEMETRY_KEY=""
export LEGION_TELEMETRY_RETENTION_DAYS=90

N_REPORTS=500
FIXED_SEED=20260903

python3 - "$ROOT" "$TMPDIR" "$N_REPORTS" "$FIXED_SEED" <<'PYEOF'
"""Autoresearch benchmark driver: 500 gzipped Schema v4 reports via TestClient."""
import gzip
import json
import os
import random
import sqlite3
import sys
import time

ROOT, TMPDIR = sys.argv[1], sys.argv[2]
N_REPORTS = int(sys.argv[3])
FIXED_SEED = int(sys.argv[4])

SERVER_DIR = os.path.join(ROOT, "server")
sys.path.insert(0, SERVER_DIR)

# Env must be set BEFORE importing collector (it reads them at import time).
os.environ["LEGION_TELEMETRY_DB"] = os.path.join(TMPDIR, "bench.db")
os.environ["LEGION_TELEMETRY_RATE_PER_MIN"] = "0"
os.environ.pop("LEGION_TELEMETRY_KEY", None)

import collector  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

assert collector.RATE_LIMIT_PER_MIN <= 0, (
    f"rate limiter must be disabled for the benchmark, got {collector.RATE_LIMIT_PER_MIN}"
)

rng = random.Random(FIXED_SEED)


def make_report(i: int) -> dict:
    """Deterministic Schema v4 report mirroring diagnostics.rs emission shape.

    Closed-vocabulary tokens match the collector's v4 test fixture
    (`_report_v4`); only numbers/ids/sensor readings vary via the fixed seed.
    """
    return {
        "schema_version": 4,
        "generated_at": f"2026-09-03T00:{(i // 60) % 60:02d}:{i % 60:02d}+00:00",
        "app_version": "0.2.5",
        "machine_id": f"bench-{rng.getrandbits(64):016x}-{i:04d}",
        "device": {
            "model": "Legion Pro 7 16ARX8H",
            "model_type": "laptop",
            "bios_version": "LPCN31WW",
        },
        "os": {
            "distro": "CachyOS",
            "kernel": "7.2.2-1-cachyos-bore",
        },
        "sensors": {
            "cpu_temp": round(45.0 + rng.random() * 30.0, 1),
            "gpu_temp": round(40.0 + rng.random() * 35.0, 1),
            "fan0_rpm": int(1800 + rng.random() * 2400),
            "fan1_rpm": int(1800 + rng.random() * 2400),
        },
        "battery": {
            "capacity_pct": int(60 + rng.random() * 40),
            "cycle_count": int(rng.random() * 300),
            "health_pct": round(90.0 + rng.random() * 10.0, 1),
            "charge_limit_pct": 80,
        },
        "fan_backend": "legion_ec",
        "fan_control_backend": "legion_ec",
        "fans": [
            {"id": 0, "rpm": int(1800 + rng.random() * 2400)},
            {"id": 1, "rpm": int(1800 + rng.random() * 2400)},
        ],
        "thermal": {"mode": "performance"},
        "profiles": {
            "current": ("low-power", "balanced", "performance")[i % 3],
            "choices": ["low-power", "balanced", "performance"],
            "acpi_choices": ["low-power", "balanced", "performance"],
        },
        "curve_optimizer": {"status": "ok"},
        "settings": {"digest": f"{rng.getrandbits(64):016x}"},
        "faults": [],
        "log_digest": {"error_count": 0, "warn_count": int(rng.random() * 3)},
        "self_checks": [],
        "system_info": {"uptime_secs": 3600 + i},
        "hardware": {
            "cpu": {
                "governor": "performance",
                "energy_performance_preference": "performance",
                "scaling_driver": "amd-pstate-epp",
                "pstate_mode": "amd-pstate:active",
                "boost_enabled": True,
            },
            "gpu": {
                "power_limit_w": 175.0,
                "power_max_w": 175.0,
                "power_default_w": 80.0,
                "dynamic_boost_headroom_w": 95.0,
                "pstate": "P0",
            },
            "display": {"connector": "eDP-1", "vrr_capable": None, "refresh_hz": None},
        },
        "power": {
            "ac_online": True,
            "ac_type": "Mains",
            "charge_state": "Full",
            "charge_rate_w": round(rng.random() * 5.0, 2),
            "voltage_v": round(16.5 + rng.random() * 1.2, 3),
        },
        "audio": {
            "health": "ok",
            "amp_present": True,
            "amp_bound": True,
            "modules_loaded": True,
            "firmware_ok": True,
            "fixable": True,
            "speakers_muted": False,
            "bass_off": False,
            "wrong_default_sink": False,
        },
        "deep": None,
    }


# Pre-build all gzipped bodies OUTSIDE the timed section so the measured
# window covers only the collector ingest path (ASGI -> gunzip -> SQLite).
bodies = []
for i in range(N_REPORTS):
    plain = json.dumps(make_report(i)).encode("utf-8")
    bodies.append(gzip.compress(plain, compresslevel=6))

db_path = os.path.join(TMPDIR, "bench.db")

with TestClient(collector.app) as client:
    start = time.perf_counter()
    for body in bodies:
        resp = client.post(
            "/v1/diagnostics",
            content=body,
            headers={"Content-Encoding": "gzip", "Content-Type": "application/json"},
        )
        if resp.status_code != 200:
            print(f"ingest failed: {resp.status_code} {resp.text}", file=sys.stderr)
            sys.exit(1)
    end = time.perf_counter()

duration_s = end - start
if duration_s <= 0:
    print("non-positive benchmark duration", file=sys.stderr)
    sys.exit(1)

conn = sqlite3.connect(db_path)
try:
    (count,) = conn.execute("SELECT COUNT(*) FROM reports").fetchone()
finally:
    conn.close()

if count != N_REPORTS:
    print(f"row count mismatch: expected {N_REPORTS}, got {count}", file=sys.stderr)
    sys.exit(1)

duration_ms = duration_s * 1000.0
latency_us = duration_s * 1_000_000.0 / N_REPORTS
throughput_rps = N_REPORTS / duration_s

# Primary metric first, then secondaries — exact `METRIC <k>=<v>` format.
print(f"METRIC latency_us={latency_us:.2f}")
print(f"METRIC throughput_rps={throughput_rps:.2f}")
print(f"METRIC duration_ms={duration_ms:.2f}")
PYEOF
