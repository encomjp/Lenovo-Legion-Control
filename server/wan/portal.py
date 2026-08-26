#!/usr/bin/env python3
"""Legion Telemetry Operations Portal.

Multi-page server-rendered dashboard with SVG charts, fault tracking,
per-machine health grid, sensor bars, and error attribution.
No external JS/CSS/CDN. Tailscale-only access.
"""
from __future__ import annotations

import html
import json
import os
import sys
from datetime import datetime, timedelta, timezone
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse, JSONResponse

if __package__:
    from . import db
else:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import db

log = logging.getLogger("portal")
SCAN_LIMIT = 5000


# ─── helpers ────────────────────────────────────────────────────────────────

def esc(v: Any) -> str:
    return html.escape(str(v if v is not None else ""), quote=True)


def parse_ts(v: Any) -> datetime | None:
    if v is None or isinstance(v, bool): return None
    if isinstance(v, (int, float)):
        try: return datetime.fromtimestamp(float(v), tz=timezone.utc)
        except Exception: return None
    t = str(v).strip()
    if not t: return None
    try: return datetime.fromtimestamp(float(t), tz=timezone.utc)
    except Exception: pass
    iso = t[:-1] + "+00:00" if t[-1] in "Zz" else t
    try:
        d = datetime.fromisoformat(iso)
        return d.replace(tzinfo=timezone.utc) if d.tzinfo is None else d.astimezone(timezone.utc)
    except Exception: return None


def fmt_ts(v: Any) -> str:
    d = parse_ts(v)
    return d.strftime("%Y-%m-%d %H:%M UTC") if d else "—"


def rel_time(v: Any) -> str:
    d = parse_ts(v)
    if not d: return "—"
    s = int((datetime.now(timezone.utc) - d).total_seconds())
    if s < 60: return f"{s}s ago"
    if s < 3600: return f"{s//60}m ago"
    if s < 86400: return f"{s//3600}h ago"
    return f"{s//86400}d ago"


def load_reports() -> list[dict]:
    rows = db.recent(SCAN_LIMIT)
    out = []
    for row in rows:
        e = dict(row)
        raw = db.get_payload(e["id"])
        try:
            p = json.loads(raw or "{}")
            e["_p"] = p
            e["_sensors"] = p.get("sensors", {})
            e["_battery"] = p.get("battery", {})
            e["_fans"] = p.get("fans", [])
            e["_faults"] = p.get("faults", [])
            e["_system"] = p.get("system_info", {})
        except Exception:
            for k in ("_p","_sensors","_battery","_fans","_faults","_system"): e[k] = {} if k != "_fans" and k != "_faults" else []
        out.append(e)
    return out


# ─── CSS ────────────────────────────────────────────────────────────────────

_CSS = """\
:root{--bg:#080c0a;--panel:#0e1412;--line:#1a2820;--fg:#b5ccb9;--dim:#6b8a73;
--accent:#46f08a;--warn:#ffb454;--crit:#e85c5c;--info:#7ab8d4;--ok:#46f08a;
--blue:#5cb3e8;--purple:#b48ce8}
*{box-sizing:border-box;margin:0;padding:0}
html{background:var(--bg);color-scheme:dark;scroll-behavior:smooth}
body{margin:0 auto;max-width:82rem;padding:1.25rem 1rem 4rem;color:var(--fg);
font-family:'JetBrains Mono','Fira Code',ui-monospace,Consolas,monospace;font-size:.85rem;line-height:1.55;
background:radial-gradient(ellipse at top,rgba(70,240,138,.04),transparent 70%),var(--bg)}
nav{display:flex;gap:.1rem;margin-bottom:1.5rem;border-bottom:1px solid var(--line);padding-bottom:.55rem;flex-wrap:wrap;align-items:center}
nav .logo{color:var(--accent);font-weight:700;margin-right:auto;font-size:.9rem}
nav a{color:var(--dim);text-decoration:none;padding:.3rem .7rem;border-radius:4px;font-size:.78rem;text-transform:uppercase;letter-spacing:.08em}
nav a:hover{color:var(--fg);background:rgba(255,255,255,.04)}
nav a.on{color:var(--accent);background:rgba(70,240,138,.08)}
h1{font-size:1.25rem;margin:0 0 .15rem}.prompt{color:var(--accent);margin-right:.35rem}
.tagline{color:var(--dim);margin:0 0 1.5rem;font-size:.8rem}
h2{color:var(--dim);font-size:.75rem;text-transform:uppercase;letter-spacing:.14em;margin:1.6rem 0 .55rem}
.cards{display:flex;gap:.65rem;flex-wrap:wrap;margin-bottom:1.5rem}
.card{border:1px solid var(--line);background:var(--panel);padding:.8rem 1rem;border-radius:5px;flex:1;min-width:9rem;text-align:center}
.card b{display:block;font-size:1.9rem;color:var(--accent)}
.card.warn b{color:var(--warn)}.card.crit b{color:var(--crit)}.card.blue b{color:var(--blue)}
.card span{color:var(--dim);font-size:.68rem;text-transform:uppercase;letter-spacing:.12em}
table{width:100%;border-collapse:collapse}
th{color:var(--dim);font-size:.65rem;text-transform:uppercase;letter-spacing:.12em;border-bottom:1px solid var(--line);padding:.35rem .55rem;text-align:left}
td{padding:.38rem .55rem;border-bottom:1px solid rgba(30,43,35,.4)}
tr:hover td{background:rgba(255,255,255,.02)}
.panel{border:1px solid var(--line);background:var(--panel);border-radius:5px;padding:.85rem 1rem;margin-bottom:1rem}
.grid{display:grid;gap:.7rem}.grid2{grid-template-columns:1fr 1fr}.grid3{grid-template-columns:repeat(auto-fill,minmax(200px,1fr))}.grid4{grid-template-columns:repeat(auto-fill,minmax(160px,1fr))}
.badge{display:inline-block;padding:.08rem .45rem;border-radius:3px;font-size:.72rem;font-weight:600}
.b-ok{background:rgba(70,240,138,.1);color:var(--accent)}.b-warn{background:rgba(255,180,84,.1);color:var(--warn)}.b-crit{background:rgba(232,92,92,.12);color:var(--crit)}.b-info{background:rgba(122,184,212,.1);color:var(--info)}
.dim{color:var(--dim)}.green{color:var(--accent)}.warn{color:var(--warn)}.red{color:var(--crit)}
pre{background:var(--panel);border:1px solid var(--line);padding:.8rem 1rem;border-radius:4px;overflow-x:auto;font-size:.8rem;line-height:1.5;color:var(--fg)}
.hbar{display:flex;align-items:center;gap:.5rem;margin:.18rem 0}
.hbar-label{min-width:110px;text-align:right;color:var(--dim);font-size:.76rem}
.hbar-track{flex:1;height:10px;background:rgba(255,255,255,.05);border-radius:3px;overflow:hidden;min-width:60px}
.hbar-fill{height:100%;border-radius:3px}
.hbar-val{min-width:50px;font-size:.76rem}
.machine-card{border:1px solid var(--line);border-left:3px solid var(--accent);background:var(--panel);border-radius:5px;padding:.75rem .95rem}
.machine-card h3{font-size:.85rem;margin-bottom:.25rem}
.machine-card .meta{color:var(--dim);font-size:.74rem;line-height:1.45}
.fault-entry{padding:.45rem 0;border-bottom:1px solid rgba(30,43,35,.4);font-size:.82rem}
.fault-entry:last-child{border:none}
.spark-poly{fill:none;stroke-width:1.5}
.donut-label{fill:var(--fg);font-size:.7rem;text-anchor:middle;font-family:inherit}
.foot{margin-top:2.5rem;color:var(--dim);font-size:.72rem}
a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
"""


def _page(title: str, body: str, active: str = "") -> HTMLResponse:
    nav_items = [("Dashboard","/"),("Reports","/reports"),("Machines","/machines"),("Faults","/faults"),("Errors","/errors"),("Privacy","/privacy")]
    nav_html = "".join(
        f'<a href="{href}"{" class=on" if href == active else ""}>{label}</a>' for label, href in nav_items
    )
    return HTMLResponse(f"""<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width"><meta name="robots" content="noindex,nofollow">
<title>{title} — Legion Telemetry</title><style>{_CSS}</style></head>
<body><nav><span class="logo">&gt;_ legion</span>{nav_html}</nav>
<h1><span class="prompt">&gt;</span> {title}</h1>{body}
<div class="foot">Legion Control telemetry · alpha · operator access via Tailscale only</div></body></html>""")


def _svg_sparkline(values: list[float], width: int = 200, height: int = 30, color: str = "#46f08a") -> str:
    if len(values) < 2:
        return '<span class="dim">—</span>'
    lo, hi = min(values), max(values)
    rng = (hi - lo) or 1
    pts = ",".join(f"{i*width/(len(values)-1):.1f},{height - 2 - (v-lo)/rng*(height-4):.1f}" for i, v in enumerate(values))
    return f'<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}"><polyline class="spark-poly" stroke="{color}" points="{pts}"/></svg>'


def _svg_donut(segments: list[tuple[str, int, str]], size: int = 120) -> str:
    total = sum(c for _, c, _ in segments) or 1
    cx, cy, r, sw = size//2, size//2, size//2 - 10, 16
    arcs = []
    angle = -90.0
    for label, count, color in segments:
        frac = count / total
        sweep = frac * 360
        if sweep < 0.5: continue
        start_rad = angle * 3.14159 / 180
        end_rad = (angle + sweep) * 3.14159 / 180
        x1, y1 = cx + r * __import__("math").cos(start_rad), cy + r * __import__("math").sin(start_rad)
        x2, y2 = cx + r * __import__("math").cos(end_rad), cy + r * __import__("math").sin(end_rad)
        large = 1 if sweep > 180 else 0
        arcs.append(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{color}" '
                     f'stroke-dasharray="{frac*2*3.14159*r:.1f} {(1-frac)*2*3.14159*r:.1f}" '
                     f'stroke-dashoffset="{-angle/360*2*3.14159*r:.1f}" stroke-width="{sw}"/>')
        angle += sweep
    legend = "".join(f'<div style="display:flex;align-items:center;gap:.3rem">'
                     f'<span style="width:10px;height:10px;border-radius:50%;background:{c};display:inline-block"></span>'
                     f'{esc(l)} ({n})</div>' for l, n, c in segments)
    return (f'<div style="display:flex;gap:1rem;align-items:center">'
            f'<svg width="{size}" height="{size}">{"".join(arcs)}</svg><div>{legend}</div></div>')
