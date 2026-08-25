#!/usr/bin/env python3
"""Legion Control — anonymous diagnostics collector (alpha).

Hardened version. Contract with clients (lenovo-legion-tool/src/diagnostics.rs):
POST one JSON report per explicit user action; the client whitelists every
field, so payloads are anonymous by construction (never hostname / username /
serials / MACs / IPs / per-key maps).

Server-side hardening:
- shared-secret header  X-Legion-Telemetry-Key  (env LEGION_TELEMETRY_KEY;
  constant-time compare; requests without the key are rejected 401)
- body cap 256 KiB via Content-Length AND a streaming counter
- RecursionError -> 400 (CPython's JSON scanner raises it on deep nesting)
- sqlite in WAL mode + busy_timeout, schema created once at startup
- naive sliding-window rate limit per client IP (default 30/min, 429 beyond)
- retention: reports older than RETENTION_DAYS (default 90) pruned hourly

Run:
    LEGION_TELEMETRY_KEY=<secret> python3 collector.py
Binds to $LEGION_TELEMETRY_HOST (default the Tailscale IP) so alpha stays
private; front with nginx + TLS before going public.
"""

import hmac
import json
import os
import sqlite3
import threading
import time
from datetime import datetime, timedelta, timezone

from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse

DB_PATH = os.environ.get("LEGION_TELEMETRY_DB",
                         os.path.join(os.path.dirname(__file__), "diagnostics.db"))
TELEMETRY_KEY = os.environ.get("LEGION_TELEMETRY_KEY", "")
RETENTION_DAYS = int(os.environ.get("LEGION_TELEMETRY_RETENTION_DAYS", "90"))
MAX_BODY = 256 * 1024
RATE_LIMIT_PER_MIN = int(os.environ.get("LEGION_TELEMETRY_RATE_PER_MIN", "30"))

app = FastAPI(docs_url=None, redoc_url=None)
_db_lock = threading.Lock()
_write_conn: sqlite3.Connection | None = None
_seen: dict[str, list[float]] = {}
_seen_lock = threading.Lock()


def _init_db() -> None:
    global _write_conn
    conn = sqlite3.connect(DB_PATH)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA busy_timeout=5000")
    conn.execute(
        "CREATE TABLE IF NOT EXISTS reports ("
        " id INTEGER PRIMARY KEY AUTOINCREMENT,"
        " ts TEXT NOT NULL, payload TEXT NOT NULL)"
    )
    conn.commit()
    _write_conn = conn


_init_db()


def _check_key(request: Request) -> None:
    if not TELEMETRY_KEY:
        return  # no key configured — Tailscale-only bind is the gate
    got = request.headers.get("x-legion-telemetry-key", "")
    if not hmac.compare_digest(got, TELEMETRY_KEY):
        raise HTTPException(status_code=401, detail="unauthorized")


def _rate_limited(client_ip: str) -> bool:
    now = time.monotonic()
    with _seen_lock:
        window = [t for t in _seen.get(client_ip, []) if now - t < 60.0]
        if len(window) >= RATE_LIMIT_PER_MIN:
            _seen[client_ip] = window
            return True
        window.append(now)
        _seen[client_ip] = window
        return False


def _prune_old() -> None:
    cutoff = (datetime.now(timezone.utc) - timedelta(days=RETENTION_DAYS)).isoformat()
    with _db_lock:
        _write_conn.execute("DELETE FROM reports WHERE ts < ?", (cutoff,))
        _write_conn.commit()


@app.get("/health")
def health() -> dict:
    with _db_lock:
        row = _write_conn.execute("SELECT COALESCE(MAX(id), 0) FROM reports").fetchone()
    return {"ok": True, "last_id": row[0]}


@app.post("/v1/diagnostics")
async def submit_report(request: Request) -> dict:
    _check_key(request)
    ip = request.client.host if request.client else "unknown"
    if _rate_limited(ip):
        raise HTTPException(status_code=429, detail="slow down")

    cl = request.headers.get("content-length", "")
    try:
        if cl and int(cl) > MAX_BODY:
            raise HTTPException(status_code=413, detail="payload too large")
    except ValueError as exc:
        raise HTTPException(status_code=400, detail="bad content-length") from exc

    chunks, total = [], 0
    async for chunk in request.stream():
        total += len(chunk)
        if total > MAX_BODY:
            raise HTTPException(status_code=413, detail="payload too large")
        chunks.append(chunk)
    raw = b"".join(chunks)

    try:
        doc = json.loads(raw)
    except (ValueError, RecursionError):
        # RecursionError is not a ValueError — deep-nested junk must be a 400.
        raise HTTPException(status_code=400, detail="invalid JSON") from None
    if not isinstance(doc, dict) or doc.get("schema_version") != 1:
        raise HTTPException(status_code=400, detail="unsupported report")

    ts = datetime.now(timezone.utc).isoformat()
    with _db_lock:
        _write_conn.execute(
            "INSERT INTO reports (ts, payload) VALUES (?, ?)", (ts, raw.decode("utf-8"))
        )
        _write_conn.commit()
    return {"ok": True}


@app.on_event("startup")
def _startup_prune() -> None:
    _prune_old()
    t = threading.Thread(target=_retention_loop, daemon=True)
    t.start()


def _retention_loop() -> None:
    while True:
        time.sleep(3600)
        try:
            _prune_old()
        except sqlite3.Error:
            pass


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(
        app,
        host=os.environ.get("LEGION_TELEMETRY_HOST", "127.0.0.1"),
        port=int(os.environ.get("LEGION_TELEMETRY_PORT", "8787")),
    )
