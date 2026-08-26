#!/usr/bin/env python3
"""Legion Telemetry Operations Portal."""
from __future__ import annotations

import html as _h
import json as _json
import math, os, sys
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse, PlainTextResponse

if __package__:
    from . import db
else:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import db

SCAN_LIMIT = 5000


def esc(v):
    return _h.escape(str(v if v is not None else ""), quote=True)

def parse_ts(v):
    if v is None or isinstance(v, bool): return None
    try: return datetime.fromtimestamp(float(v), tz=timezone.utc)
    except Exception: pass
    t = str(v).strip(); iso = t[:-1]+"+00:00" if t and t[-1] in "Zz" else t
    try: d = datetime.fromisoformat(iso); return d.replace(tzinfo=timezone.utc) if d.tzinfo is None else d
    except Exception: return None

def fmt_ts(v):
    d = parse_ts(v)
    return d.strftime("%m-%d %H:%M") if d else "—"

def load_reports():
    rows = db.recent(SCAN_LIMIT); out = []
    for row in rows:
        e = dict(row); raw = db.get_payload(e["id"])
        try:
            p = json.loads(raw or "{}"); e["_p"] = p
            e["_s"] = p.get("sensors",{}); e["_b"] = p.get("battery",{})
            e["_f"] = p.get("fans",[]); e["_fl"] = p.get("faults",[])
            e["_ts_dt"] = parse_ts(p.get("generated_at"))
        except Exception:
            e.update(_p={},_s={},_b={},_f=[],_fl=[],_ts_dt=None)
        out.append(e)
    out.sort(key=lambda r: r.get("_ts_dt") or datetime.min.replace(tzinfo=timezone.utc))
    return out


_CSS = """\
:root{--bg:#0a0e0c;--p:#0e1412;--ln:#1e2b23;--fg:#b7ccb9;--dim:#6f8a77;
--ac:#46f08a;--wrn:#ffb454;--crt:#e85c5c;--inf:#7ab8d4;--blu:#5cb3e8;--purp:#b48ce8}
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
.b-ok{background:rgba(70,240,138,.1);color:var(--ac)}.b-warn{background:rgba(255,180,84,.1);color:var(--wrn)}.b-crit{background:rgba(232,92,92,.12);color:var(--crt)}.b-info{background:rgba(122,184,212,.1);color:var(--info)}
.dim{color:var(--dim)}.green{color:var(--ac)}.warn-c{color:var(--wrn)}.red-c{color:var(--crt)}
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
.big-num{font-size:2.2rem;font-weight:700;color:var(--ac)}
.sub-num{font-size:.72rem;color:var(--dim);margin-top:-.3rem;margin-bottom:.5rem}
.spark-poly{fill:none;stroke-width:1.5;stroke-linejoin:round;stroke-linecap:round}
.donut-seg:hover{opacity:.75;cursor:pointer}
.filter-bar{display:flex;gap:.5rem;align-items:center;margin-bottom:.7rem;flex-wrap:wrap}
.filter-bar input[type=text]{background:var(--p);border:1px solid var(--ln);color:var(--fg);padding:.3rem .55rem;border-radius:3px;font-family:inherit;font-size:.78rem;width:160px}
.filter-bar select{background:var(--p);border:1px solid var(--ln);color:var(--fg);padding:.3rem .5rem;border-radius:3px;font-family:inherit;font-size:.78rem}
.trend-chart{width:100%;height:auto;margin-top:.3rem}
.legend-dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:.3rem;vertical-align:middle}
.section-hdr{font-size:.72rem;text-transform:uppercase;letter-spacing:.14em;color:var(--dim);
margin:1.3rem 0 .45rem;display:flex;align-items:center;gap:.4rem}
.section-hdr::after{content:'';flex:1;border-bottom:1px solid var(--ln)}
"""


def _page(title, body, active=""):
    nav_items = [("Dashboard","/","dashboard"),("Reports","/reports","reports"),
                 ("Machines","/machines","machines"),("Faults","/faults","faults"),
                 ("Errors","/errors","errors"),("Privacy","/privacy","privacy")]
    nav = "".join(f'<a href="{href}"{" class=on" if act==active else ""}>{label}</a>'
                  for label,href,act in nav_items)
    return HTMLResponse(f'''<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width"><meta name="robots" content="noindex,nofollow">
<title>{title} — Legion Telemetry</title><style>{_CSS}</style></head>
<body><nav><span class="logo">&gt;_ legion</span>{nav}</nav>
<h1><span class="prompt">&gt;</span> {title}</h1>{body}
<div class="foot">Legion Control telemetry · alpha · Tailscale-only operator access</div></body></html>''')


def load_reports():
    rows = db.recent(SCAN_LIMIT); out = []
    for row in rows:
        e = dict(row); raw = db.get_payload(e["id"])
        try:
            p = json.loads(raw or "{}"); e["_p"]=p
            e["_s"]=p.get("sensors",{}); e["_b"]=p.get("battery",{})
            e["_f"]=p.get("fans",[]); e["_fl"]=p.get("faults",[])
            e["_ts_dt"] = parse_ts(p.get("generated_at"))
        except Exception: e.update(_p={},_s={},_b={},_f=[],_fl=[],_ts_dt=None)
        out.append(e)
    out.sort(key=lambda r: r.get("_ts_dt") or datetime.min.replace(tzinfo=timezone.utc))
    return out


# ─── Dashboard ───────────────────────────────────────────────────────────────

@app.get("/", response_class=HTMLResponse)
async def dashboard_page():
    reps = load_reports()
    n = len(reps)
    machines = len({r.get("_p",{}).get("machine_id","") for r in reps})
    last24 = sum(1 for r in reps if (parse_ts(r.get("_p",{}).get("generated_at")) or datetime.min.replace(tzinfo=timezone.utc)) >= datetime.now(timezone.utc)-timedelta(hours=24))
    crit_count = sum(len([f for f in r.get("_fl",[]) if f.get("severity")=="Critical"]) for r in reps)
    warn_count = sum(len([f for f in r.get("_fl",[]) if f.get("severity")=="Warning"]) for r in reps)

    s = reps[0].get("_s",{}) if reps else {}
    temp_bars = ""
    for label, key in [("CPU","cpu_temp"),("CCD1","cpu_temp_1"),("CCD2","cpu_temp_2"),
                       ("dGPU","dgpu_temp"),("iGPU","igpu_edge"),("EC CPU","ec_cpu")]:
        val = s.get(key)
        if val is None or val < 0: continue
        pct = min(100,max(0,(val-20)/(100-20)*100))
        col = "var(--crt)" if val>=90 else "var(--wrn)" if val>=75 else "var(--ac)"
        temp_bars += f'<div class="hbar"><span class="hl">{label}</span>'
        temp_bars += f'<div class="ht"><div class="hf" style="width:{pct:.0f}%;background:{col}"></div></div>'
        temp_bars += f'<span class="hv">{val:.1f}°C</span></div>\n'
    for i,t in enumerate(s.get("ssd_composite",[])):
        col = "var(--crt)" if t>=80 else "var(--wrn)" if t>=60 else "var(--ac)"
        temp_bars += f'<div class="hbar"><span class="hl">NVMe {i}</span>'
        temp_bars += f'<div class="ht"><div class="hf" style="width:{min(100,t):.0f}%;background:{col}"></div></div>'
        temp_bars += f'<span class="hv">{t:.1f}°C</span></div>\n'

    fan_bars = ""
    latest_fans = reps[0].get("_f",[]) if reps else []
    for f in latest_fans:
        rpm=f.get("rpm",0); mx=f.get("max_rpm",5000)
        pct=min(100,rpm/mx*100) if mx else 0
        col="var(--crt)" if pct>80 else "var(--warn)" if pct>50 else "var(--ac)"
        fan_bars+=f'<div class="hbar"><span class="hl">Fan {f.get("id","?")}</span>'
        fan_bars+=f'<div class="ht"><div class="hf" style="width:{pct:.0f}%;background:{col}"></div></div>'
        fan_bars+=f'<span class="hv">{rpm} RPM</span></div>\n'

    act_rows=""
    for r in reversed(reps[-10:]):
        ts_d=r.get("_ts_dt"); ts_str=ts_d.strftime("%m-%d %H:%M") if ts_d else "?"
        model=r.get("_p",{}).get("device",{}).get("model","?")
        nf=len(r.get("_fl",[]))
        fc=f'<span class="badge {"b-crit" if nf else "b-ok"}">{nf}</span>'
        act_rows+=f"<tr><td>{ts_str}</td><td>{esc(model)}</td><td>{fc}</td></tr>"

    cards="".join([
        f'<div class="card"><b>{total_r}</b><span>Total Reports</span></div>',
        f'<div class="card bluc"><b>+{last24}</b><span>Last 24 Hours</span></div>',
        f'<div class="card"><b>{machines}</b><span>Machines</span></div>',
        f'<div class="card wrnc"><b class="warn">{warn_faults}</b><span>Warnings</span></div>',
        f'<div class="card crtc"><b class="red">{crit_count}</b><span>Critical Faults</span></div>',
    ])

    body=f"""
<p class="tagline">Anonymous diagnostics · alpha · Tailscale-only operator access</p>
<div class="cards">{cards}</div>
<div class="grid2">
<div>
<h2>Sensor Temperatures</h2>
<div class="panel">{temp_bars or '<span class="dim">No data yet</span>'}</div>
<h2>Fan Status</h2>
<div class="panel">{fan_bars or '<span class="dim">No fans detected</span>'}</div>
</div><div>
<h2>Fault Distribution</h2>
<div class="panel">{fault_donut}</div>
<h2>Fault Details</h2>
<div class="panel">{fault_details_html or '<span class="dim">Clean</span>'}</div>
</div></div>
<h2>Recent Activity</h2>
<div class="panel"><table><thead><tr><th>Time</th><th>Model</th><th>Faults</th></tr></thead>
<tbody>{act_rows or '<tr><td colspan="3" class="dim">none</td></tr>'}</tbody></table></div>
"""
    return _page("Dashboard", body)


@app.get("/reports", response_class=HTMLResponse)
def reports_list_page():
    reports = load_reports()
    rows=""
    for r in reversed(reports):
        rid=r["id"]; ts=fmt_ts(r.get("_p",{}).get("generated_at"))
        mid=str(r.get("machine_id","—"))[:8]
        distro=esc(r.get("_p",{}).get("os",{}).get("distro","?"))
        model=esc(r.get("_p",{}).get("device",{}).get("model","?"))
        nf=len(r.get("_fl",[]))
        fc=f'<span class="badge b-crit">{nf}</span>' if nf else '<span class="badge b-ok">0</span>'
        rows+=f"<tr><td><a href='/reports/{rid}'>#{rid}</a></td><td>{ts}</td><td>{mid}</td><td>{distro}</td><td>{model}</td><td>{fc}</td></tr>"
    body=f"<h2>All Reports ({len(reports)})</h2><div class='panel'><table style='margin-top:0'><thead><tr><th>ID</th><th>Time</th><th>Machine ID</th><th>Distro</th><th>Model</th><th>Faults</th></tr></thead><tbody>{rows or '<tr><td colspan=\"6\" class=\"dim\">empty</td></tr>'}</tbody></table></div>"
    return _page("Reports", body)


@app.get("/machines", response_class=HTMLResponse)
def machines_view_page():
    reports = load_reports()
    groups: dict[str,list] = {}
    for r in reports:
        mid = r.get("_p",{}).get("machine_id") or r.get("_p",{}).get("device",{}).get("model","unknown")
        groups.setdefault(mid, []).append(r)
    cards=""
    for mid, grp in sorted(groups.items()):
        last=grp[-1]
        dev=last.get("_p",{}).get("device",{}); os_d=last.get("_p",{}).get("os",{}).get("distro","?")
        nflts=sum(len(g.get("_fl",[])) for g in grp)
        hc="#46f08a" if not nflts else ("#ffb454" if nflts<5 else "#e85c5c")
        cards+=f'''<div class="mc" style="border-left:3px solid {hc}">
<h3>{dev.get("model","?")}</h3>
<div class="meta">ID: {mid[:12]}…<br>Distro: {os_d}<br>
Reports: {len(grp)} | Faults: {nflts}<br>Last: {last.get("ts","?")[:16]}</div></div>'''
    body=f"<h2>Machines ({len(groups)})</h2><div class='grid3'>{cards or '<p class=\"dim\">none</p>'}</div>"
    return _page("Machines", body)


@app.get("/faults", response_class=HTMLResponse)
def faults_view_page():
    reports=load_reports()
    sections=""; total=0
    for r in reversed(reports):
        flts=r.get("_fl",[])
        if not flts: continue
        ts=fmt_ts(r.get("_p",{}).get("generated_at"))
        items="".join(
            f"<div class='fe'><span class='badge {'b-crit' if f.get('severity')=='Critical' else 'b-warn'}'>"
            f"{f.get('severity','?')}</span> <strong>{f.get('id','')}</strong> — {f.get('detail','')}</div>"
            for f in flts)
        sections+=f"<h2>{ts}</h2><div class='panel'>{items}</div>"
        total+=len(flts)
    body=f"<h2>Fault History ({total} entries)</h2>"+(sections or "<p>No faults recorded.</p>")
    return _page("Fault Tracker", body)
