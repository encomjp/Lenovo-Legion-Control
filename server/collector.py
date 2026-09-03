"""Alpha telemetry collector for Legion Control.

Accepts anonymized diagnostics JSON reports from opt-in Legion Control builds
(`POST /v1/diagnostics`) and appends them verbatim to a local SQLite database
(`diagnostics.db`). Each row contains a UTC ISO-8601 timestamp plus the raw
JSON string.

Privacy & Security Hardening:
- Payload contents are anonymous by client contract (see
  lenovo-legion-tool/src/diagnostics.rs): hardware model/type/BIOS/CPU/GPU/EC
  identity, distro/kernel, sensors, fan states, battery health stats (no
  serial), thermal/Curve-Optimizer settings, settings digest, and sanitized
  daemon log tail — never hostname, username, serial numbers, MAC/IP
  addresses or per-key colour maps.
- Shared-secret authentication via header `X-Legion-Telemetry-Key` when
  `LEGION_TELEMETRY_KEY` environment variable is set (constant-time compare).
- Sliding-window rate limiting per client IP (default 30/min, configurable via
  `LEGION_TELEMETRY_RATE_PER_MIN`).
- Gzip stream decompression with bomb guard cap (`MAX_BODY_BYTES = 512 KiB`)
  supporting `Content-Encoding: gzip` and `x-gzip`.
- Supported schema versions: 1, 2, 3, and 4 (v4 adds power, CPU-freq, display, audio-amp, and dGPU-limit blocks).
- SQLite WAL mode + busy_timeout with thread-safe connections.
- Automated hourly retention pruning for reports older than `RETENTION_DAYS` (default 90).
- Modern FastAPI lifespan context manager.
"""

import asyncio
import hmac
import json
import os
import sqlite3
import threading
import time
import zlib
from contextlib import asynccontextmanager
from datetime import datetime, timedelta, timezone

from fastapi import FastAPI, HTTPException, Request

MAX_BODY_BYTES = 512 * 1024
DB_PATH = os.environ.get(
    "LEGION_TELEMETRY_DB",
    os.path.join(os.path.dirname(__file__), "diagnostics.db"),
)
TELEMETRY_KEY = os.environ.get("LEGION_TELEMETRY_KEY", "")
RETENTION_DAYS = int(os.environ.get("LEGION_TELEMETRY_RETENTION_DAYS", "90"))
RATE_LIMIT_PER_MIN = int(os.environ.get("LEGION_TELEMETRY_RATE_PER_MIN", "30"))

SCHEMA = (
    "CREATE TABLE IF NOT EXISTS reports ("
    "id INTEGER PRIMARY KEY AUTOINCREMENT, "
    "ts TEXT NOT NULL, payload TEXT NOT NULL)"
)

_seen: dict[str, list[float]] = {}
_seen_lock = threading.Lock()

_local = threading.local()


def _new_connection() -> sqlite3.Connection:
    """Open a fresh SQLite connection for the calling thread."""
    conn = sqlite3.connect(DB_PATH, timeout=5.0)
    conn.execute("PRAGMA busy_timeout=5000")
    return conn


def connect() -> sqlite3.Connection:
    """Return the calling thread's cached connection, reopening if needed.

    Connections are reused across requests on the same thread instead of
    opening (and re-running PRAGMAs on) a brand-new connection per request.
    Callers MUST NOT close the returned connection; it stays cached in
    thread-local storage. A liveness probe reopens it if it was closed
    externally (e.g. direct ``collector.connect().close()`` in tests).
    """
    conn = getattr(_local, "conn", None)
    if conn is not None:
        try:
            conn.execute("SELECT 1")
            return conn
        except sqlite3.Error:
            pass  # closed or unusable — fall through and reopen
    conn = _new_connection()
    _local.conn = conn
    return conn


def init_db() -> None:
    """Create the table once at startup with WAL mode enabled."""
    conn = connect()
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute("PRAGMA busy_timeout=5000")
    conn.execute(SCHEMA)
    conn.commit()


def prune_old_reports() -> int:
    """Delete reports older than RETENTION_DAYS."""
    cutoff = (datetime.now(timezone.utc) - timedelta(days=RETENTION_DAYS)).isoformat()
    conn = connect()
    cur = conn.execute("DELETE FROM reports WHERE ts < ?", (cutoff,))
    conn.commit()
    return cur.rowcount


def _check_key(request: Request) -> None:
    """Validate shared secret header if LEGION_TELEMETRY_KEY is configured."""
    if not TELEMETRY_KEY:
        return  # no key configured — tailnet or network boundary is the gate
    got = request.headers.get("x-legion-telemetry-key", "")
    if not hmac.compare_digest(got, TELEMETRY_KEY):
        raise HTTPException(status_code=401, detail="unauthorized")


def _rate_limited(client_ip: str) -> bool:
    """Sliding-window rate limiter per client IP."""
    if RATE_LIMIT_PER_MIN <= 0:
        return False
    now = time.monotonic()
    with _seen_lock:
        window = [t for t in _seen.get(client_ip, []) if now - t < 60.0]
        if len(window) >= RATE_LIMIT_PER_MIN:
            _seen[client_ip] = window
            return True
        window.append(now)
        _seen[client_ip] = window
        return False


def gunzip_capped(data: bytes, cap: int) -> bytes:
    """Decompress a gzip body, refusing results larger than `cap` bytes.

    Streaming decompression with an output cap: gzip can expand ~1000x, so a
    naive `gzip.decompress(body)` on a 512 KiB body could allocate gigabytes.
    Raises ValueError when the cap is exceeded and zlib.error on corrupt data.
    """
    dec = zlib.decompressobj(wbits=31)  # 31 = gzip container
    out = bytearray()
    piece = data
    while piece:
        out += dec.decompress(piece, cap + 1 - len(out))
        if len(out) > cap:
            raise ValueError("decompressed payload too large")
        piece = dec.unconsumed_tail
    out += dec.flush()
    if len(out) > cap:
        raise ValueError("decompressed payload too large")
    if not dec.eof:
        raise zlib.error("incomplete or truncated gzip stream")
    return bytes(out)


async def _retention_worker() -> None:
    """Background task to periodically prune expired reports."""
    while True:
        await asyncio.sleep(3600)
        try:
            prune_old_reports()
        except Exception:
            pass


@asynccontextmanager
async def lifespan(_app: FastAPI):
    """Run init_db at startup and run background retention pruning."""
    init_db()
    prune_old_reports()
    worker_task = asyncio.create_task(_retention_worker())
    try:
        yield
    finally:
        worker_task.cancel()
        try:
            await worker_task
        except asyncio.CancelledError:
            pass


app = FastAPI(docs_url=None, redoc_url=None, lifespan=lifespan)


@app.get("/health")
async def health() -> dict:
    conn = connect()
    row = conn.execute(
        "SELECT COUNT(*), COALESCE(MAX(id), 0) FROM reports"
    ).fetchone()
    count, last_id = (row[0], row[1]) if row else (0, 0)
    return {"ok": True, "count": count, "last_id": last_id}


@app.post("/v1/diagnostics")
async def submit_report(request: Request) -> dict:
    """Validate one diagnostics report and append it to the database."""
    _check_key(request)

    ip = request.client.host if request.client else "unknown"
    if _rate_limited(ip):
        raise HTTPException(status_code=429, detail="slow down")

    declared = request.headers.get("content-length", "")
    if declared:
        try:
            if int(declared) > MAX_BODY_BYTES:
                raise HTTPException(status_code=413, detail="payload too large")
        except ValueError as exc:
            raise HTTPException(status_code=400, detail="bad content-length") from exc

    body = b""
    async for chunk in request.stream():
        body += chunk
        if len(body) > MAX_BODY_BYTES:
            raise HTTPException(status_code=413, detail="payload too large")

    # The client may gzip the payload (Content-Encoding: gzip) — the ASGI
    # stack does not decode request bodies, so do it here.
    encoding = (request.headers.get("content-encoding") or "").strip().lower()
    if encoding in ("", "identity"):
        raw = body
    elif encoding in ("gzip", "x-gzip"):
        try:
            raw = gunzip_capped(body, MAX_BODY_BYTES)
        except zlib.error as exc:
            raise HTTPException(status_code=400, detail=f"corrupt gzip body: {exc}") from exc
        except ValueError:
            raise HTTPException(status_code=413, detail="decompressed payload too large") from None
    else:
        raise HTTPException(
            status_code=415, detail=f"unsupported Content-Encoding: {encoding}"
        )

    try:
        doc = json.loads(raw)
    except (ValueError, RecursionError) as exc:  # JSONDecodeError / deep recursion
        raise HTTPException(status_code=400, detail=f"invalid JSON: {exc}") from exc

    if not isinstance(doc, dict) or doc.get("schema_version") not in (1, 2, 3, 4):
        raise HTTPException(status_code=400, detail="schema_version must be 1, 2, 3, or 4")

    try:
        payload = raw.decode("utf-8")
    except UnicodeDecodeError:
        raise HTTPException(status_code=400, detail="payload must be UTF-8") from None

    ts = datetime.now(timezone.utc).isoformat(timespec="seconds")
    conn = connect()
    conn.execute(
        "INSERT INTO reports (ts, payload) VALUES (?, ?)",
        (ts, payload),
    )
    conn.commit()
    return {"ok": True}


if __name__ == "__main__":
    import uvicorn

    host = os.environ.get("LEGION_TELEMETRY_HOST", "127.0.0.1")
    port = int(os.environ.get("LEGION_TELEMETRY_PORT", "8787"))
    uvicorn.run(app, host=host, port=port)
