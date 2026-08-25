#!/usr/bin/env python3
"""Legion Control — operator portal for diagnostics reports (alpha).

Read-only dashboard over the same sqlite store that server/wan/db.py owns.
from __future__ import annotations
This is the SECOND service instance, bound by deploy to the Tailscale IP on
:8788 (collector.py runs on :8787). There is deliberately NO authentication —
the gate is network position on the tailnet. Never expose this to the WAN.

Routes:
    GET /               dashboard: headline stats + 50 most recent reports
    GET /reports/{rid}  one raw report payload, pretty-printed JSON in <pre>
    GET /reports.json   machine-readable recent() rows (for scripting)
    GET /healthz        {"ok": true}

Every response carries X-Robots-Tag: noindex and Cache-Control: no-store.

Run standalone (from this directory; falls back to a sys.path tweak):
    python3 portal.py                       # binds 127.0.0.1:8788 (safe default)
    LEGION_PORTAL_HOST=127.0.0.1 python3 portal.py   # tailnet-only bind
Or as part of the package / via uvicorn (deploy path):
    python3 -m server.wan.portal
    uvicorn server.wan.portal:app --host <tailscale-ip> --port 8788

Env overrides for the __main__ launcher: LEGION_PORTAL_HOST, LEGION_PORTAL_PORT.
"""

from __future__ import annotations

import html
import json
import logging
import os
import sys
from contextlib import asynccontextmanager
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse, JSONResponse

if __package__:  # imported as server.wan.portal (python -m … / uvicorn …)
    from . import db
else:  # standalone: python3 portal.py — put this directory on sys.path
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import db  # type: ignore[no-redef]

log = logging.getLogger("portal")

RECENT_LIMIT = 50     # dashboard table size == recent()'s default
SCAN_LIMIT = 5000     # upper bound for stats scans; store is retention-pruned
DEFAULT_PORT = 8788

# --------------------------------------------------------------------------- #
# rendering helpers — EVERY dynamic value goes through esc() before templates
# --------------------------------------------------------------------------- #

_TS_FMT = "%Y-%m-%d %H:%M:%SZ"


def esc(value: Any) -> str:
    """HTML-escape anything destined for markup (attributes included)."""
    return html.escape(str(value if value is not None else ""), quote=True)


def _parse_ts(value: Any) -> datetime | None:
    """Best-effort parse of a report timestamp (epoch or ISO-8601) to UTC."""
    if value is None or isinstance(value, bool):
        return None
    if isinstance(value, (int, float)):
        try:
            return datetime.fromtimestamp(float(value), tz=timezone.utc)
        except (OverflowError, OSError, ValueError):
            return None
    text = str(value).strip()
    if not text:
        return None
    try:  # epoch-as-string
        return datetime.fromtimestamp(float(text), tz=timezone.utc)
    except (OverflowError, OSError, ValueError):
        pass
    iso = text[:-1] + "+00:00" if text[-1] in "Zz" else text
    try:
        parsed = datetime.fromisoformat(iso)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _fmt_ts(value: Any) -> str:
    parsed = _parse_ts(value)
    return parsed.strftime(_TS_FMT) if parsed else str(value if value is not None else "")


def _headline_stats(rows: list[dict[str, Any]]) -> tuple[int, int, int, int]:
    """total reports, reports last 24h, distinct distros, distinct models."""
    cutoff = datetime.now(timezone.utc) - timedelta(hours=24)
    last24 = 0
    distros: set[str] = set()
    models: set[str] = set()
    for row in rows:
        seen_at = _parse_ts(row.get("ts"))
        if seen_at is not None and seen_at >= cutoff:
            last24 += 1
        distro = str(row.get("distro") or "").strip()
        model = str(row.get("model") or "").strip()
        if distro:
            distros.add(distro)
        if model:
            models.add(model)
    return db.count(), last24, len(distros), len(models)


# --------------------------------------------------------------------------- #
# page chrome — single inline-styled template (no external JS/CSS/CDN)
# --------------------------------------------------------------------------- #

_CSS = """
:root {
  --bg: #0a0e0c; --panel: #101613; --line: #1e2b23;
  --fg: #b7ccb9; --dim: #6f8a77; --accent: #46f08a; --warn: #ffb454;
}
* { box-sizing: border-box; }
html { background: var(--bg); }
body {
  margin: 0 auto; max-width: 76rem; padding: 2rem 1.25rem 4rem;
  color: var(--fg);
  font-family: ui-monospace, "SFMono-Regular", "JetBrains Mono", Menlo,
    Consolas, monospace;
  font-size: 14px; line-height: 1.55;
  background:
    radial-gradient(ellipse at top, rgba(70, 240, 138, .06), transparent 60%),
    repeating-linear-gradient(0deg,
      rgba(255, 255, 255, .015) 0 1px, transparent 1px 3px),
    var(--bg);
}
h1 { color: var(--fg); font-size: 1.35rem; margin: 0 0 .25rem; letter-spacing: .02em; }
h1 .prompt { color: var(--accent); margin-right: .4rem; }
h2 { color: var(--dim); font-size: .95rem; text-transform: uppercase;
     letter-spacing: .12em; margin: 2rem 0 .75rem; }
.tagline { color: var(--dim); margin: 0 0 2rem; }
.stats { display: flex; gap: 1rem; flex-wrap: wrap; }
.stat { border: 1px solid var(--line); background: var(--panel);
        padding: .8rem 1.1rem; min-width: 11rem; border-radius: 4px; }
.stat b { display: block; font-size: 1.7rem; color: var(--accent);
          font-weight: 600; }
.stat span { color: var(--dim); font-size: .78rem; text-transform: uppercase;
             letter-spacing: .1em; }
table { width: 100%; border-collapse: collapse; margin-top: .25rem; }
th, td { border: 1px solid var(--line); padding: .45rem .7rem;
         text-align: left; white-space: nowrap; }
th { color: var(--dim); font-size: .74rem; text-transform: uppercase;
     letter-spacing: .12em; background: var(--panel); }
tbody tr:hover { background: #131c16; }
td.wrap { white-space: normal; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
.dim { color: var(--dim); }
pre {
  border: 1px solid var(--line); background: var(--panel); color: var(--fg);
  padding: 1rem 1.25rem; border-radius: 4px; overflow-x: auto;
  font-size: .85rem; line-height: 1.5;
}
.crumb { margin: 1.25rem 0; }
.err h1, h1.err { color: var(--warn); }
.foot { margin-top: 2.5rem; color: var(--dim); font-size: .8rem; }
.empty { color: var(--dim); text-align: center; padding: 1.5rem !important; }
"""


def _page(title: str, body: str, status_code: int = 200) -> HTMLResponse:
    """Wrap pre-rendered sections (all values already escaped upstream)."""
    return HTMLResponse(
        f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Crect width='16' height='16' rx='3' fill='%23101613'/%3E%3Ctext x='2' y='12' font-family='monospace' font-size='10' fill='%2346f08a'%3E%3E_%3C/text%3E%3C/svg%3E">
<title>{title} — legion reports</title>
<style>{_CSS}</style>
</head>
<body>
<main>
{body}
<p class="foot">legion control · operator portal · tailscale-only, unauthenticated
by design — never expose to WAN · feeds: <a href="/reports.json">/reports.json</a>
· <a href="/healthz">/healthz</a></p>
</main>
</body>
</html>""",
        status_code=status_code,
    )


def _row_html(row: dict[str, Any]) -> str:
    rid = esc(row.get("id"))
    mid = esc(str(row.get("machine_id") or "?")[:8])
    cells = (
        f'<td><a href="/reports/{rid}">#{rid}</a></td>',
        f"<td>{esc(_fmt_ts(row.get('ts')))}</td>",
        f"<td><code>{mid}</code></td>",
        f'<td class="wrap">{esc(row.get("distro"))}</td>',
        f'<td class="wrap">{esc(row.get("model"))}</td>',
        f"<td>{esc(row.get('app_version'))}</td>",
    )
    return "<tr>" + "".join(cells) + "</tr>"


# --------------------------------------------------------------------------- #
# app
# --------------------------------------------------------------------------- #


@asynccontextmanager
async def _lifespan(_app: FastAPI):
    db.init()
    log.info("portal ready — report store initialised")
    yield


app = FastAPI(
    title="legion report portal",
    docs_url=None,
    redoc_url=None,
    openapi_url=None,
    lifespan=_lifespan,
)


@app.middleware("http")
async def _noindex_no_store(request: Request, call_next):
    """Every response — success, 404, crash — leaves without caching/indexing."""
    try:
        response = await call_next(request)
    except Exception:
        log.exception("unhandled error serving %s %s", request.method, request.url.path)
        response = JSONResponse({"detail": "internal server error"}, status_code=500)
    response.headers["X-Robots-Tag"] = "noindex"
    response.headers["Cache-Control"] = "no-store"
    return response


@app.api_route("/", methods=["GET", "HEAD"], response_class=HTMLResponse)
def dashboard() -> HTMLResponse:
    rows = db.recent(limit=SCAN_LIMIT)
    total, last24, ndistros, nmodels = _headline_stats(rows)
    table_rows = "\n".join(_row_html(r) for r in rows[:RECENT_LIMIT])
    if not table_rows:
        table_rows = '<tr><td class="empty" colspan="5">no reports yet — waiting for telemetry</td></tr>'
    stats_block = "".join(
        f'<div class="stat"><b>{esc(n)}</b><span>{esc(label)}</span></div>'
        for n, label in (
            (total, "total reports"),
            (last24, "last 24 hours"),
            (ndistros, "distinct distros"),
            (nmodels, "distinct models"),
        )
    )
    body = (
        '<h1><span class="prompt">$</span>legion report portal</h1>\n'
        '<p class="tagline">anonymous diagnostics · alpha · read-only view</p>\n'
        f'<section class="stats">{stats_block}</section>\n'
        f"<h2>recent reports <span class=\"dim\">— latest {RECENT_LIMIT}</span></h2>\n"
        "<table>\n<thead><tr>"
        "<th>id</th><th>received (utc)</th><th>machine</th>"
        "<th>distro</th><th>model</th><th>app version</th>"
        "</tr></thead>\n<tbody>\n" + table_rows + "\n</tbody>\n</table>"
    )
    return _page("dashboard", body)


@app.api_route("/reports.json", methods=["GET", "HEAD"])
def reports_json() -> JSONResponse:
    return JSONResponse(db.recent(limit=RECENT_LIMIT))


@app.api_route("/reports/{rid}", methods=["GET", "HEAD"], response_class=HTMLResponse)
def report_detail(rid: str) -> HTMLResponse:
    raw = db.get_payload(int(rid)) if rid.strip().isdigit() else None
    if raw is None:
        body = (
            '<h1 class="err"><span class="prompt">!</span>404 — report not found</h1>\n'
            f'<p class="dim">no report #{esc(rid)} (or its payload is gone).</p>\n'
            '<p class="crumb"><a href="/">← back to dashboard</a></p>'
        )
        return _page("404", body, status_code=404)
    try:
        pretty = json.dumps(json.loads(raw), indent=2, sort_keys=True)
    except (TypeError, ValueError):
        pretty = raw  # stored payload was not valid JSON — show it verbatim
    body = (
        f'<h1><span class="prompt">$</span>cat report #{esc(rid)}</h1>\n'
        '<p class="crumb"><a href="/">← back to dashboard</a></p>\n'
        f"<pre>{esc(pretty)}</pre>"
    )
    return _page(f"report #{esc(rid)}", body)


@app.api_route("/healthz", methods=["GET", "HEAD"])
def healthz() -> dict[str, bool]:
    return {"ok": True}


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s %(message)s")
    import uvicorn

    uvicorn.run(
        app,
        host=os.environ.get("LEGION_PORTAL_HOST", "127.0.0.1"),
        port=int(os.environ.get("LEGION_PORTAL_PORT", str(DEFAULT_PORT))),
    )
