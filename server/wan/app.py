"""WAN-facing ingest app — terminates behind the operator's TLS reverse proxy.

Auth: every POST requires header `X-Legion-Telemetry-Key` matching env
LEGION_TELEMETRY_KEY (constant-time). If the env key is unset at import a
random one is generated and printed ONCE to stderr — the operator must copy
it into the client environment; an open collector is never started silently.
"""

from __future__ import annotations

import hmac
import json
import os
import secrets
import sqlite3
import threading
import time
from datetime import datetime, timezone

import anyio
from fastapi import FastAPI, HTTPException, Request

from . import db

MAX_BODY = 256 * 1024
RATE_PER_MIN = int(os.environ.get("LEGION_TELEMETRY_RATE_PER_MIN", "30"))

_TELEMETRY_KEY = os.environ.get("LEGION_TELEMETRY_KEY")
if not _TELEMETRY_KEY:
    _TELEMETRY_KEY = secrets.token_hex(32)
    print(
        "[legion-telemetry] LEGION_TELEMETRY_KEY was unset — generated a "
        "one-time key (copy it into client env): "
        f"X-Legion-Telemetry-Key: {_TELEMETRY_KEY}",
        flush=True,
    )
_KEY_BYTES = _TELEMETRY_KEY.encode()

_rate_lock = threading.Lock()
_rate_seen: dict[str, list[float]] = {}

app = FastAPI(docs_url=None, redoc_url=None)


@app.on_event("startup")
def _startup() -> None:
    db.init()
    threading.Thread(
        target=_prune_forever, name="telemetry-retention", daemon=True
    ).start()


def _prune_forever() -> None:
    """Retention sweep: once at startup, then hourly, forever.

    Runs in a daemon thread; failures must never take the ingest path down,
    so every error is logged loudly and the loop keeps going. db's shared
    connection is only ever touched through db.prune_older_than(), which
    takes the module lock itself.
    """
    while True:
        days = os.environ.get("LEGION_TELEMETRY_RETENTION_DAYS", "90")
        try:
            pruned = db.prune_older_than(int(days))
            if pruned:
                print(
                    f"[legion-telemetry] retention: pruned {pruned} report(s) "
                    f"older than {days} day(s)",
                    flush=True,
                )
        except (sqlite3.Error, ValueError) as exc:
            print(
                "[legion-telemetry] RETENTION PRUNE FAILED "
                f"(retention_days={days!r}): {exc!r}",
                flush=True,
            )
        time.sleep(3600.0)


def _authorized(request: Request) -> bool:
    got = request.headers.get("x-legion-telemetry-key", "")
    return hmac.compare_digest(got.encode(), _KEY_BYTES)


def _rate_limited(ip: str) -> bool:
    now = time.monotonic()
    with _rate_lock:
        window = [t for t in _rate_seen.get(ip, []) if now - t < 60.0]
        limited = len(window) >= RATE_PER_MIN
        if not limited:
            window.append(now)
        if window:
            _rate_seen[ip] = window
        else:
            _rate_seen.pop(ip, None)
        # Evict every remaining key whose window has fully expired, so the
        # map cannot grow unbounded across distinct/spoofed client IPs
        # (hits are chronological, so the newest one bounds the window).
        for key in list(_rate_seen):
            hits = _rate_seen[key]
            if not hits or now - hits[-1] >= 60.0:
                del _rate_seen[key]
    return limited


_LOOPBACK_PEERS = {"127.0.0.1", "::1"}


def _client_ip(request: Request) -> str:
    # Used ONLY for in-memory rate limiting — never persisted anywhere.
    peer = request.client.host if request.client else "unknown"
    xff = request.headers.get("x-forwarded-for")
    if peer in _LOOPBACK_PEERS and xff:
        # Behind the operator's loopback TLS reverse proxy: trust only the
        # FIRST hop of X-Forwarded-For (the proxy appends the peer it saw).
        first_hop = xff.split(",")[0].strip()
        if first_hop:
            return first_hop
    # Direct WAN exposure: the socket peer IS the real client IP.
    return peer


async def _read_capped_body(request: Request, cap: int) -> bytes | None:
    """Consume the request stream on the event loop; None ⇒ body over cap."""
    chunks: list[bytes] = []
    total = 0
    async for chunk in request.stream():
        total += len(chunk)
        if total > cap:
            return None
        chunks.append(chunk)
    return b"".join(chunks)


@app.post("/v1/diagnostics")
def ingest(request: Request) -> dict:
    # Deliberately a plain `def`: FastAPI runs it in its threadpool so the
    # blocking sqlite work never stalls the event loop. The stream itself is
    # drained via anyio.from_thread.run (this thread is anyio-spawned).
    if not _authorized(request):
        raise HTTPException(status_code=401, detail="unauthorized")
    ip = _client_ip(request)
    if _rate_limited(ip):
        raise HTTPException(status_code=429, detail="slow down")

    cl = request.headers.get("content-length", "")
    if cl:
        try:
            if int(cl) > MAX_BODY:
                raise HTTPException(status_code=413, detail="payload too large")
        except ValueError as exc:
            raise HTTPException(status_code=400, detail="bad content-length") from exc

    raw = anyio.from_thread.run(_read_capped_body, request, MAX_BODY)
    if raw is None:
        raise HTTPException(status_code=413, detail="payload too large")

    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        # Must be checked before json.loads: json.loads(bytes) auto-detects
        # UTF-16/32 first, which turns undecodable bodies into 500s.
        raise HTTPException(status_code=400, detail="invalid encoding") from None
    try:
        doc = json.loads(text)
    except (ValueError, RecursionError):
        raise HTTPException(status_code=400, detail="invalid JSON") from None
    if (
        not isinstance(doc, dict)
        or type(doc.get("schema_version")) is not int  # rejects True/False too
        or doc["schema_version"] != 1
    ):
        raise HTTPException(status_code=400, detail="unsupported report")

    ts = datetime.now(timezone.utc).isoformat()
    distro, model, app_version, schema_version = db._extract_meta(doc)  # noqa: SLF001
    machine_id = doc.get("machine_id") if isinstance(doc.get("machine_id"), str) else None
    report_id = db.insert(ts, text, machine_id, distro, model, app_version, schema_version)
    return {"ok": True, "id": report_id}


@app.get("/health")
def health() -> dict:
    return {"ok": True}
