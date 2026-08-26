#!/usr/bin/env python3
"""Legion Telemetry Operations Portal."""

from __future__ import annotations

import html as _h
import json
import math
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse

try:
    from . import db
except ImportError:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import db  # type: ignore[no-redef]

SCAN_LIMIT = 5000

app = FastAPI(docs_url=None, redoc_url=None)


def esc(v: Any) -> str:
    return _h.escape(str(v if v is not None else ""), quote=True)


def parse_ts(v: Any) -> datetime | None:
    if v is None or isinstance(v, bool):
        return None
    try:
        return datetime.fromtimestamp(float(v), tz=timezone.utc)
    except Exception:
        pass
    t = str(v).strip()
    iso = t[:-1] + "+00:00" if t and t[-1] in "Zz" else t
    try:
        d = datetime.fromisoformat(iso)
        return d.replace(tzinfo=timezone.utc) if d.tzinfo is None else d.astimezone(timezone.utc)
    except Exception:
        return None


def fmt_ts(v: Any) -> str:
    d = parse_ts(v)
    return d.strftime("%m-%d %H:%M") if d else "—"


def load_reports() -> list[dict[str, Any]]:
    rows = db.recent(SCAN_LIMIT)
    out: list[dict[str, Any]] = []
    for row in rows:
        e = dict(row)
        raw = db.get_payload(e["id"])
        try:
            p = json.loads(raw or "{}")
            e["_p"] = p
            e["_s"] = p.get("sensors", {})
            e["_b"] = p.get("battery", {})
            e["_f"] = p.get("fans", [])
            e["_fl"] = p.get("faults", [])
            e["_ts_dt"] = parse_ts(p.get("generated_at"))
        except Exception:
            e["_p"] = {}
            e["_s"] = {}
            e["_b"] = {}
            e["_f"] = []
            e["_fl"] = []
            e["_ts_dt"] = None
        out.append(e)
    out.sort(key=lambda r: r.get("_ts_dt") or datetime.min.replace(tzinfo=timezone.utc))
    return out


_CSS = """\
:root{--bg:#0a0e0c;--p:#0e1412;--ln:#1e2b23;--fg:#b7ccb9;--dim:#6f8a77;
--ac:#46f08a;--wrn:#ffb454;--crt:#e85c5c;--inf:#7ab8d4;--blu:#5cb3e8}
*{box-sizing:border-box;margin:0;padding:0}
html{background:var(--bg);color-scheme:dark;scroll-behavior:smooth}
body{margin:0 auto;max-width:82rem;padding:1rem 1rem 3rem;color:var(--fg);
font-family:'JetBrains Mono',Consolas,monospace;font-size:.84rem;line-height:1.55;
background:radial-gradient(ellipse at top,rgba(70,240,138,.04),transparent 70%),var(--bg)}
nav{display:flex;gap:.08rem;margin-bottom:1.1rem;border-bottom:1px solid var(--ln);padding-bottom:.42rem;flex-wrap:wrap;align-items:center}
nav .logo{color:var(--ac);font-weight:700;margin-right:auto;font-size:.86rem}
nav a{color:var(--dim);text-decoration:none;padding:.22rem .52rem;border-radius:3px;font-size:.74rem;text-transform:uppercase;letter-spacing:.08em}
nav a:hover{color:var(--fg);background:rgba(255,255,255,.04)}nav a.on{color:var(--ac);background:rgba(70,240,138,.08)}
h2{color:var(--dim);font-size:.7rem;text-transform:uppercase;letter-spacing:.14em;margin:1.3rem 0 .45rem}
.tagline{color:var(--dim);margin:0 0 1rem;font-size:.76rem}
.cards{display:flex;gap:.55rem;flex-wrap:wrap;margin-bottom:1rem}
.card{border:1px solid var(--ln);background:var(--p);padding:.6rem .8rem;border-radius:4px;flex:1;min-width:8.5rem;text-align:center;transition:border-color .2s}
.card:hover{border-color:rgba(70,240,138,.25)}
.card b{display:block;font-size:1.65rem;color:var(--ac)}
.card.wrnc b{color:var(--wrn)}.card.crtc b{color:var(--crt)}.card.bluc b{color:var(--blu)}
.card span{color:var(--dim);font-size:.62rem;text-transform:uppercase;letter-spacing:.12em}
table{width:100%;border-collapse:collapse}
th{color:var(--dim);font-size:.62rem;text-transform:uppercase;letter-spacing:.12em;border-bottom:1px solid var(--ln);padding:.28rem .48rem;text-align:left}
td{padding:.28rem .48rem;border-bottom:1px solid rgba(30,43,35,.35)}
tr:hover td{background:rgba(255,255,255,.02)}
.panel{border:1px solid var(--ln);background:var(--p);border-radius:4px;padding:.7rem .8rem;margin-bottom:.7rem}
.badge{display:inline-block;padding:.05rem .38rem;border-radius:3px;font-size:.68rem;font-weight:600}
.b-ok{background:rgba(70,240,138,.1);color:var(--ac)}.b-warn{background:rgba(255,180,84,.1);color:var(--wrn)}.b-crit{background:rgba(232,92,92,.12);color:var(--crt)}.b-info{background:rgba(122,184,212,.1);color:var(--inf)}
.dim{color:var(--dim)}
pre{background:var(--p);border:1px solid var(--ln);padding:.65rem;border-radius:3px;overflow-x:auto;font-size:.78rem;color:var(--fg)}
.mc{border:1px solid var(--ln);border-left:3px solid var(--ac);background:var(--p);border-radius:4px;padding:.6rem .8rem}
.mc h3{font-size:.8rem;margin-bottom:.18rem}.mc .meta{color:var(--dim);font-size:.72rem;line-height:1.4}
.fe{padding:.32rem 0;border-bottom:1px solid rgba(30,43,35,.3);font-size:.78rem}.fe:last-child{border:none}
.hbar{display:flex;align-items:center;gap:.35rem;margin:.12rem 0}
.hl{min-width:85px;text-align:right;color:var(--dim);font-size:.72rem}
.ht{flex:1;height:9px;background:rgba(255,255,255,.04);border-radius:2px;overflow:hidden;min-width:40px}
.hf{height:100%;border-radius:2px;transition:width .3s ease}
.hv{min-width:48px;font-size:.72rem}
.grid2{display:grid;grid-template-columns:1fr 1fr;gap:.65rem}.grid3{display:grid;grid-template-columns:repeat(auto-fill,minmax(210px,1fr));gap:.65rem}
.foot{margin-top:2rem;color:var(--dim);font-size:.68rem}
a{color:var(--ac);text-decoration:none}a:hover{text-decoration:underline}
"""


def _page(title: str, body: str, active: str = "") -> HTMLResponse:
    nav_items = [
        ("Dashboard", "/", "dashboard"),
        ("Reports", "/reports", "reports"),
        ("Machines", "/machines", "machines"),
        ("Faults", "/faults", "faults"),
        ("Errors", "/errors", "errors"),
        ("Privacy", "/privacy", "privacy"),
    ]
    nav = "".join(
        f'<a href="{href}"{" class=on" if act == active else ""}>{label}</a>'
        for label, href, act in nav_items
    )
    html_doc = (
        '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width"><meta name="robots" content="noindex,nofollow">'
        f"<title>{esc(title)} — Legion Telemetry</title><style>{_CSS}</style></head>"
        f'<body><nav><span class="logo">&gt;_ legion</span>{nav}</nav>'
        f'<h1><span class="prompt" style="color:var(--ac)">&gt;</span> {esc(title)}</h1>{body}'
        '<div class="foot">Legion Control telemetry · alpha · Tailscale-only operator access</div></body></html>'
    )
    return HTMLResponse(html_doc)


# ─── Dashboard ───────────────────────────────────────────────────────────────


@app.get("/", response_class=HTMLResponse)
async def dashboard_page() -> HTMLResponse:
    reps = load_reports()
    n = len(reps)
    machines = len({r.get("_p", {}).get("machine_id", "") for r in reps if r.get("_p", {}).get("machine_id")})
    # last 24h by generated_at
    now = datetime.now(timezone.utc)
    last24 = sum(
        1
        for r in reps
        if (parse_ts(r.get("_p", {}).get("generated_at")) or datetime.min.replace(tzinfo=timezone.utc))
        >= now - timedelta(hours=24)
    )
    crit_count = sum(1 for r in reps for f in r.get("_fl", []) if f.get("severity") == "Critical")
    warn_count = sum(1 for r in reps for f in r.get("_fl", []) if f.get("severity") == "Warning")

    # Sensor bars from most recent report (chronologically last)
    latest = reps[-1] if reps else {}
    s = latest.get("_s", {}) if isinstance(latest.get("_s"), dict) else {}
    temp_bars = ""
    for label, key in [
        ("CPU", "cpu_temp"),
        ("CCD1", "cpu_temp_1"),
        ("CCD2", "cpu_temp_2"),
        ("dGPU", "dgpu_temp"),
        ("iGPU", "igpu_edge"),
        ("EC CPU", "ec_cpu"),
    ]:
        val = s.get(key)
        if val is None or not isinstance(val, (int, float)) or val < 0:
            continue
        pct = min(100, max(0, (float(val) - 20) / (100 - 20) * 100))
        col = "var(--crt)" if float(val) >= 90 else "var(--wrn)" if float(val) >= 75 else "var(--ac)"
        temp_bars += (
            f'<div class="hbar"><span class="hl">{esc(label)}</span>'
            f'<div class="ht"><div class="hf" style="width:{pct:.0f}%;background:{col}"></div></div>'
            f'<span class="hv">{float(val):.1f}°C</span></div>\n'
        )
    for i, t in enumerate(s.get("ssd_composite", []) if isinstance(s.get("ssd_composite"), list) else []):
        if not isinstance(t, (int, float)):
            continue
        col = "var(--crt)" if float(t) >= 80 else "var(--wrn)" if float(t) >= 60 else "var(--ac)"
        pct = min(100, max(0, float(t)))
        temp_bars += (
            f'<div class="hbar"><span class="hl">NVMe {i}</span>'
            f'<div class="ht"><div class="hf" style="width:{pct:.0f}%;background:{col}"></div></div>'
            f'<span class="hv">{float(t):.1f}°C</span></div>\n'
        )

    fan_bars = ""
    latest_fans = latest.get("_f", []) if isinstance(latest.get("_f"), list) else []
    for f in latest_fans:
        if not isinstance(f, dict):
            continue
        rpm = f.get("rpm", 0)
        mx = f.get("max_rpm", 5000)
        if not isinstance(rpm, (int, float)) or not isinstance(mx, (int, float)) or mx <= 0:
            continue
        pct = min(100, max(0, float(rpm) / float(mx) * 100))
        col = "var(--crt)" if pct > 80 else "var(--wrn)" if pct > 50 else "var(--ac)"
        fid = esc(f.get("id", "?"))
        fan_bars += (
            f'<div class="hbar"><span class="hl">Fan {fid}</span>'
            f'<div class="ht"><div class="hf" style="width:{pct:.0f}%;background:{col}"></div></div>'
            f'<span class="hv">{int(rpm)} RPM</span></div>\n'
        )

    # Fault donut
    sev_counts: dict[str, int] = {"Critical": 0, "Warning": 0, "Info": 0}
    for r in reps:
        for flt in r.get("_fl", []) if isinstance(r.get("_fl"), list) else []:
            sev = flt.get("severity") if isinstance(flt, dict) else None
            if sev in sev_counts:
                sev_counts[sev] += 1
            elif isinstance(sev, str):
                sev_counts[sev] = sev_counts.get(sev, 0) + 1
    total_faults = sum(sev_counts.values())
    colors = {"Critical": "#e85c5c", "Warning": "#ffb454", "Info": "#46f08a"}
    cx = cy = 60
    r_rad = 44
    arcs: list[str] = []
    angle = -90.0
    denom = total_faults or 1
    for sev_name in ["Critical", "Warning", "Info"]:
        cnt = sev_counts.get(sev_name, 0)
        if cnt <= 0:
            continue
        frac = cnt / denom
        sweep = frac * 360
        if sweep < 0.5:
            continue
        col = colors.get(sev_name, "#46f08a")
        start = math.radians(angle)
        end = math.radians(angle + sweep)
        x1, y1 = cx + r_rad * math.cos(start), cy + r_rad * math.sin(start)
        x2, y2 = cx + r_rad * math.cos(end), cy + r_rad * math.sin(end)
        large = 1 if sweep > 180 else 0
        arcs.append(
            f'<path d="M {cx} {cy} L {x1:.1f} {y1:.1f} A {r_rad} {r_rad} 0 {large} 1 {x2:.1f} {y2:.1f} Z" fill="{col}" opacity=".8"/>'
        )
        angle += sweep
    donut_svg = (
        f'<svg width="120" height="120">{"".join(arcs)}'
        f'<circle cx="{cx}" cy="{cy}" r="{r_rad - 14}" fill="var(--bg)"/>'
        f'<text x="{cx}" y="{cy - 3}" text-anchor="middle" fill="var(--fg)" font-size="18" font-weight="700">{total_faults}</text>'
        f'<text x="{cx}" y="{cy + 14}" text-anchor="middle" fill="var(--dim)" font-size="8">FAULTS</text></svg>'
    )
    legend = "".join(
        f'<div style="display:flex;align-items:center;gap:.3rem;margin:.1rem 0">'
        f'<span style="width:9px;height:9px;border-radius:50%;background:{colors[s2]};display:inline-block"></span>'
        f"{esc(s2)}: {sev_counts.get(s2,0)}</div>"
        for s2 in ["Critical", "Warning", "Info"]
    )
    fault_donut = f'<div style="display:flex;gap:.8rem;align-items:center">{donut_svg}<div>{legend}</div></div>'

    # Fault details list (recent faults)
    fault_details_html = ""
    for r in reversed(reps[-5:]):
        for flt in r.get("_fl", []) if isinstance(r.get("_fl"), list) else []:
            if not isinstance(flt, dict):
                continue
            sev = flt.get("severity", "?")
            fid = flt.get("id", "?")
            detail = flt.get("detail", "")
            badge_cls = "b-crit" if sev == "Critical" else "b-warn" if sev == "Warning" else "b-info"
            fault_details_html += (
                f'<div class="fe"><span class="badge {badge_cls}">{esc(sev)}</span> '
                f"<strong>{esc(fid)}</strong> — {esc(detail)}</div>"
            )
    if not fault_details_html:
        fault_details_html = '<span class="dim">Clean — no faults</span>'

    act_rows = ""
    for r in reversed(reps[-10:]):
        ts_d = r.get("_ts_dt")
        ts_str = ts_d.strftime("%m-%d %H:%M") if isinstance(ts_d, datetime) else "?"
        model = r.get("_p", {}).get("device", {}).get("model", "?") if isinstance(r.get("_p"), dict) else "?"
        nf = len(r.get("_fl", []) if isinstance(r.get("_fl"), list) else [])
        badge = "b-crit" if nf else "b-ok"
        act_rows += f"<tr><td>{esc(ts_str)}</td><td>{esc(model)}</td><td><span class=\"badge {badge}\">{nf}</span></td></tr>"
    if not act_rows:
        act_rows = '<tr><td colspan="3" class="dim">none</td></tr>'

    cards = "".join(
        [
            f'<div class="card"><b>{n}</b><span>Total Reports</span></div>',
            f'<div class="card bluc"><b>+{last24}</b><span>Last 24 Hours</span></div>',
            f'<div class="card"><b>{machines}</b><span>Machines</span></div>',
            f'<div class="card wrnc"><b>{warn_count}</b><span>Warnings</span></div>',
            f'<div class="card crtc"><b>{crit_count}</b><span>Critical Faults</span></div>',
        ]
    )

    body = (
        '<p class="tagline">Anonymous diagnostics · alpha · Tailscale-only operator access</p>'
        f'<div class="cards">{cards}</div>'
        '<div class="grid2"><div>'
        "<h2>Sensor Temperatures</h2>"
        f'<div class="panel">{temp_bars or "<span class=\"dim\">No data yet</span>"}</div>'
        "<h2>Fan Status</h2>"
        f'<div class="panel">{fan_bars or "<span class=\"dim\">No fans detected</span>"}</div>'
        "</div><div>"
        "<h2>Fault Distribution</h2>"
        f'<div class="panel" style="display:flex;justify-content:center">{fault_donut}</div>'
        "<h2>Fault Details</h2>"
        f'<div class="panel">{fault_details_html}</div>'
        "</div></div>"
        "<h2>Recent Activity</h2>"
        '<div class="panel"><table><thead><tr><th>Time</th><th>Model</th><th>Faults</th></tr></thead>'
        f"<tbody>{act_rows}</tbody></table></div>"
    )
    return _page("Dashboard", body, active="dashboard")


@app.get("/reports", response_class=HTMLResponse)
def reports_list_page() -> HTMLResponse:
    reports = load_reports()
    rows = ""
    for r in reversed(reports):
        rid = r["id"]
        ts = fmt_ts(r.get("_p", {}).get("generated_at") if isinstance(r.get("_p"), dict) else None)
        mid = str(r.get("machine_id", "—"))[:8]
        distro = esc(r.get("_p", {}).get("os", {}).get("distro", "?") if isinstance(r.get("_p"), dict) else "?")
        model = esc(r.get("_p", {}).get("device", {}).get("model", "?") if isinstance(r.get("_p"), dict) else "?")
        nf = len(r.get("_fl", []) if isinstance(r.get("_fl"), list) else [])
        fc = f'<span class="badge b-crit">{nf}</span>' if nf else '<span class="badge b-ok">0</span>'
        rows += f"<tr><td><a href='/reports/{rid}'>#{rid}</a></td><td>{esc(ts)}</td><td>{esc(mid)}</td><td>{distro}</td><td>{model}</td><td>{fc}</td></tr>"
    empty_row = '<tr><td colspan="6" class="dim">empty</td></tr>'
    body_rows = rows if rows else empty_row
    body = (
        f"<h2>All Reports ({len(reports)})</h2>"
        "<div class='panel'><table style='margin-top:0'><thead><tr>"
        "<th>ID</th><th>Time</th><th>Machine ID</th><th>Distro</th><th>Model</th><th>Faults</th>"
        f"</tr></thead><tbody>{body_rows}</tbody></table></div>"
    )
    return _page("Reports", body, active="reports")


@app.get("/machines", response_class=HTMLResponse)
def machines_view_page() -> HTMLResponse:
    reports = load_reports()
    groups: dict[str, list[dict[str, Any]]] = {}
    for r in reports:
        p = r.get("_p") if isinstance(r.get("_p"), dict) else {}
        mid = p.get("machine_id") or (p.get("device", {}).get("model", "unknown") if isinstance(p.get("device"), dict) else "unknown")
        mid = str(mid)
        groups.setdefault(mid, []).append(r)
    cards = ""
    for mid, grp in sorted(groups.items()):
        last = grp[-1]
        p = last.get("_p") if isinstance(last.get("_p"), dict) else {}
        dev = p.get("device", {}) if isinstance(p.get("device"), dict) else {}
        os_d = p.get("os", {}).get("distro", "?") if isinstance(p.get("os"), dict) else "?"
        nflts = sum(len(g.get("_fl", []) if isinstance(g.get("_fl"), list) else []) for g in grp)
        hc = "#46f08a" if not nflts else ("#ffb454" if nflts < 5 else "#e85c5c")
        model = esc(dev.get("model", "?") if isinstance(dev, dict) else "?")
        cards += (
            f'<div class="mc" style="border-left:3px solid {hc}">'
            f"<h3>{model}</h3>"
            f'<div class="meta">ID: {esc(mid[:12])}…<br>Distro: {esc(os_d)}<br>'
            f"Reports: {len(grp)} | Faults: {nflts}<br>Last: {esc(str(last.get('ts','?')[:16]))}</div></div>"
        )
    empty_cards = '<p class="dim">none</p>'
    body_cards = cards if cards else empty_cards
    body = f"<h2>Machines ({len(groups)})</h2><div class='grid3'>{body_cards}</div>"
    return _page("Machines", body, active="machines")


@app.get("/faults", response_class=HTMLResponse)
def faults_view_page() -> HTMLResponse:
    reports = load_reports()
    sections = ""
    total = 0
    for r in reversed(reports):
        flts = r.get("_fl", []) if isinstance(r.get("_fl"), list) else []
        if not flts:
            continue
        ts = fmt_ts(r.get("_p", {}).get("generated_at") if isinstance(r.get("_p"), dict) else None)
        items = "".join(
            f"<div class='fe'><span class='badge {'b-crit' if f.get('severity') == 'Critical' else 'b-warn' if f.get('severity') == 'Warning' else 'b-info'}'>"
            f"{esc(f.get('severity','?'))}</span> <strong>{esc(f.get('id',''))}</strong> — {esc(f.get('detail',''))}</div>"
            for f in flts
            if isinstance(f, dict)
        )
        sections += f"<h2>{esc(ts)}</h2><div class='panel'>{items}</div>"
        total += len(flts)
    body = f"<h2>Fault History ({total} entries)</h2>" + (sections if sections else "<p class='dim'>Clean — no faults recorded.</p>")
    return _page("Fault Tracker", body, active="faults")


@app.get("/errors", response_class=HTMLResponse)
def errors_view_page() -> HTMLResponse:
    reports = load_reports()
    agg: dict[str, dict[str, int]] = {}
    for r in reports:
        p = r.get("_p") if isinstance(r.get("_p"), dict) else {}
        ld = p.get("log_digest", {}) if isinstance(p.get("log_digest"), dict) else {}
        by_target = ld.get("errors_by_target") if isinstance(ld.get("errors_by_target"), dict) else {}
        for tgt, cnt in by_target.items():
            if not isinstance(tgt, str) or not isinstance(cnt, int):
                continue
            if tgt not in agg:
                agg[tgt] = {"ERROR": 0, "WARN": 0}
            agg[tgt]["ERROR"] = agg[tgt].get("ERROR", 0) + cnt
        # also handle warn counts if present
        warn_by = ld.get("warnings_by_target") if isinstance(ld.get("warnings_by_target"), dict) else {}
        for tgt, cnt in warn_by.items():
            if not isinstance(tgt, str) or not isinstance(cnt, int):
                continue
            if tgt not in agg:
                agg[tgt] = {"ERROR": 0, "WARN": 0}
            agg[tgt]["WARN"] = agg[tgt].get("WARN", 0) + cnt

    max_e = max((v.get("ERROR", 0) for v in agg.values()), default=1) or 1
    rows = ""
    for tgt, c in sorted(agg.items(), key=lambda x: -x[1].get("ERROR", 0)):
        err_c = c.get("ERROR", 0)
        warn_c = c.get("WARN", 0)
        pct = min(100, int(err_c * 100 / max(max_e, 1))) if max_e else 0
        bar_color = "var(--crt)" if err_c > 10 else "var(--wrn)"
        rows += (
            f"<tr><td>{esc(tgt)}</td><td>{err_c}</td><td>{warn_c}</td>"
            f'<td><div style="width:{pct}%;height:8px;background:{bar_color};border-radius:2px"></div></td></tr>'
        )
    if not rows:
        rows = '<tr><td colspan="4" class="dim">clean — no errors tracked</td></tr>'
    table = f"<table><thead><tr><th>Module</th><th>Errors</th><th>Warnings</th><th>Load</th></tr></thead><tbody>{rows}</tbody></table>"
    body = f"<p class='tagline'>{len(agg)} module(s) tracked</p><div class='panel'>{table}</div>"
    return _page("Error Attribution", body, active="errors")


@app.get("/privacy", response_class=HTMLResponse)
def privacy_page(request: Request) -> HTMLResponse:  # noqa: ARG001
    legal_dir = Path(__file__).resolve().parent / "legal"
    # also check meta/legal for migrated layout
    alt_legal = Path(__file__).resolve().parents[2] / "meta" / "legal"
    candidates = [legal_dir, alt_legal]

    def load_md(name: str) -> str:
        for d in candidates:
            fp = d / name
            if fp.exists():
                return fp.read_text(encoding="utf-8", errors="replace")
        return ""

    de_raw = load_md("DATENSCHUTZ-TELEMETRIE.md")
    en_raw = load_md("PRIVACY-TELEMETRY.md")

    def md_html(text: str) -> str:
        if not text:
            return '<p class="dim">Not available.</p>'
        out: list[str] = []
        for ln in text.split("\n"):
            e_ln = esc(ln)
            if ln.startswith("# "):
                out.append(f"<h2>{e_ln[2:]}</h2>")
            elif ln.startswith("## "):
                out.append(f"<h3>{e_ln[3:]}</h3>")
            elif ln.startswith("- "):
                out.append(f"<li>{e_ln[2:]}</li>")
            elif ln.strip():
                out.append(f"<p>{e_ln}</p>")
        return "\n".join(out)

    de_html = md_html(de_raw)
    en_html = md_html(en_raw)
    body = f"<div class='panel'><h2>Datenschutzerklärung</h2>{de_html}</div><div class='panel'><h2>Privacy Statement</h2>{en_html}</div>"
    return _page("Privacy & GDPR", body, active="privacy")


@app.get("/healthz")
async def healthz_endpoint() -> dict[str, Any]:
    try:
        cnt = db.count()
    except Exception:
        cnt = 0
    return {"ok": True, "count": cnt}


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(
        app,
        host=os.environ.get("LEGION_PORTAL_HOST", "127.0.0.1"),
        port=int(os.environ.get("LEGION_PORTAL_PORT", "8788")),
    )
