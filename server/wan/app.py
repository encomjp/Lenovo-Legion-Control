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
import threading
import time
from datetime import datetime, timezone

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
        _rate_seen[ip] = window
    return limited


def _client_ip(request: Request) -> str:
    # Used ONLY for in-memory rate limiting — never persisted anywhere.
    return request.client.host if request.client else "unknown"


@app.post("/v1/diagnostics")
async def ingest(request: Request) -> dict:
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

    chunks: list[bytes] = []
    total = 0
    async for chunk in request.stream():
        total += len(chunk)
        if total > MAX_BODY:
            raise HTTPException(status_code=413, detail="payload too large")
        chunks.append(chunk)
    raw = b"".join(chunks)

    try:
        doc = json.loads(raw)
    except (ValueError, RecursionError):
        raise HTTPException(status_code=400, detail="invalid JSON") from None
    if not isinstance(doc, dict) or doc.get("schema_version") != 1:
        raise HTTPException(status_code=400, detail="unsupported report")

    ts = datetime.now(timezone.utc).isoformat()
    distro, model, app_version, schema_version = db._extract_meta(doc)  # noqa: SLF001
    report_id = db.insert(ts, raw.decode("utf-8"), distro, model, app_version, schema_version)
    return {"ok": True, "id": report_id}


@app.get("/health")
def health() -> dict:
    return {"ok": True}
