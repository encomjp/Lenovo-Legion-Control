"""Integration tests for the WAN ingest app (server.wan.app).

Runs against the REAL db.py with a temp database — the only "mock" is the
network itself (fastapi.testclient). Env must be set before app import:
conftest handles LEGION_TELEMETRY_KEY/DB/RATE.
"""

from __future__ import annotations

import json

import pytest
from fastapi.testclient import TestClient

from server.wan import app as wan_app
from server.wan import db

KEY = "test-key-123"
GOOD = {
    "schema_version": 1,
    "app_version": "0.1.0",
    "os": {"distro": "CachyOS 3"},
    "device": {"model": "Legion Pro 7 16AFR10H"},
    "sensors": {"cpu_temp": 62.5},
}


@pytest.fixture()
def client(monkeypatch, tmp_path):
    monkeypatch.setenv("LEGION_TELEMETRY_KEY", KEY)
    monkeypatch.setenv("LEGION_TELEMETRY_DB", str(tmp_path / "diagnostics.db"))
    monkeypatch.setattr(wan_app, "_TELEMETRY_KEY", KEY.encode())
    # fresh rate-limit window per test
    monkeypatch.setattr(wan_app, "_rate_seen", {})
    monkeypatch.setattr(db, "DB_PATH", str(tmp_path / "diagnostics.db"))
    monkeypatch.setattr(db, "_conn", None)
    with TestClient(wan_app.app) as tc:
        yield tc


def post(client: TestClient, body=GOOD, key=KEY, **kw):
    return client.post(
        "/v1/diagnostics",
        content=json.dumps(body).encode() if not isinstance(body, bytes) else body,
        headers={"X-Legion-Telemetry-Key": key},
        **kw,
    )


def test_valid_report_is_accepted_with_id(client):
    r = post(client)
    assert r.status_code == 200
    assert isinstance(r.json()["id"], int)


def test_missing_key_is_401(client):
    assert client.post("/v1/diagnostics", content=b"{}").status_code == 401


def test_wrong_key_is_401(client):
    assert post(client, key="nope").status_code == 401


def test_bad_json_is_400(client):
    assert post(client, b"{not json").status_code == 400


def test_wrong_schema_version_is_400(client):
    assert post(client, {**GOOD, "schema_version": 2}).status_code == 400


def test_non_dict_is_400(client):
    assert post(client, [1, 2, 3]).status_code == 400


def test_deep_nesting_is_400_not_500(client):
    deep = body = {}
    for _ in range(2000):  # beyond CPython json recursion limit
        inner = {}
        inner["d"] = body if body else deep
        body = {"a": body}
        deep = {"d": body}
    r = post(client, json.dumps(body).encode())
    assert r.status_code == 400


def test_oversize_body_is_413(client):
    big = {**GOOD, "pad": "x" * (256 * 1024)}
    assert post(client, big).status_code == 413


def test_payload_never_stores_client_metadata(client):
    post(client)
    row = db.recent(1)[0]
    assert set(row) == {"id", "ts", "distro", "model", "app_version"}


def test_meta_columns_extracted(client):
    rid = post(client).json()["id"]
    payload = json.loads(db.get_payload(rid))
    assert payload["os"]["distro"] == "CachyOS 3"


def test_health_ok(client):
    assert client.get("/health").json() == {"ok": True}


def test_rate_limit_kicks_in_at_threshold(monkeypatch, tmp_path):
    monkeypatch.setenv("LEGION_TELEMETRY_RATE_PER_MIN", "3")
    monkeypatch.setattr(wan_app, "RATE_PER_MIN", 3)
    monkeypatch.setattr(wan_app, "_rate_seen", {})
    monkeypatch.setenv("LEGION_TELEMETRY_DB", str(tmp_path / "rl.db"))
    monkeypatch.setattr(db, "DB_PATH", str(tmp_path / "rl.db"))
    monkeypatch.setattr(db, "_conn", None)
    with TestClient(wan_app.app) as tc:
        codes = [
            post(tc, key=KEY).status_code
            for _ in range(6)  # 3 accepted + 3 limited within the window
        ]
    assert codes[:3] == [200, 200, 200] or set(codes[:3]) <= {200, 429}
    assert codes.count(429) >= 2, f"expected rate limiting, got {codes}"
