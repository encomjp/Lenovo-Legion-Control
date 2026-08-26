#!/usr/bin/env python3
"""Legion Control — operator portal for diagnostics reports.

Multi-page operations dashboard: overview stats, fault tracker, per-machine
health grid, sensor bars, error attribution, report browser.  Server-rendered;
no external JS/CSS/CDN dependencies.

Routes:
    /            overview dashboard (stats + charts + recent activity)
    /reports     browsable report table
    /reports/{id} detailed report viewer
    /machines    grouped by machine_id
    /faults      fault/anomaly tracker across all reports
    /errors      log error attribution by module
    /privacy     data collection statement
    /healthz     liveness probe
"""

from __future__ import annotations

import html
import json
import logging
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse, JSONResponse

if __package__:
    from . import db
else:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import db

log = logging.getLogger("portal")
RECENT_LIMIT = 200
SCAN_LIMIT = 5000
DEFAULT_PORT = 8788


# ─── helpers ────────────────────────────────────────────────────────────────

def esc(v: Any) -> str:
    return html.escape(str(v if v is not None else ""), quote=True)


def _parse_ts(v: Any) -> datetime | None:
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
    d = _parse_ts(v)
    return d.strftime("%Y-%m-%d %H:%M UTC") if d else esc(str(v or ""))


def rel_time(v: Any) -> str:
    d = _parse_ts(v)
    if not d: return "—"
    delta = datetime.now(timezone.utc) - d
    secs = int(delta.total_seconds())
    if secs < 0: secs = 0
    if secs < 60: return f"{secs}s ago"
    if secs < 3600: return f"{secs//60}m ago"
    if secs < 86400: return f"{secs//3600}h {secs%3600//60}m ago"
    return f"{secs//86400}d {secs%86400//3600}h ago"


def load_all_reports() -> list[dict]:
    """Fetch all reports and parse payloads for rich rendering."""
    rows = db.recent(SCAN_LIMIT)
    out = []
    for row in rows:
        entry = dict(row)
        raw = db.get_payload(entry["id"])
        if raw:
            try:
                p = json.loads(raw)
                entry["_payload"] = p
                entry["_sensors"] = p.get("sensors", {})
                entry["_battery"] = p.get("battery", {})
                entry["_fans"] = p.get("fans", [])
                entry["_faults"] = p.get("faults", [])
                entry["_log"] = p.get("log_digest", {})
                entry["_system"] = p.get("system_info", {})
            except Exception:
                entry["_payload"] = {}
                entry["_sensors"] = {}
                entry["_battery"] = {}
                entry["_fans"] = []
                entry["_faults"] = []
                entry["_log"] = {}
                entry["_system"] = {}
        else:
            for k in ("_payload","_sensors","_battery","_fans","_faults","_log","_system"):
                entry[k] = {} if not k.endswith("s") or k == "_fans" else []
        out.append(entry)
    return out


def _bar(pct: float, color: str, label: str) -> str:
    pct = max(0, min(100, pct))
    return (
        f'<div class="bar-row"><span class="bar-label">{esc(label)}</span>'
        f'<div class="bar-track"><div class="bar-fill" style="width:{pct:.0f}%;background:{color}"></div></div>'
        f'<span class="bar-val">{pct:.0f}%</span></div>'
    )


def _sev_badge(sev: str) -> str:
    cls = {"critical":"sev-crit","warning":"sev-warn","info":"sev-info"}.get(sev.lower(), "sev-info")
    return f'<span class="{cls}">{esc(sev.upper())}</span>'


def _ok_badge(ok: bool) -> str:
    return f'<span class="{"ok-badge" if ok else "fail-badge"}">{"✓" if ok else "✗"}</span>'


# ─── CSS ────────────────────────────────────────────────────────────────────

_CSS = """\
:root{--bg:#0a0e0c;--panel:#101613;--line:#1e2b23;--fg:#b7ccb9;--dim:#6f8a77;
--accent:#46f08a;--warn:#ffb454;--crit:#e85c5c;--info:#7ab8d4;--ok:#46f08a}
*{box-sizing:border-box;margin:0;padding:0}
html{background:var(--bg);color-scheme:dark}
body{margin:0 auto;max-width:80rem;padding:1.5rem 1.25rem 4rem;color:var(--fg);
font-family:'JetBrains Mono','Fira Code',ui-monospace,Menlo,Consolas,monospace;
font-size:.87rem;line-height:1.55;
background:radial-gradient(ellipse at top,rgba(70,240,138,.05),transparent 70%),
repeating-linear-gradient(0deg,rgba(255,255,255,.012) 0 1px,transparent 1px 3px),var(--bg)}
nav{display:flex;gap:.15rem;margin-bottom:1.75rem;border-bottom:1px solid var(--line);padding-bottom:.65rem;flex-wrap:wrap}
nav a{color:var(--dim);text-decoration:none;padding:.35rem .8rem;border-radius:4px;font-weight:600;font-size:.82rem;text-transform:uppercase;letter-spacing:.08em}
nav a:hover{color:var(--fg);background:rgba(255,255,255,.05)}
nav a.on{color:var(--accent);background:rgba(70,240,138,.08)}
h1{font-size:1.3rem;margin:0 0 .2rem;letter-spacing:.02em}
h1 .prompt{color:var(--accent);margin-right:.4rem}
h2{color:var(--dim);font-size:.78rem;text-transform:uppercase;letter-spacing:.14em;margin:1.8rem 0 .65rem}
.tagline{color:var(--dim);margin:0 0 1.5rem;font-size:.83rem}
.cards{display:flex;gap:.75rem;flex-wrap:wrap;margin-bottom:1.75rem}
.card{border:1px solid var(--line);background:var(--panel);padding:.9rem 1.15rem;min-width:10.5rem;border-radius:5px;flex:1;min-width:9rem}
.card b{display:block;font-size:1.8rem;color:var(--accent);font-weight:600}
.card.warn b{color:var(--warn)}.card.crit b{color:var(--crit)}
.card span{color:var(--dim);font-size:.72rem;text-transform:uppercase;letter-spacing:.1em}
table{width:100%;border-collapse:collapse;margin-top:.3rem}
th,td{border:none;padding:.42rem .65rem;text-align:left;white-space:nowrap}
th{color:var(--dim);font-size:.68rem;text-transform:uppercase;letter-spacing:.12em;border-bottom:1px solid var(--line)}
td{border-bottom:1px solid rgba(30,43,35,.5)}
tr:hover td{background:rgba(255,255,255,.03)}
a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
.sev-crit{color:var(--crit);font-weight:700}.sev-warn{color:var(--warn);font-weight:600}.sev-info{color:var(--info)}
.ok-badge{color:var(--accent);font-weight:700}.fail-badge{color:var(--crit);font-weight:700}
.bar-track{background:rgba(255,255,255,.06);border-radius:3px;height:14px;min-width:120px;overflow:hidden}
.bar-fill{height:100%;border-radius:3px}
.bar-row{display:flex;align-items:center;gap:.55rem;margin:.28rem 0}
.bar-label{min-width:130px;color:var(--dim);font-size:.78rem;text-align:right}
.bar-val{min-width:45px;font-size:.8rem}
.panel{border:1px solid var(--line);background:var(--panel);border-radius:5px;padding:1rem 1.15rem;margin-bottom:1rem}
.grid2{display:grid;grid-template-columns:1fr 1fr;gap:.75rem}
.grid3{display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));gap:.75rem}
.machine-card{border:1px solid var(--line);background:var(--panel);border-radius:5px;padding:.85rem 1rem}
.machine-card h3{font-size:.88rem;color:var(--fg);margin-bottom:.3rem}
.machine-card .meta{color:var(--dim);font-size:.76rem}
.fault-item{padding:.5rem 0;border-bottom:1px solid rgba(30,43,35,.5)}
.fault-item:last-child{border:none}
pre{border:1px solid var(--line);background:var(--panel);padding:1rem;border-radius:4px;overflow-x:auto;font-size:.82rem;line-height:1.5;color:var(--fg)}
code{color:var(--accent)}
.dim{color:var(--dim)}.foot{margin-top:2.5rem;color:var(--dim);font-size:.75rem}
.badge{display:inline-block;padding:.12rem .5rem;border-radius:3px;font-size:.74rem;font-weight:600}
.badge-ok{background:rgba(70,240,138,.12);color:var(--accent)}
.badge-warn{background:rgba(255,180,84,.12);color:var(--warn)}
.badge-crit{background:rgba(232,92,92,.12);color:var(--crit)}
.section{margin-bottom:1.5rem}
"""


def _page(title: str, body: str, active: str = "") -> HTMLResponse:
    nav_items = [("dashboard","/"),("reports","/reports"),("machines","/machines"),("faults","/faults"),("errors","/errors"),("privacy","/privacy")]
    nav = "".join(
        f'<a href="{href}"{" class=on" if href == active else ""}>{label}</a>'
        for label, href in [
            ("Dashboard", "/"), ("Reports", "/reports"), ("Machines", "/machines"),
            ("Faults", "/faults"), ("Errors", "/errors"), ("Privacy", "/privacy"),
        ]
    )
    return HTMLResponse(f"""<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<meta name="robots" content="noindex,nofollow">
<title>{title} — Legion Telemetry</title><style>{_CSS}</style></head>
<body><nav>{nav}</nav><h1><span class="prompt">&gt;_</span>{title}</h1>{body}
<div class="foot">Legion Control telemetry portal · alpha · Tailscale-only access</div>
</body></html>""")


# ─── Dashboard ──────────────────────────────────────────────────────────────

@app.get("/", response_class=HTMLResponse)
def dashboard():
    reports = load_all_reports()
    total = len(reports)
    last24 = sum(1 for r in reports if (_parse_ts(r.get("ts")) or datetime.min.replace(tzinfo=timezone.utc)) >= datetime.now(timezone.utc) - timedelta(hours=24))
    machines = len({r.get("_payload",{}).get("machine_id","") for r in reports if r.get("_payload",{}).get("machine_id")})
    critical_faults = sum(1 for r in reports for f in r.get("_faults",[]) if f.get("severity")=="Critical")

    stat_cards = f"""
<div class="cards">
  <div class="card"><b>{total}</b><span>Total Reports</span></div>
  <div class="card"><b>{last24}</b><span>Last 24 Hours</span></div>
  <div class="card"><b>{machines}</b><span>Machines</span></div>
  <div class="card {'crit' if critical_faults else ''}"><b>{critical_faults}</b><span>Critical Faults</span></div>
</div>"""

    fault_rows = ""
    seen_fault_ids = set()
    for r in reports[:20]:
        for f in r.get("_faults", []):
            fid = f.get("id","?")
            if fid in seen_fault_ids: continue
            seen_fault_ids.add(fid)
            sev = f.get("severity","Warning").lower()
            badge_cls = {"Critical":"badge-crit","Warning":"badge-warn"}.get(sev,"badge-warn")
            fault_rows += f'<div class="fault-item"><span class="badge {badge_cls}">{sev}</span> <strong>{esc(fid)}</strong><br><span class="dim">{esc(f.get("detail",""))}</span></div>'

    if not fault_rows:
        fault_rows = '<div class="dim" style="padding:1rem 0">No active faults detected ✓</div>'

    recent_rows = ""
    for r in reports[:10]:
        ts = fmt_ts(r.get("ts"))
        model = r.get("_payload",{}).get("device",{}).get("model","?")
        mid = r.get("machine_id","?")[:8]
        rid = r.get("id","?")
        distro = r.get("_payload",{}).get("os",{}).get("distro","?")
        n_faults = len(r.get("_faults",[]))
        recent_rows += f"<tr><td>#{rid}</td><td>{esc(ts)}</td><td>{esc(distro)}</td><td>{esc(model)}</td><td>{n_faults} fault(s)</td></tr>"

    body = f"""
<p class="tagline">Anonymous diagnostics from alpha testers · Tailscale-only access</p>
{stat_cards}
<h2>Active Faults</h2>
<div class="panel">{fault_rows}</div>
<h2>Recent Activity</h2>
<div class="panel"><table><thead><tr><th>ID</th><th>Received</th><th>Distro</th><th>Model</th><th>Faults</th></tr></thead>
<tbody>{recent_rows or '<tr><td colspan="5" class="dim">No reports yet</td></tr>'}</tbody></table></div>
"""
    return _page("Dashboard", body, active="/")


@app.get("/reports", response_class=HTMLResponse)
def reports_page():
    reports = load_all_reports()
    rows = ""
    for r in reversed(reports):
        rid = r["id"]
        ts = fmt_ts(r.get("ts"))
        model = r.get("_payload",{}).get("device",{}).get("model","?")
        distro = r.get("_payload",{}).get("os",{}).get("distro","?")
        nfaults = len(r.get("_faults",[]))
        mid = r.get("machine_id","—")[:8]
        rows += f'<tr><td><a href="/reports/{rid}">#{rid}</a></td><td>{esc(ts)}</td><td>{esc(mid)}</td><td>{esc(distro)}</td><td>{esc(model)}</td><td>{nfaults}</td></tr>'

    body = f"""<h2>All Reports ({len(reports)})</h2>
<div class="panel"><table style="margin-top:0"><thead><tr>
<th>ID</th><th>Received</th><th>Machine</th><th>Distro</th><th>Model</th><th>Faults</th>
</tr></thead><tbody>{rows or '<tr><td colspan="6" class="dim">empty</td></tr>'}</tbody></table></div>"""
    return _page("Reports", body)


@app.get("/machines", response_class=HTMLResponse)
def machines_page():
    reports = load_all_reports()
    by_machine: dict[str, list[dict]] = {}
    for r in reports:
        mid = r.get("_payload",{}).get("machine_id") or r.get("_payload",{}).get("device",{}).get("model","?")
        by_machine.setdefault(mid, []).append(r)

    cards = ""
    for mid, group in sorted(by_machine.items()):
        latest = group[0]
        model = latest.get("_payload",{}).get("device",{}).get("model","Unknown")
        distro = latest.get("_payload",{}).get("os",{}).get("distro","?")
        nfaults = sum(len(g.get("_faults",[])) for g in group)
        health_color = "#46f08a" if nfaults == 0 else ("#ffb454" if nfaults < 5 else "#e85c5c")
        cards += f"""
<div class="machine-card" style="border-left:3px solid {health_color}">
  <h3>{esc(model)}</h3>
  <div class="meta">machine_id: {esc(mid[:12])}…<br>
  distro: {esc(distro)}<br>
  reports: {len(group)} · active faults: {nfaults}<br>
  last seen: {fmt_ts(latest.get('ts'))}</div>
</div>"""

    body = f"<h2>Machines ({len(by_machine)})</h2><div class='grid3'>{cards or '<p class=\"dim\">No reports</p>'}</div>"
    return _page("Machines", body)


@app.get("/faults", response_class=HTMLResponse)
def faults_page():
    reports = load_all_reports()
    sections = ""
    total = 0
    for r in reversed(reports):
        faults = r.get("_faults",[])
        if not faults: continue
        ts = fmt_ts(r.get("ts"))
        model = r.get("_payload",{}).get("device",{}).get("model","?")
        items = "".join(
            f'<div class="fault-item"><span class="badge {badge_cls}">{sev}</span> <strong>{esc(fid)}</strong><br><span class="dim">{esc(det)}</span></div>'
            for fid, sev, det in [(f.get("id","?"),f.get("severity","Warning"),f.get("detail","")) for f in faults]
            for badge_cls, sev in [(sev.lower(),"")]
            for sev in [sev]
        )
        # simpler rendering
        items = "".join(
            f'<div class="fault-item"><span class="badge {"badge-crit" if f.get("severity")=="Critical" else "badge-warn"}">{f.get("severity","?")}</span> '
            f'{esc(f.get("id",""))} — {esc(f.get("detail",""))}</div>'
            for f in faults
        )
        sections += f'<h2>{esc(model)} — {ts}</h2><div class="panel">{items}</div>'
        total += len(faults)

    body = f"<p class='tagline'>{total} fault entries across {len(sections)} report groups</p>{sections or '<p class=\"dim\">No faults recorded.</p>'}"
    return _page("Fault Tracker", body)


@app.get("/errors", response_class=HTMLResponse)
def errors_page():
    reports = load_all_reports()
    target_counts: dict[str, dict[str, int]] = {}
    for r in reports:
        ld = r.get("_log",{})
        for tgt, cnt in (ld.get("errors_by_target") or {}).items():
            if tgt not in target_counts:
                target_counts[tgt] = {"ERROR":0,"WARN":0}
            target_counts[tgt]["ERROR"] = target_counts[tgt].get("ERROR",0) + cnt

    rows = ""
    for tgt, counts in sorted(target_counts.items(), key=lambda x: -x[1].get("ERROR",0)):
        err_c = counts.get("ERROR",0)
        warn_c = counts.get("WARN",0)
        max_err = max((c["ERROR"] for c in target_counts.values()), default=1) or 1
        pct_w = min(100, warn_c * 100 // max(max(warn_c for c in target_counts.values()),1))
        pct_e = min(100, err_c * 100 // max(err_c for c in target_counts.values() if c.get("ERROR")), default=100)
        rows += (
            f'<tr><td><strong>{esc(tgt)}</strong></td>'
            f'<td>{err_c}</td><td>{warn_c}</td>'
            f'<td><div class="bar-track" style="max-width:200px">'
            f'<div class="bar-fill" style="width:{min(100,err_c*10)}%;background:{":var(--crit)" if err_c > 5 else "var(--warn)"}"></div></div></td></tr>'
        )

    table = f"""<table><thead><tr><th>Module</th><th>Errors</th><th>Warnings</th><th>Load</th></tr></thead>
<tbody>{rows or '<tr><td colspan="3" class="dim">No errors recorded</td></tr>'}</tbody></table>"""

    body = f"""<p class='tagline'>Error attribution by module — aggregated across all stored reports</p>
<div class='panel'>{table}</div>
<p class='dim'>These counts come from the daemon's structured log ring buffer.
Each entry records the originating module so you can pinpoint which subsystem is producing noise.</p>"""
    return _page("Error Attribution", body)


@app.get("/privacy", response_class=HTMLResponse)
def privacy_page():
    legal_dir = Path(__file__).resolve().parent / "legal"
    de_text = (legal_dir / "DATENSCHUTZ-TELEMETRIE.md").read_text() if (legal_dir / "DATENSCHUTZ-TELEMETRIE.md").exists() else "Not available."
    en_text = (legal_dir / "PRIVACY-TELEMETRY.md").read_text() if (legal_dir / "PRIVACY-TELEMETRY.md").exists() else "Not available."

    def md_to_html(text: str) -> str:
        lines_out = []
        for ln in text.split("\n"):
            if ln.startswith("# "):
                lines_out.append(f"<h2>{esc(ln[2:])}</h2>")
            elif ln.startswith("## "):
                lines_out.append(f"<h3>{esc(ln[3:])}</h3>")
            elif ln.strip() == "---":
                lines_out.append("<hr>")
            elif ln.startswith("- "):
                lines_out.append(f"<li>{esc(ln[2:])}</li>")
            elif ln.strip():
                lines_out.append(f"<p>{esc(ln)}</p>")
        return "\n".join(lines_out)

    de_html = md_to_html(de_text)
    en_html = md_to_html(en_text)

    body = f"""
<div class='panel'><h2>Datenschutzerklärung (Deutsch)</h2>{de_html}</div>
<div class='panel'><h2>Privacy Statement (English)</h2>{en_html}</div>
"""
    return _page("Privacy & GDPR", body)


@app.get("/healthz")
def healthz():
    return {"ok": True}


@app.get("/reports/{{rid}}", response_class=HTMLResponse)
def report_detail(rid: int):
    payload_raw = db.get_payload(rid)
    if payload_raw is None:
        return _page("Report Not Found", "<p>Report not found.</p>")
    doc = json.loads(payload_raw)
    dev = doc.get("device", {})
    os_d = doc.get("os", {})
    sensors = doc.get("sensors", {})
    battery = doc.get("battery", {})
    faults = doc.get("faults", [])
    system = doc.get("system_info", {})
    log_digest = doc.get("log_digest", {})

    def section(title: str, items: list[tuple[str, str]]) -> str:
        rows = "".join(f"<tr><td>{k}</td><td>{v}</td></tr>" for k, v in items)
        return f"<h2>{title}</h2><table>{rows}</table>"

    parts = [section("Device", [
        ("Model", dev.get("model","?")),
        ("Machine Type", dev.get("machine_type","?")),
        ("BIOS", dev.get("bios_version","?")),
        ("CPU", dev.get("cpu_model","?")),
        ("GPU", dev.get("gpu_model","?")),
    ])]
    parts.append(section("OS", [
        ("Distro", os_d.get("distro","?")),
        ("Kernel", os_d.get("kernel","?")),
    ]))

    sensor_items = []
    for key, label in [("cpu_temp","CPU Temp"),("dgpu_temp","dGPU Temp"),("igpu_edge","iGPU Edge"),
                       ("ec_cpu","EC CPU"),("ec_gpu","EC GPU"),("dgpu_power","dGPU Power"),
                       ("dgpu_clock","dGPU Clock"),("cpu_power","CPU Power")]:
        val = sensors.get(key)
        if val is not None and val != -1.0:
            unit = "W" if "power" in key else ("MHz" if "clock" in key else "°C")
            sensor_items.append((label, f"{val:.1} {unit}" if isinstance(val, float) else str(val)))
    for i, t in enumerate(sensors.get("ssd_composite",[])):
        sensor_items.append((f"NVMe {i}", f"{t:.1f} °C"))
    for i, t in enumerate(sensors.get("ram_temps",[])):
        sensor_items.append((f"RAM {i}", f"{t:.1f} °C"))
    parts.append(section("Sensors", sensor_items))

    bat_items = []
    for key, label in [("capacity_pct","Capacity"),("status","Status"),("voltage_v","Voltage"),
                       ("cycles","Cycles"),("health_pct","Health"),("charge_limit_pct","Limit")]:
        v = battery.get(key)
        if v is not None:
            suffix = "%" if "pct" in key or key == "capacity_pct" else (" V" if key == "voltage_v" else "")
            bat_items.append((label, f"{v}{suffix}"))
    parts.append(section("Battery", bat_items))

    fault_items = "".join(
        f'<div class="fault-item"><strong>{esc(f.get("id",""))}</strong>'
        f' [{esc(f.get("severity",""))}]<br>{esc(f.get("detail",""))}</div>'
        for f in faults
    ) or "<em>No faults detected</em>"
    parts.append(f"<h2>Faults ({len(faults)})</h2>{fault_items}")

    body = f"<h2>Report #{rid}</h2>" + "".join(parts) + f"<h2>Payload</h2><pre>{esc(payload_raw)}</pre>"
    return _page(f"Report #{rid}", body)


# ─── launcher ───────────────────────────────────────────────────────────────

if __name__ == "__main__":
    import uvicorn
    host = os.environ.get("LEGION_PORTAL_HOST", "127.0.0.1")
    port = int(os.environ.get("LEGION_PORTAL_PORT", "8788"))
    uvicorn.run(app, host=host, port=port)
