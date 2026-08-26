#!/usr/bin/env python3
"""Legion Telemetry Operations Portal — multi-page dashboard with SVG charts."""
from __future__ import annotations

import html, json, math, os, sys
from datetime import datetime, timedelta, timezone
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse

if __package__:
    from . import db
else:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import db

SCAN_LIMIT = 5000


def esc(v): return html.escape(str(v if v is not None else ""), quote=True)


def parse_ts(v):
    if v is None or isinstance(v, bool): return None
    try: return datetime.fromtimestamp(float(v), tz=timezone.utc)
    except Exception: pass
    t = str(v).strip()
    iso = t[:-1] + "+00:00" if t and t[-1] in "Zz" else t
    try: d = datetime.fromisoformat(iso); return d.replace(tzinfo=timezone.utc) if d.tzinfo is None else d
    except Exception: return None


def fmt_ts(v):
    d = parse_ts(v)
    return d.strftime("%m-%d %H:%M") if d else "—"


def rel_time(v):
    d = parse_ts(v)
    if not d: return "—"
    s = max(0,int((datetime.now(timezone.utc)-d).total_seconds()))
    return f"{s}s" if s<60 else f"{s//60}m" if s<3600 else f"{s//3600}h"


def load_reports():
    rows = db.recent(SCAN_LIMIT)
    out = []
    for row in rows:
        e = dict(row)
        try:
            p = json.loads(db.get_payload(e["id"]) or "{}")
            e["_p"] = p; e["_s"] = p.get("sensors",{}); e["_b"] = p.get("battery",{})
            e["_f"] = p.get("fans",[]); e["_fl"] = p.get("faults",[]); e["_sys"] = p.get("system_info",{})
        except Exception:
            e.update(_p={}, _s={}, _b={}, _f=[], _fl=[], _sys={})
        out.append(e)
    return sorted(out, key=lambda r: r.get("_p",{}).get("machine_id",""), reverse=True)


_CSS = """\
:root{--bg:#0a0e0c;--p:#101613;--ln:#1a2820;--fg:#b5ccb9;--dim:#6b8a73;--ac:#46f08a;
--wrn:#ffb454;--crt:#e85c5c;--inf:#7ab8d4;--blu:#5cb3e8}
*{box-sizing:border-box;margin:0;padding:0}
html{background:var(--bg);color-scheme:dark}
body{margin:0 auto;max-width:82rem;padding:1rem 1rem 3rem;color:var(--fg);
font-family:'JetBrains Mono',Consolas,monospace;font-size:.84rem;line-height:1.55;
background:radial-gradient(ellipse at top,rgba(70,240,138,.04),transparent 70%),var(--bg)}
nav{display:flex;gap:.1rem;margin-bottom:1.2rem;border-bottom:1px solid var(--ln);padding-bottom:.45rem;flex-wrap:wrap}
nav a{color:var(--dim);text-decoration:none;padding:.25rem .6rem;border-radius:3px;font-size:.75rem;text-transform:uppercase;letter-spacing:.08em}
nav a:hover{color:var(--fg);background:rgba(255,255,255,.04)}
nav a.on{color:var(--ac);background:rgba(70,240,138,.08)}
h2{color:var(--dim);font-size:.72rem;text-transform:uppercase;letter-spacing:.14em;margin:1.4rem 0 .5rem}
.tagline{color:var(--dim);margin:0 0 1.2rem;font-size:.78rem}
.cards{display:flex;gap:.55rem;flex-wrap:wrap;margin-bottom:1.2rem}
.card{border:1px solid var(--ln);background:var(--p);padding:.65rem .85rem;border-radius:4px;flex:1;min-width:8.5rem;text-align:center}
.card b{display:block;font-size:1.7rem;color:var(--ac)}
.card.wrnc b{color:var(--wrn)}.card.crtc b{color:var(--crt)}.card.bluc b{color:var(--blu)}
.card span{color:var(--dim);font-size:.64rem;text-transform:uppercase;letter-spacing:.12em}
table{width:100%;border-collapse:collapse}
th{color:var(--dim);font-size:.62rem;text-transform:uppercase;letter-spacing:.12em;border-bottom:1px solid var(--ln);padding:.3rem .5rem;text-align:left}
td{padding:.32rem .5rem;border-bottom:1px solid rgba(30,43,35,.35)}
tr:hover td{background:rgba(255,255,255,.02)}
.panel{border:1px solid var(--ln);background:var(--p);border-radius:4px;padding:.7rem .85rem;margin-bottom:.8rem}
.badge{display:inline-block;padding:.06rem .4rem;border-radius:3px;font-size:.68rem;font-weight:600}
.b-ok{background:rgba(70,240,138,.1);color:var(--ac)}.b-warn{background:rgba(255,180,84,.1);color:var(--wrn)}.b-crit{background:rgba(232,92,92,.12);color:var(--crt)}
.dim{color:var(--dim)}a{color:var(--ac);text-decoration:none}a:hover{text-decoration:underline}
pre{background:var(--p);border:1px solid var(--ln);padding:.7rem;border-radius:3px;overflow-x:auto;font-size:.78rem;color:var(--fg)}
.mc{border:1px solid var(--ln);border-left:3px solid var(--ac);background:var(--p);border-radius:4px;padding:.65rem .85rem}
.mc h3{font-size:.82rem;margin-bottom:.2rem}.mc .meta{color:var(--dim);font-size:.72rem;line-height:1.4}
.fe{padding:.35rem 0;border-bottom:1px solid rgba(30,43,35,.3);font-size:.78rem}
.fe:last-child{border:none}
.grid2{display:grid;grid-template-columns:1fr 1fr;gap:.65rem}
.grid3{display:grid;grid-template-columns:repeat(auto-fill,minmax(210px,1fr));gap:.65rem}
.hbar{display:flex;align-items:center;gap:.4rem;margin:.14rem 0}
.hl{min-width:90px;text-align:right;color:var(--dim);font-size:.74rem}
.ht{flex:1;height:9px;background:rgba(255,255,255,.04);border-radius:2px;overflow:hidden;min-width:50px}
.hf{height:100%;border-radius:2px}
.hv{min-width:48px;font-size:.74rem}
.foot{margin-top:2rem;color:var(--dim);font-size:.68rem}
.big-num{font-size:1.6rem;font-weight:600}
"""


def _page(title, body, active=""):
    navs = [("Dashboard","/","dashboard"),("Reports","/reports","reports"),
            ("Machines","/machines","machines"),("Faults","/faults","faults"),
            ("Errors","/errors","errors"),("Privacy","/privacy","privacy")]
    nav = "".join(f'<a href="{href}"{" class=on" if act==active else ""}>{label}</a>' for label,href,act in
                  [("Dashboard","/","dashboard"),("Reports","/reports","reports"),
                   ("Machines","/machines","machines"),("Faults","/faults","faults"),
                   ("Errors","/errors","errors"),("Privacy","/privacy","privacy")])
    return HTMLResponse(f'''<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width"><meta name="robots" content="noindex,nofollow">
<title>{title} — Legion Telemetry</title><style>{_CSS}</style></head>
<body><nav><span class="logo">&gt;_ legion</span>{nav}</nav>
<h1><span class="prompt">&gt;</span> {title}</h1>{body}
<div class="foot">Legion Control telemetry · alpha · Tailscale-only</div></body></html>''')


def load_reports():
    rows = db.recent(SCAN_LIMIT)
    out = []
    for row in rows:
        e = dict(row)
        raw = db.get_payload(e["id"])
        try:
            p = json.loads(raw or "{}"); e["_p"] = p
            e["_s"] = p.get("sensors",{}); e["_b"] = p.get("battery",{})
            e["_f"] = p.get("fans",[]); e["_fl"] = p.get("faults",[])
        except Exception:
            e["_p"]=e["_s"]=e["_b"]=e["_fl"]={}; e["_f"]=[]
        out.append(e)
    return out


# ─── Dashboard ───────────────────────────────────────────────────────────────

@app.get("/", response_class=HTMLResponse)
def dashboard():
    reps = load_reports()
    n = len(reps)
    machines = len({r.get("_p",{}).get("machine_id","") for r in reps})
    crit = sum(1 for r in reps for f in r.get("_fl",[]) if f.get("severity")=="Critical")
    warns = sum(1 for r in reps for f in r.get("_fl",[]) if f.get("severity")=="Warning")

    # Sensor bars from latest report
    s = reps[0].get("_s",{}) if reps else {}
    temp_bars = ""
    for label, val, lo, hi, warn in [
        ("CPU", s.get("cpu_temp",0), 20, 100, 80),
        ("dGPU", s.get("dgpu_temp",-1), 20, 100, 80),
        ("CCD1", s.get("cpu_temp_1",-1), 20, 100, 80),
        ("CCD2", s.get("cpu_temp_2",-1), 20, 100, 80)]:
        if val is None or val < 0: continue
        pct = min(100, max(0, (val-lo)/(hi-lo)*100))
        color = "var(--crit)" if val >= 90 else "var(--warn)" if val >= warn else "var(--accent)"
        temp_bars += f'<div class="hbar"><span class="hl">{label}</span><div class="ht"><div class="hf" style="width:{pct:.0f}%;background:{color}"></div></div><span class="hv">{val:.1f}°C</span></div>\n'

    # Fan bars from latest report
    fan_bars = ""
    for f in reps[0].get("_f",[]) if reps else []:
        rpm = f.get("rpm",0); mx = f.get("max_rpm",5000); mn = f.get("min_rpm",0)
        tgt = f.get("target",0)
        pct = min(100, (rpm-mn)/(mx-mn)*100) if mx > mn else 0
        color = "var(--crit)" if rpm < mn else "var(--accent)"
        lbl = f'Fan {f.get("id","?")}'
        tgt_txt = f" → {tgt}" if tgt else ""
        fan_bars += f'<div class="hbar"><span class="hl">{lbl}</span><div class="ht"><div class="hf" style="width:{pct:.0f}%;background:{color}"></div></div><span class="hv">{rpm}{tgt_txt}</span></div>\n'

    # Fault severity donut (SVG)
    sev_counts = {"Critical":0,"Warning":0,"Info":0}
    for r in reps:
        for f in r.get("_fl",[]): sev_counts[f.get("severity","Info")] = sev_counts.get(f.get("severity","Info"),0)+1
    total_sev = sum(sev_counts.values()) or 1
    colors = {"Critical":"#e85c5c","Warning":"#ffb454","Info":"#46f08a"}
    angle = -90.0; cx=cy=60; r=44
    arcs = ""
    for sev_name in ["Critical","Warning","Info"]:
        cnt = sev_counts[sev_name]
        frac = cnt / total_sev
        sweep = frac * 360
        if sweep < 1: continue
        col = colors[sev_name]
        start = angle * math.pi / 180
        end = (angle+sweep) * math.pi / 180
        x1,y1 = cx+r*math.cos(start), cy+r*math.sin(start)
        x2,y2 = cx+r*math.cos(end), cy+r*math.sin(end)
        large = 1 if sweep > 180 else 0
        arcs.append(f'<path d="M {cx} {cy} L {x1:.1f} {y1:.1f} A {r} {r} 0 {large} 1 {x2:.1f} {y2:.1f} Z" fill="{col}" opacity=".85"/>')
        angle += sweep
    donut_svg = f'<svg width="120" height="120"><circle cx="60" cy="60" r="44" fill="none" stroke="var(--ln)" stroke-width="18"/>{"".join(arcs)}<text x="60" y="58" text-anchor="middle" fill="var(--fg)" font-size="20" font-weight="700">{total_sev}</text><text x="60" y="76" text-anchor="middle" fill="var(--dim)" font-size="9">FAULTS</text></svg>'
    legend = "".join(f'<div style="display:flex;align-items:center;gap:.3rem;margin:.15rem 0">'
                     f'<span style="width:9px;height:9px;border-radius:50%;background:{colors[s]};display:inline-block"></span>'
                     f'{s}: {sev_counts[s]}</div>' for s in ["Critical","Warning","Info"])
    fault_donut = f'<div style="display:flex;gap:1rem;align-items:center">{donut_svg}<div>{legend}</div></div>'

    # Recent activity
    act = ""
    for r in reversed(reports[-10:]):
        ts = fmt_ts(r.get("_p",{}).get("generated_at"))
        model = r.get("_p",{}).get("device",{}).get("model","?")
        nf = len(r.get("_fl",[]))
        fc = f'<span class="badge b-crit">{nf}</span>' if nf else '<span class="badge b-ok">0</span>'
        act += f"<tr><td>{fmt_ts(r.get('_p',{}).get('generated_at'))}</td><td>{fc}</td></tr>"

    body = f"""
<p class="tagline">Anonymous diagnostics · alpha · Tailscale-only operator access</p>
<div class="cards">
<div class="card"><b>{total_r}</b><span>Reports</span></div>
<div class="card bluc"><b>{machines}</b><span>Machines</span></div>
<div class="card wrnc"><b>{warns}</b><span>Warnings</span></div>
<div class="card crtc"><b>{crit}</b><span>Critical Faults</span></div>
</div>

<h2>Sensor Temperatures</h2>
<div class="panel">{temp_bars or '<span class="dim">No data</span>'}</div>
<h2>Fan Status</h2>
<div class="panel">{fan_bars or '<span class="dim">No fans</span>'}</div>
<h2>Fault Distribution</h2>
<div class="panel" style="display:flex;justify-content:center">{fault_donut}</div>
<h2>Recent Reports</h2>
<div class="panel"><table><thead><tr><th>Time</th><th>Faults</th></tr></thead><tbody>{act or '<tr><td colspan="2" class="dim">none</td></tr>'}</tbody></table></div>
"""
    return _page("Dashboard", body)


@app.get("/reports", response_class=HTMLResponse)
def reports_list_page():
    reports = load_reports()
    rows = "".join(
        f'<tr><td><a href="/reports/{r["id"]}">#{r["id"]}</a></td>'
        f'<td>{fmt_ts(r.get("_p",{}).get("generated_at"))}</td>'
        f'<td>{str(r.get("machine_id","—"))[:8]}</td>'
        f'<td>{esc(r.get("_p",{}).get("os",{}).get("distro","?"))}</td>'
        f'<td>{esc(r.get("_p",{}).get("device",{}).get("model","?"))}</td>'
        f'<td>{len(r.get("_fl",[]))}</td></tr>'
        for r in reversed(reports)
    )
    body = f"<h2>All Reports ({len(reports)})</h2><div class='panel'><table>" \
           f"<thead><tr><th>ID</th><th>Time</th><th>Machine</th><th>Distro</th><th>Model</th><th>Faults</th></tr></thead>" \
           f"<tbody>{rows}</tbody></table></div>"
    return _page("Reports", body)


@app.get("/reports/{rid}", response_class=HTMLResponse)
def report_detail_page(rid: int):
    raw = db.get_payload(rid)
    if not raw: return HTMLResponse("<h1>Not found</h1>", status_code=404)
    doc = json.loads(raw)
    dev = doc.get("device",{}); os_d = doc.get("os",{})
    sensors_d = doc.get("sensors",{}); bat = doc.get("battery",{})
    faults_l = doc.get("faults",[])
    sys_i = doc.get("system_info",{})

    def sec(title, pairs):
        rows = "".join(f"<tr><td>{k}</td><td>{v}</td></tr>" for k,v in pairs)
        return f"<h2>{title}</h2><table>{rows}</table>"

    body = f"<h2>Report #{rid}</h2>"
    body += "<div class='panel'><table>"
    body += f"<tr><td>Model</td><td>{esc(dev.get('model','?'))}</td></tr>"
    body += f"<tr><td>Machine Type</td><td>{esc(dev.get('machine_type','?'))}</td></tr>"
    body += f"<tr><td>BIOS</td><td>{esc(dev.get('bios_version','?'))}</td></tr>"
    body += f"<tr><td>CPU</td><td>{esc(dev.get('cpu_model','?'))}</td></tr>"
    body += f"<tr><td>GPU</td><td>{esc(dev.get('gpu_model','?'))}</td></tr>"
    body += "</table></div>"
    body += f"<h2>OS</h2><div class='panel'><table>"
    body += f"<tr><td>Distro</td><td>{esc(os_d.get('distro','?'))}</td></tr>"
    body += f"<tr><td>Kernel</td><td>{esc(os_d.get('kernel','?'))}</td></tr></table></div>"
    body += f"<h2>Battery</h2><div class='panel'><table>"
    body += f"<tr><td>Capacity</td><td>{bat.get('capacity_pct','?')}%</td></tr>"
    body += f"<tr><td>Health</td><td>{bat.get('health_pct','?')}</td></tr>"
    body += f"<tr><td>Status</td><td>{bat.get('status','?')}</td></tr></table></div>"
    body += f"<h2>Faults ({len(faults_l)})</h2><div class='panel'>"
    for flt in faults_l:
        body += f"<div class='fe'><strong>{flt['id']}</strong> [{flt['severity']}] — {flt['detail']}</div>"
    if not faults_l: body += "<span class='dim'>None detected</span>"
    body += "</div>"
    body += f"<h2>Payload</h2><pre>{esc(payload_raw)}</pre>"
    return _page(f"Report #{rid}", body)


@app.get("/machines", response_class=HTMLResponse)
def machines_view_page():
    reports = load_reports()
    groups: dict[str,list] = {}
    for r in reports:
        mid = r.get("_p",{}).get("machine_id") or "?"
        groups.setdefault(mid, []).append(r)

    cards = ""
    for mid, grp in sorted(groups.items()):
        last = grp[-1]
        dev = last.get("_p",{}).get("device",{})
        os_d = last.get("_p",{}).get("os",{}).get("distro","?")
        nflts = sum(len(g.get("_fl",[])) for g in grp)
        hc = "#46f08a" if not nflts else ("#ffb454" if nflts < 5 else "#e85c5c")
        cards += f'''<div class="mc" style="border-left:3px solid {hc}">
<h3>{dev.get("model","?")}</h3>
<div class="meta">ID: {mid[:12]}… | Distro: {os_d}<br>
Reports: {len(grp)} | Faults: {nflts}<br>Last: {last.get("ts","?")[:16]}</div></div>'''

    body = f"<h2>Machines ({len(groups)})</h2><div class='grid3'>{cards}</div>"
    return _page("Machines", body)


@app.get("/faults", response_class=HTMLResponse)
def faults_view_page():
    reports = load_reports()
    sections = ""; total = 0
    for r in reversed(reports):
        flts = r.get("_fl",[])
        if not flts: continue
        ts = fmt_ts(r.get("_p",{}).get("generated_at"))
        items = "".join(
            f"<div class='fe'><span class='badge {'b-crit' if f.get('severity')=='Critical' else 'b-warn'}'>"
            f"{f.get('severity','?')}</span> <strong>{f.get('id','')}</strong> — {f.get('detail','')}</div>"
            for f in flts)
        sections += f"<h2>{ts}</h2><div class='panel'>{items}</div>"
        total += len(flts)

    body = f"<h2>Fault History ({total} entries)</h2>" + (sections or "<p>No faults recorded.</p>")
    return _page("Fault Tracker", body)


@app.get("/errors", response_class=HTMLResponse)
def errors_view_page():
    reports = load_reports()
    agg: dict[str,dict] = {}
    for r in reports:
        ld = r.get("_log",{})
        for tgt, cnt in (ld.get("errors_by_target") or {}).items():
            if tgt not in agg: agg[tgt] = {"ERROR":0,"WARN":0}
            agg[tgt]["ERROR"] = agg[tgt].get("ERROR",0) + cnt

    max_e = max((v["ERROR"] for v in agg.values()), default=1) or 1
    rows = "".join(
        f'<tr><td>{tgt}</td><td>{c["ERROR"]}</td><td>{c.get("WARN",0)}</td>'
        f'<td><div style="width:{min(100,c["ERROR"]*100//max(max_e,1))}%;height:8px;'
        f'background:{"var(--crt)" if c["ERROR"]>10 else "var(--wrn)"};border-radius:2px"></div></td></tr>'
        for tgt, c in sorted(agg.items(), key=lambda x:-x[1].get("ERROR",0))
    )
    table = f"<table><thead><tr><th>Module</th><th>Err</th><th>Warn</th><th>Load</th></tr></thead><tbody>{rows}</tbody></table>"
    return _page("Error Attribution", f"<p class='tagline'>{len(agg)} module(s) tracked</p><div class='panel'>{table}</div>")


@app.get("/privacy", response_class=HTMLResponse)
def privacy_view():
    legal = Path(__file__).resolve().parent / "legal"

    def md_html(fp: Path) -> str:
        if not fp.exists(): return ""
        out = []
        for ln in fp.read_text().split("\n"):
            e_ln = esc(ln)
            if ln.startswith("# "): out.append(f"<h2>{e_ln[2:]}</h2>")
            elif ln.startswith("## "): out.append(f"<h3>{e_ln[3:]}</h3>")
            elif ln.startswith("- "): out.append(f"<li>{e_ln[2:]}</li>")
            elif ln.strip(): out.append(f"<p>{e_ln}</p>")
        return "\n".join(out)

    de = md_html(legal / "DATENSCHUTZ-TELEMETRIE.md")
    en = md_html(legal / "PRIVACY-TELEMETRY.md")
    return _page("Privacy & GDPR",
                 f"<div class='panel'><h2>Datenschutzerklärung</h2>{de}</div>"
                 f"<hr style='border-color:var(--ln)'>"
                 f"<div class='panel'><h2>Privacy Statement (English)</h2>{en}</div>")


@app.get("/healthz")
async def healthz_endpoint():
    import sqlite3
    conn = db._conn
    count = 0
    if conn:
        row = conn.execute("SELECT COUNT(*) FROM reports").fetchone()
        count = row[0] if row else 0
    return {"ok": True, "count": count}


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app,
                host=os.environ.get("LEGION_PORTAL_HOST", "127.0.0.1"),
                port=int(os.environ.get("LEGION_PORTAL_PORT", "8788")))
