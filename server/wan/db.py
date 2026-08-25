"""SQLite storage for the WAN telemetry collector.

Single shared WAL connection guarded by a module lock; schema created once
at init(). Never stores client IPs or user agents — callers pass only the
whitelisted envelope fields extracted from the report.
"""

from __future__ import annotations

import os
import sqlite3
import threading
from typing import Any

DB_PATH = os.environ.get(
    "LEGION_TELEMETRY_DB",
    os.path.join(os.path.dirname(__file__), "diagnostics.db"),
)

_lock = threading.Lock()
_conn: sqlite3.Connection | None = None


def _connect() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH, check_same_thread=False)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA busy_timeout=5000")
    return conn


def init() -> None:
    """Create schema once. Safe to call repeatedly (idempotent DDL)."""
    global _conn
    with _lock:
        if _conn is None:
            _conn = _connect()
        _conn.execute(
            "CREATE TABLE IF NOT EXISTS reports ("
            " id INTEGER PRIMARY KEY AUTOINCREMENT,"
            " ts TEXT NOT NULL,"
            " received_at TEXT NOT NULL,"
            " payload TEXT NOT NULL,"
            " distro TEXT,"
            " model TEXT,"
            " app_version TEXT,"
            " schema_version INTEGER)"
        )
        _conn.execute("CREATE INDEX IF NOT EXISTS idx_reports_ts ON reports(ts)")
        _conn.commit()


def _extract_meta(doc: dict[str, Any]) -> tuple[str | None, str | None, str | None, int | None]:
    os_info = doc.get("os") if isinstance(doc.get("os"), dict) else {}
    dev = doc.get("device") if isinstance(doc.get("device"), dict) else {}
    distro = os_info.get("distro") if isinstance(os_info.get("distro"), str) else None
    model = dev.get("model") if isinstance(dev.get("model"), str) else None
    app_version = doc.get("app_version") if isinstance(doc.get("app_version"), str) else None
    sv = doc.get("schema_version")
    schema_version = sv if isinstance(sv, int) else None
    return distro, model, app_version, schema_version


def insert(
    ts: str,
    payload_json: str,
    distro: str | None,
    model: str | None,
    app_version: str | None,
    schema_version: int | None,
) -> int:
    """Store one report; returns the new row id."""
    init()
    with _lock:
        assert _conn is not None
        cur = _conn.execute(
            "INSERT INTO reports (ts, received_at, payload, distro, model, app_version,"
            " schema_version) VALUES (?, datetime('now'), ?, ?, ?, ?, ?)",
            (ts, payload_json, distro, model, app_version, schema_version),
        )
        _conn.commit()
        return int(cur.lastrowid)


def recent(limit: int = 50) -> list[dict[str, Any]]:
    """Metadata rows (never payloads), newest first."""
    init()
    with _lock:
        assert _conn is not None
        rows = _conn.execute(
            "SELECT id, ts, COALESCE(distro,''), COALESCE(model,''),"
            " COALESCE(app_version,'') FROM reports ORDER BY id DESC LIMIT ?",
            (limit,),
        ).fetchall()
    return [
        {"id": r[0], "ts": r[1], "distro": r[2], "model": r[3], "app_version": r[4]}
        for r in rows
    ]


def get_payload(report_id: int) -> str | None:
    init()
    with _lock:
        assert _conn is not None
        row = _conn.execute(
            "SELECT payload FROM reports WHERE id = ?", (report_id,)
        ).fetchone()
    return row[0] if row else None


def count() -> int:
    init()
    with _lock:
        assert _conn is not None
        row = _conn.execute("SELECT COUNT(*) FROM reports").fetchone()
    return int(row[0])


def prune_older_than(days: int) -> int:
    """Delete reports older than `days`; returns deleted rowcount."""
    init()
    cutoff_expr = f"datetime('now', '-{int(days)} days')"
    with _lock:
        assert _conn is not None
        cur = _conn.execute(f"DELETE FROM reports WHERE ts < {cutoff_expr}")
        _conn.commit()
    return cur.rowcount
