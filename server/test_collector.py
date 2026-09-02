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
