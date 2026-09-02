"""Alpha telemetry collector for Legion Control.

Accepts one anonymized diagnostics JSON per alpha-client report and appends
it verbatim to a local SQLite database (./diagnostics.db). Each row is a UTC
ISO-8601 timestamp plus the raw JSON string; nothing else is stored or
derived server-side.

The client (lenovo-legion-tool/src/diagnostics.rs::send) gzips its payload
when that shrinks it and sets `Content-Encoding: gzip` — uvicorn/Starlette do
NOT decode request bodies, so this endpoint decompresses gzip bodies itself
(output hard-capped at MAX_BODY_BYTES as a decompression-bomb guard).

Payload contents are anonymous by client contract (see
lenovo-legion-tool/src/diagnostics.rs): hardware model/type/BIOS/CPU/GPU/EC
identity, distro/kernel, sensors, fan states, battery health stats (no
serial), thermal/Curve-Optimizer settings, a settings digest, a sanitized
daemon log tail and self-check results — never hostname, username, serial
numbers, MAC/IP addresses or per-key colour maps.

Retention is the operator's responsibility: delete or regulate old rows in
the database yourself (see README.md).

The default bind address is the developer's Tailscale IP (127.0.0.1),
which keeps this endpoint private to the tailnet during alpha.
"""

import json
import os
import sqlite3
import zlib
from contextlib import asynccontextmanager
from datetime import datetime, timezone

from fastapi import FastAPI, HTTPException, Request

MAX_BODY_BYTES = 512 * 1024
DB_PATH = os.environ.get("LEGION_TELEMETRY_DB", "./diagnostics.db")

SCHEMA = (
    "CREATE TABLE IF NOT EXISTS reports ("
    "id INTEGER PRIMARY KEY AUTOINCREMENT, "
    "ts TEXT NOT NULL, payload TEXT NOT NULL)"
)


def connect() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH)
    return conn


def init_db() -> None:
    """Create the table once at startup instead of on every request."""
    conn = connect()
    try:
        conn.execute(SCHEMA)
        conn.commit()
    finally:
        conn.close()


@asynccontextmanager
async def lifespan(_app: FastAPI):
    """Run init_db at startup (replaces the deprecated on_event hook)."""
    init_db()
    yield
    # No shutdown cleanup: connections are opened per-request and closed in
    # finally blocks.


app = FastAPI(docs_url=None, redoc_url=None, lifespan=lifespan)


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


@app.post("/v1/diagnostics")
async def submit_report(request: Request) -> dict:
    """Validate one diagnostics report and append it to the database."""
    declared = request.headers.get("content-length", "")
    if declared.isdigit() and int(declared) > MAX_BODY_BYTES:
        raise HTTPException(status_code=413, detail="payload too large")
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
    except ValueError as exc:  # JSONDecodeError / undecodable bytes
        raise HTTPException(status_code=400, detail=f"invalid JSON: {exc}") from exc
    if not isinstance(doc, dict) or doc.get("schema_version") not in (1, 2, 3):
        raise HTTPException(status_code=400, detail="schema_version must be 1, 2, or 3")
    try:
        payload = raw.decode("utf-8")
    except UnicodeDecodeError:
        raise HTTPException(status_code=400, detail="payload must be UTF-8") from None
    ts = datetime.now(timezone.utc).isoformat(timespec="seconds")
    conn = connect()
    try:
        conn.execute(
            "INSERT INTO reports (ts, payload) VALUES (?, ?)",
            (ts, payload),
        )
        conn.commit()
    finally:
        conn.close()
    return {"ok": True}


@app.get("/health")
async def health() -> dict:
    conn = connect()
    try:
        (count,) = conn.execute("SELECT COUNT(*) FROM reports").fetchone()
    finally:
        conn.close()
    return {"ok": True, "count": count}


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(
        app,
        host=os.environ.get("LEGION_TELEMETRY_HOST", "127.0.0.1"),
        port=int(os.environ.get("LEGION_TELEMETRY_PORT", "8787")),
    )
