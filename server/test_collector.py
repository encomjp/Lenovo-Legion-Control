"""End-to-end regression tests for collector.py request handling.

The client (lenovo-legion-tool/src/diagnostics.rs::send) gzips its payload
when that shrinks it and sets `Content-Encoding: gzip` — uvicorn/Starlette do
NOT decode request bodies, so the collector must decompress them itself.
These tests pin that contract plus the guards around it: corrupt gzip,
unsupported encodings, the decompression-bomb cap, the schema_version gate,
and verbatim JSON storage.

Run (needs fastapi + httpx in addition to pytest):

    python3 -m pip install pytest fastapi uvicorn httpx
    python3 -m pytest server/test_collector.py -v
"""

import gzip
import json
import os
import sqlite3
import sys
import tempfile
import zlib
from pathlib import Path

import pytest

# The collector needs fastapi; the TestClient needs httpx. Skip cleanly on
# hosts that cannot run the collector at all instead of erroring the suite.
pytest.importorskip("fastapi")
pytest.importorskip("httpx")

SERVER_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SERVER_DIR))

# DB_PATH is read from the environment at collector import time, so the
# isolated test database must be configured BEFORE importing it.
_TEST_DB = Path(tempfile.mkdtemp(prefix="legion-collector-test-")) / "test.db"
os.environ["LEGION_TELEMETRY_DB"] = str(_TEST_DB)

import collector  # noqa: E402  (deliberately after the env setup above)
from fastapi.testclient import TestClient  # noqa: E402


def _report(machine_id: str) -> dict:
    return {
        "schema_version": 3,
        "generated_at": "2026-09-02T12:00:00+00:00",
        "machine_id": machine_id,
        "sensors": {"cpu_temp": 55.0},
    }


def _post(client: TestClient, body: bytes, **headers) -> "object":
    return client.post("/v1/diagnostics", content=body, headers=headers)


def _row_count() -> int:
    conn = sqlite3.connect(_TEST_DB)
    try:
        (n,) = conn.execute("SELECT COUNT(*) FROM reports").fetchone()
    finally:
        conn.close()
    return n


def _payloads_like(fragment: str) -> list[str]:
    conn = sqlite3.connect(_TEST_DB)
    try:
        rows = conn.execute(
            "SELECT payload FROM reports WHERE payload LIKE ? ORDER BY id",
            (f"%{fragment}%",),
        ).fetchall()
    finally:
        conn.close()
    return [r[0] for r in rows]


@pytest.fixture(scope="module")
def client():
    # Context manager runs the FastAPI startup handler (init_db) exactly as
    # uvicorn would under production.
    with TestClient(collector.app) as c:
        yield c


# ── gzip transport contract (the client's actual wire format) ──────────


def test_gzipped_push_is_accepted(client):
    plain = json.dumps(_report("gzip-accepted-machine")).encode()
    before = _row_count()
    r = _post(
        client,
        gzip.compress(plain),
        **{"Content-Encoding": "gzip", "Content-Type": "application/json"},
    )
    assert r.status_code == 200, r.text
    assert r.json() == {"ok": True}
    assert _row_count() == before + 1


def test_x_gzip_alias_is_accepted(client):
    plain = json.dumps(_report("xgzip-alias-machine")).encode()
    before = _row_count()
    r = _post(client, gzip.compress(plain), **{"Content-Encoding": "x-gzip"})
    assert r.status_code == 200, r.text
    assert _row_count() == before + 1


def test_plain_push_still_accepted(client):
    plain = json.dumps(_report("plain-machine")).encode()
    before = _row_count()
    r = _post(client, plain, **{"Content-Type": "application/json"})
    assert r.status_code == 200, r.text
    assert _row_count() == before + 1


def test_stored_payload_is_verbatim_gunzipped_json(client):
    marker = "verbatim-storage-machine"
    plain = json.dumps(_report(marker)).encode()
    r = _post(client, gzip.compress(plain), **{"Content-Encoding": "gzip"})
    assert r.status_code == 200, r.text
    rows = _payloads_like(marker)
    assert len(rows) == 1
    # Contract: the DB stores the raw JSON string, never the gzip bytes.
    assert rows[0] == plain.decode()


# ── guards ──────────────────────────────────────────────────────────────


def test_corrupt_gzip_is_400_not_500(client):
    before = _row_count()
    r = _post(client, b"\x1f\x8b garbage-not-gzip", **{"Content-Encoding": "gzip"})
    assert r.status_code == 400, r.text
    assert _row_count() == before


def test_unsupported_content_encoding_is_415(client):
    r = _post(client, b"x", **{"Content-Encoding": "br"})
    assert r.status_code == 415, r.text


def test_decompression_bomb_is_413(client):
    # 50 MB of zeros compresses to ~50 KB on the wire (passes the
    # content-length/stream cap) but decompresses far past the cap.
    bomb = gzip.compress(b"0" * (50 * 1024 * 1024))
    before = _row_count()
    r = _post(client, bomb, **{"Content-Encoding": "gzip"})
    assert r.status_code == 413, r.text
    assert _row_count() == before


def test_unknown_schema_version_is_400_even_gzipped(client):
    bad = json.dumps({"schema_version": 99}).encode()
    r = _post(client, gzip.compress(bad), **{"Content-Encoding": "gzip"})
    assert r.status_code == 400, r.text


def test_non_dict_payload_is_400(client):
    r = _post(client, gzip.compress(b"[1, 2, 3]"), **{"Content-Encoding": "gzip"})
    assert r.status_code == 400, r.text


def test_health_counts_stored_rows(client):
    r = client.get("/health")
    assert r.status_code == 200
    body = r.json()
    assert body["ok"] is True
    assert body["count"] == _row_count()


# ── gunzip_capped unit behavior ─────────────────────────────────────────


def test_gunzip_capped_roundtrip():
    assert collector.gunzip_capped(gzip.compress(b"abc"), 100) == b"abc"


def test_gunzip_capped_enforces_cap():
    big = gzip.compress(b"0" * 10_000)
    with pytest.raises(ValueError):
        collector.gunzip_capped(big, 100)


def test_gunzip_capped_rejects_corrupt_data():
    with pytest.raises(zlib.error):
        collector.gunzip_capped(b"\x1f\x8b garbage", 100)


def test_gunzip_capped_rejects_truncated_data():
    truncated = gzip.compress(b"abc")[:10]
    with pytest.raises(zlib.error):
        collector.gunzip_capped(truncated, 100)


# ── security, rate limit & retention tests ──────────────────────────────


def test_auth_key_enforcement(client, monkeypatch):
    secret = "test-secret-key-12345"
    monkeypatch.setattr(collector, "TELEMETRY_KEY", secret)

    payload = json.dumps(_report("auth-machine")).encode()

    # Missing key -> 401
    r = _post(client, payload)
    assert r.status_code == 401

    # Wrong key -> 401
    r = _post(client, payload, **{"X-Legion-Telemetry-Key": "wrong-secret"})
    assert r.status_code == 401

    # Correct key -> 200
    r = _post(client, payload, **{"X-Legion-Telemetry-Key": secret})
    assert r.status_code == 200


def test_rate_limiting(client, monkeypatch):
    monkeypatch.setattr(collector, "RATE_LIMIT_PER_MIN", 3)
    # Clear seen tracker
    with collector._seen_lock:
        collector._seen.clear()

    payload = json.dumps(_report("ratelimit-machine")).encode()
    for _ in range(3):
        r = _post(client, payload)
        assert r.status_code == 200

    # 4th request exceeds rate limit
    r = _post(client, payload)
    assert r.status_code == 429
    assert "slow down" in r.text


def test_prune_old_reports():
    conn = collector.connect()
    # Insert an old report (100 days ago) and a fresh report
    old_ts = "2020-01-01T00:00:00+00:00"
    fresh_ts = "2099-01-01T00:00:00+00:00"
    conn.execute("INSERT INTO reports (ts, payload) VALUES (?, ?)", (old_ts, "{}"))
    conn.execute("INSERT INTO reports (ts, payload) VALUES (?, ?)", (fresh_ts, "{}"))
    conn.commit()
    conn.close()

    deleted = collector.prune_old_reports()
    assert deleted >= 1

    conn = collector.connect()
    remaining = conn.execute("SELECT ts FROM reports WHERE ts = ?", (old_ts,)).fetchall()
    conn.close()
    assert len(remaining) == 0


def test_schema_versions_1_2_3_4_all_accepted(client):
    for v in (1, 2, 3, 4):
        doc = _report(f"schema-v{v}-machine")
        doc["schema_version"] = v
        r = _post(client, json.dumps(doc).encode())
        assert r.status_code == 200


def _report_v4(machine_id: str) -> dict:
    """Schema v4 report mirroring the client's actual emission shape."""
    doc = _report(machine_id)
    doc["schema_version"] = 4
    doc["power"] = {
        "ac_online": True,
        "ac_type": "Mains",
        "charge_state": "Full",
        "charge_rate_w": 0.0,
        "voltage_v": 17.142,
    }
    doc["audio"] = {
        "health": "ok",
        "amp_present": True,
        "amp_bound": True,
        "modules_loaded": True,
        "firmware_ok": True,
        "fixable": True,
        "speakers_muted": False,
        "bass_off": False,
        "wrong_default_sink": False,
    }
    doc["hardware"] = {
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
    }
    doc["profiles"] = {
        "current": "performance",
        "choices": ["low-power", "balanced", "performance"],
        "acpi_choices": ["low-power", "balanced", "performance"],
    }
    return doc


def test_schema_v4_full_telemetry_payload_accepted(client):
    """A v4 report carrying every new block stores verbatim."""
    marker = "v4-full-telemetry-machine"
    plain = json.dumps(_report_v4(marker)).encode()
    before = _row_count()
    r = _post(client, plain, **{"Content-Type": "application/json"})
    assert r.status_code == 200, r.text
    assert _row_count() == before + 1
    rows = _payloads_like(marker)
    assert rows, "v4 report was not stored"
    stored = json.loads(rows[0])
    assert stored["schema_version"] == 4
    # New blocks survive the round trip intact.
    assert stored["power"]["ac_type"] == "Mains"
    assert stored["audio"]["health"] == "ok"
    assert stored["hardware"]["gpu"]["dynamic_boost_headroom_w"] == 95.0
    assert stored["hardware"]["display"]["connector"] == "eDP-1"
    assert stored["profiles"]["acpi_choices"][0] == "low-power"


def test_schema_v4_closed_vocabularies(client):
    """Whitelisted v4 tokens hold only closed values — no identifier-shaped
    strings ride along in the new blocks."""
    rows = _payloads_like("v4-full-telemetry-machine")
    assert rows, "run test_schema_v4_full_telemetry_payload_accepted first"
    stored = json.loads(rows[0])
    assert stored["power"]["charge_state"] in (
        "Charging",
        "Discharging",
        "Full",
        "Not charging",
        "Unknown",
    )
    assert stored["power"]["ac_type"] in ("Mains", "USB", "Other")
    assert stored["audio"]["health"] in (
        "ok",
        "soft-issue",
        "hardware-broken",
        "not-applicable",
    )
    pstate = stored["hardware"]["gpu"]["pstate"]
    assert pstate is None or (
        pstate.startswith("P") and pstate[1:].isdigit() and int(pstate[1:]) <= 15
    )
    connector = stored["hardware"]["display"]["connector"]
    assert connector is None or all(
        ch.isalnum() or ch == "-" for ch in connector
    )


def test_schema_v4_gzipped_push_is_accepted(client):
    plain = json.dumps(_report_v4("v4-gzipped-machine")).encode()
    before = _row_count()
    r = _post(
        client,
        gzip.compress(plain),
        **{"Content-Encoding": "gzip", "Content-Type": "application/json"},
    )
    assert r.status_code == 200, r.text
    assert _row_count() == before + 1


def test_schema_v5_is_rejected(client):
    bad = json.dumps({"schema_version": 5}).encode()
    r = _post(client, bad, **{"Content-Type": "application/json"})
    assert r.status_code == 400, r.text
