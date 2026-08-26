package main

import (
	"bytes"
	"crypto/subtle"
	_ "embed"
	"encoding/json"
	"fmt"
	"html"
	"io"
	"log"
	"math"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"database/sql"

	_ "modernc.org/sqlite"
)

//go:embed legal/DATENSCHUTZ-TELEMETRIE.md
var datenschutzMD string

//go:embed legal/PRIVACY-TELEMETRY.md
var privacyMD string

const (
	maxBodyBytes = 256 * 1024
	scanLimit    = 5000
)

type DB struct {
	db   *sql.DB
	lock sync.Mutex
}

func initDB(dbPath string) (*DB, error) {
	if err := os.MkdirAll(filepath.Dir(dbPath), 0755); err != nil && !os.IsExist(err) {
		// ignore if directory is current directory or root
	}
	db, err := sql.Open("sqlite", dbPath+"?_journal_mode=WAL&_busy_timeout=5000&_synchronous=NORMAL")
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1) // SQLite WAL mode serialization

	ddl := `
	CREATE TABLE IF NOT EXISTS reports (
		id INTEGER PRIMARY KEY AUTOINCREMENT,
		ts TEXT NOT NULL,
		received_at TEXT NOT NULL,
		payload TEXT NOT NULL,
		machine_id TEXT,
		distro TEXT,
		model TEXT,
		app_version TEXT,
		schema_version INTEGER
	);
	CREATE INDEX IF NOT EXISTS idx_reports_ts ON reports(ts);
	`
	if _, err := db.Exec(ddl); err != nil {
		return nil, fmt.Errorf("init schema: %w", err)
	}
	return &DB{db: db}, nil
}

func (d *DB) Insert(ts, payload, machineID, distro, model, appVersion string, schemaVersion int) (int64, error) {
	d.lock.Lock()
	defer d.lock.Unlock()

	res, err := d.db.Exec(`
		INSERT INTO reports (ts, received_at, payload, machine_id, distro, model, app_version, schema_version)
		VALUES (?, datetime('now'), ?, ?, ?, ?, ?, ?)`,
		ts, payload, machineID, distro, model, appVersion, schemaVersion,
	)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

func (d *DB) FindRecentByMachine(machineID string, minutes int) (int64, error) {
	if machineID == "" {
		return 0, nil
	}
	d.lock.Lock()
	defer d.lock.Unlock()

	var id int64
	err := d.db.QueryRow(`
		SELECT id FROM reports 
		WHERE machine_id = ? AND received_at > datetime('now', ?) 
		LIMIT 1`,
		machineID, fmt.Sprintf("-%d minutes", minutes),
	).Scan(&id)
	if err == sql.ErrNoRows {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	return id, nil
}

type ReportMeta struct {
	ID         int64
	TS         string
	Distro     string
	Model      string
	AppVersion string
	MachineID  string
}

func (d *DB) Recent(limit int) ([]ReportMeta, error) {
	d.lock.Lock()
	defer d.lock.Unlock()

	rows, err := d.db.Query(`
		SELECT id, ts, COALESCE(distro,''), COALESCE(model,''), COALESCE(app_version,''), COALESCE(machine_id,'')
		FROM reports ORDER BY id DESC LIMIT ?`,
		limit,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var metas []ReportMeta
	for rows.Next() {
		var m ReportMeta
		if err := rows.Scan(&m.ID, &m.TS, &m.Distro, &m.Model, &m.AppVersion, &m.MachineID); err != nil {
			return nil, err
		}
		metas = append(metas, m)
	}
	return metas, nil
}

func (d *DB) GetPayload(id int64) (string, error) {
	d.lock.Lock()
	defer d.lock.Unlock()

	var payload string
	err := d.db.QueryRow("SELECT payload FROM reports WHERE id = ?", id).Scan(&payload)
	if err == sql.ErrNoRows {
		return "", nil
	}
	return payload, err
}

func (d *DB) Count() (int64, error) {
	d.lock.Lock()
	defer d.lock.Unlock()

	var c int64
	err := d.db.QueryRow("SELECT COUNT(*) FROM reports").Scan(&c)
	return c, err
}

func (d *DB) PruneOlderThan(days int) (int64, error) {
	d.lock.Lock()
	defer d.lock.Unlock()

	cutoff := fmt.Sprintf("datetime('now', '-%d days')", days)
	res, err := d.db.Exec(fmt.Sprintf("DELETE FROM reports WHERE ts < %s", cutoff))
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}

type Server struct {
	db         *DB
	teleKey    string
	rateLimit  int
	rateMap    map[string][]time.Time
	rateLock   sync.Mutex
	trustedHop bool
}

func NewServer(db *DB, key string, rateLimit int) *Server {
	return &Server{
		db:        db,
		teleKey:   key,
		rateLimit: rateLimit,
		rateMap:   make(map[string][]time.Time),
	}
}

func (s *Server) checkRate(ip string) bool {
	s.rateLock.Lock()
	defer s.rateLock.Unlock()

	now := time.Now()
	cutoff := now.Add(-1 * time.Minute)

	var active []time.Time
	for _, t := range s.rateMap[ip] {
		if t.After(cutoff) {
			active = append(active, t)
		}
	}

	if len(active) >= s.rateLimit {
		s.rateMap[ip] = active
		return false
	}

	active = append(active, now)
	s.rateMap[ip] = active

	// Cleanup old keys periodically
	if len(s.rateMap) > 1000 {
		for k, v := range s.rateMap {
			if len(v) == 0 || v[len(v)-1].Before(cutoff) {
				delete(s.rateMap, k)
			}
		}
	}
	return true
}

func (s *Server) clientIP(r *http.Request) string {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		host = r.RemoteAddr
	}
	if host == "127.0.0.1" || host == "::1" {
		xff := r.Header.Get("X-Forwarded-For")
		if xff != "" {
			parts := strings.Split(xff, ",")
			if len(parts) > 0 {
				return strings.TrimSpace(parts[0])
			}
		}
		cfIP := r.Header.Get("CF-Connecting-IP")
		if cfIP != "" {
			return strings.TrimSpace(cfIP)
		}
	}
	return host
}

func (s *Server) handleIngest(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, `{"detail":"method not allowed"}`, http.StatusMethodNotAllowed)
		return
	}

	// Auth check
	gotKey := r.Header.Get("X-Legion-Telemetry-Key")
	if subtle.ConstantTimeCompare([]byte(gotKey), []byte(s.teleKey)) != 1 {
		http.Error(w, `{"detail":"unauthorized"}`, http.StatusUnauthorized)
		return
	}

	ip := s.clientIP(r)
	if !s.checkRate(ip) {
		http.Error(w, `{"detail":"slow down"}`, http.StatusTooManyRequests)
		return
	}

	body, err := io.ReadAll(io.LimitReader(r.Body, maxBodyBytes+1))
	if err != nil {
		http.Error(w, `{"detail":"failed reading body"}`, http.StatusBadRequest)
		return
	}
	if len(body) > maxBodyBytes {
		http.Error(w, `{"detail":"payload too large"}`, http.StatusRequestEntityTooLarge)
		return
	}

	var doc map[string]interface{}
	dec := json.NewDecoder(bytes.NewReader(body))
	dec.UseNumber()
	if err := dec.Decode(&doc); err != nil {
		http.Error(w, `{"detail":"invalid JSON"}`, http.StatusBadRequest)
		return
	}

	svNum, ok := doc["schema_version"].(json.Number)
	if !ok {
		http.Error(w, `{"detail":"unsupported report"}`, http.StatusBadRequest)
		return
	}
	sv, err := svNum.Int64()
	if err != nil || sv != 1 {
		http.Error(w, `{"detail":"unsupported report"}`, http.StatusBadRequest)
		return
	}

	ts := time.Now().UTC().Format(time.RFC3339)
	var distro, model, appVer, machineID string

	if osInfo, ok := doc["os"].(map[string]interface{}); ok {
		if d, ok := osInfo["distro"].(string); ok {
			distro = truncate(d, 256)
		}
	}
	if dev, ok := doc["device"].(map[string]interface{}); ok {
		if m, ok := dev["model"].(string); ok {
			model = truncate(m, 256)
		}
	}
	if av, ok := doc["app_version"].(string); ok {
		appVer = truncate(av, 256)
	}
	if mid, ok := doc["machine_id"].(string); ok {
		machineID = truncate(mid, 256)
	}

	if existing, _ := s.db.FindRecentByMachine(machineID, 5); existing > 0 {
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]interface{}{
			"ok":        true,
			"duplicate": true,
			"id":        existing,
		})
		return
	}

	id, err := s.db.Insert(ts, string(body), machineID, distro, model, appVer, int(sv))
	if err != nil {
		http.Error(w, `{"detail":"internal error"}`, http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{
		"ok": true,
		"id": id,
	})
}

func truncate(s string, maxChars int) string {
	r := []rune(s)
	if len(r) > maxChars {
		return string(r[:maxChars])
	}
	return s
}

func (s *Server) handleHealth(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]bool{"ok": true})
}

func (s *Server) handleHealthz(w http.ResponseWriter, r *http.Request) {
	cnt, _ := s.db.Count()
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{
		"ok":    true,
		"count": cnt,
	})
}

const portalCSS = `
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
`

func renderPage(title, body, active string) string {
	navItems := []struct{ Label, Href, Act string }{
		{"Dashboard", "/", "dashboard"},
		{"Reports", "/reports", "reports"},
		{"Machines", "/machines", "machines"},
		{"Faults", "/faults", "faults"},
		{"Errors", "/errors", "errors"},
		{"Privacy", "/privacy", "privacy"},
	}

	var nav strings.Builder
	for _, n := range navItems {
		cls := ""
		if n.Act == active {
			cls = ` class="on"`
		}
		nav.WriteString(fmt.Sprintf(`<a href="%s"%s>%s</a>`, n.Href, cls, n.Label))
	}

	return fmt.Sprintf(`<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width"><meta name="robots" content="noindex,nofollow">
<title>%s — Legion Telemetry</title><style>%s</style></head>
<body><nav><span class="logo">&gt;_ legion</span>%s</nav>
<h1><span class="prompt" style="color:var(--ac)">&gt;</span> %s</h1>%s
<div class="foot">Legion Control telemetry · alpha · Tailscale / Cloudflare Access operator portal</div></body></html>`,
		html.EscapeString(title), portalCSS, nav.String(), html.EscapeString(title), body,
	)
}

type ParsedReport struct {
	ID        int64
	TS        string
	TimeObj   time.Time
	Distro    string
	Model     string
	AppVer    string
	MachineID string
	Payload   map[string]interface{}
	Sensors   map[string]interface{}
	Battery   map[string]interface{}
	Fans      []map[string]interface{}
	Faults    []map[string]interface{}
}

func parseTime(v interface{}) time.Time {
	if v == nil {
		return time.Time{}
	}
	switch val := v.(type) {
	case float64:
		return time.Unix(int64(val), 0).UTC()
	case string:
		val = strings.TrimSpace(val)
		if strings.HasSuffix(val, "Z") || strings.HasSuffix(val, "z") {
			val = val[:len(val)-1] + "+00:00"
		}
		if t, err := time.Parse(time.RFC3339, val); err == nil {
			return t.UTC()
		}
		if t, err := time.Parse("2006-01-02T15:04:05", val); err == nil {
			return t.UTC()
		}
	}
	return time.Time{}
}

func (s *Server) loadReports() []ParsedReport {
	metas, err := s.db.Recent(scanLimit)
	if err != nil {
		return nil
	}

	var out []ParsedReport
	for _, m := range metas {
		pr := ParsedReport{
			ID:        m.ID,
			TS:        m.TS,
			Distro:    m.Distro,
			Model:     m.Model,
			AppVer:    m.AppVersion,
			MachineID: m.MachineID,
		}

		raw, err := s.db.GetPayload(m.ID)
		if err == nil && raw != "" {
			var p map[string]interface{}
			if err := json.Unmarshal([]byte(raw), &p); err == nil {
				pr.Payload = p
				if sens, ok := p["sensors"].(map[string]interface{}); ok {
					pr.Sensors = sens
				}
				if bat, ok := p["battery"].(map[string]interface{}); ok {
					pr.Battery = bat
				}
				if fans, ok := p["fans"].([]interface{}); ok {
					for _, f := range fans {
						if fm, ok := f.(map[string]interface{}); ok {
							pr.Fans = append(pr.Fans, fm)
						}
					}
				}
				if faults, ok := p["faults"].([]interface{}); ok {
					for _, f := range faults {
						if fm, ok := f.(map[string]interface{}); ok {
							pr.Faults = append(pr.Faults, fm)
						}
					}
				}
				pr.TimeObj = parseTime(p["generated_at"])
			}
		}
		if pr.TimeObj.IsZero() {
			pr.TimeObj = parseTime(m.TS)
		}
		out = append(out, pr)
	}

	sort.Slice(out, func(i, j int) bool {
		return out[i].TimeObj.Before(out[j].TimeObj)
	})
	return out
}

func (s *Server) handleDashboard(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		if strings.HasPrefix(r.URL.Path, "/reports/") {
			s.handleReportDetail(w, r)
			return
		}
		http.NotFound(w, r)
		return
	}

	reps := s.loadReports()
	n := len(reps)
	machMap := make(map[string]bool)
	now := time.Now().UTC()
	last24 := 0
	critCount := 0
	warnCount := 0

	for _, rep := range reps {
		mid := rep.MachineID
		if mid == "" && rep.Payload != nil {
			if m, ok := rep.Payload["machine_id"].(string); ok {
				mid = m
			}
		}
		if mid != "" {
			machMap[mid] = true
		}
		if rep.TimeObj.After(now.Add(-24 * time.Hour)) {
			last24++
		}
		for _, f := range rep.Faults {
			sev, _ := f["severity"].(string)
			if sev == "Critical" {
				critCount++
			} else if sev == "Warning" {
				warnCount++
			}
		}
	}

	var latest ParsedReport
	if n > 0 {
		latest = reps[n-1]
	}

	// Temp bars
	var tempBars strings.Builder
	if latest.Sensors != nil {
		keys := []struct{ Label, Key string }{
			{"CPU", "cpu_temp"},
			{"CCD1", "cpu_temp_1"},
			{"CCD2", "cpu_temp_2"},
			{"dGPU", "dgpu_temp"},
			{"iGPU", "igpu_edge"},
			{"EC CPU", "ec_cpu"},
		}
		for _, k := range keys {
			if v, ok := latest.Sensors[k.Key]; ok {
				if fv, ok := toFloat(v); ok && fv >= 0 {
					pct := math.Min(100, math.Max(0, (fv-20)/(100-20)*100))
					col := "var(--ac)"
					if fv >= 90 {
						col = "var(--crt)"
					} else if fv >= 75 {
						col = "var(--wrn)"
					}
					tempBars.WriteString(fmt.Sprintf(
						`<div class="hbar"><span class="hl">%s</span><div class="ht"><div class="hf" style="width:%.0f%%;background:%s"></div></div><span class="hv">%.1f°C</span></div>`+"\n",
						html.EscapeString(k.Label), pct, col, fv,
					))
				}
			}
		}
		if ssds, ok := latest.Sensors["ssd_composite"].([]interface{}); ok {
			for i, sv := range ssds {
				if fv, ok := toFloat(sv); ok {
					col := "var(--ac)"
					if fv >= 80 {
						col = "var(--crt)"
					} else if fv >= 60 {
						col = "var(--wrn)"
					}
					pct := math.Min(100, math.Max(0, fv))
					tempBars.WriteString(fmt.Sprintf(
						`<div class="hbar"><span class="hl">NVMe %d</span><div class="ht"><div class="hf" style="width:%.0f%%;background:%s"></div></div><span class="hv">%.1f°C</span></div>`+"\n",
						i, pct, col, fv,
					))
				}
			}
		}
	}

	// Fan bars
	var fanBars strings.Builder
	for _, f := range latest.Fans {
		rpm, _ := toFloat(f["rpm"])
		mx, _ := toFloat(f["max_rpm"])
		if mx <= 0 {
			mx = 5000
		}
		pct := math.Min(100, math.Max(0, rpm/mx*100))
		col := "var(--ac)"
		if pct > 80 {
			col = "var(--crt)"
		} else if pct > 50 {
			col = "var(--wrn)"
		}
		fid := fmt.Sprintf("%v", f["id"])
		fanBars.WriteString(fmt.Sprintf(
			`<div class="hbar"><span class="hl">Fan %s</span><div class="ht"><div class="hf" style="width:%.0f%%;background:%s"></div></div><span class="hv">%d RPM</span></div>`+"\n",
			html.EscapeString(fid), pct, col, int(rpm),
		))
	}

	// Fault donut
	sevCounts := map[string]int{"Critical": 0, "Warning": 0, "Info": 0}
	for _, r := range reps {
		for _, flt := range r.Faults {
			sev, _ := flt["severity"].(string)
			if _, ok := sevCounts[sev]; ok {
				sevCounts[sev]++
			}
		}
	}
	totalFaults := sevCounts["Critical"] + sevCounts["Warning"] + sevCounts["Info"]
	colors := map[string]string{"Critical": "#e85c5c", "Warning": "#ffb454", "Info": "#46f08a"}
	cx, cy := 60.0, 60.0
	rRad := 44.0
	var arcs strings.Builder
	angle := -90.0
	denom := float64(totalFaults)
	if denom == 0 {
		denom = 1
	}

	for _, sevName := range []string{"Critical", "Warning", "Info"} {
		cnt := sevCounts[sevName]
		if cnt <= 0 {
			continue
		}
		frac := float64(cnt) / denom
		sweep := frac * 360.0
		if sweep < 0.5 {
			continue
		}
		col := colors[sevName]
		start := angle * math.Pi / 180.0
		end := (angle + sweep) * math.Pi / 180.0
		x1, y1 := cx+rRad*math.Cos(start), cy+rRad*math.Sin(start)
		x2, y2 := cx+rRad*math.Cos(end), cy+rRad*math.Sin(end)
		large := 0
		if sweep > 180 {
			large = 1
		}
		arcs.WriteString(fmt.Sprintf(
			`<path d="M %.1f %.1f L %.1f %.1f A %.1f %.1f 0 %d 1 %.1f %.1f Z" fill="%s" opacity=".8"/>`,
			cx, cy, x1, y1, rRad, rRad, large, x2, y2, col,
		))
		angle += sweep
	}

	donutSVG := fmt.Sprintf(
		`<svg width="120" height="120">%s<circle cx="%.0f" cy="%.0f" r="%.0f" fill="var(--bg)"/><text x="%.0f" y="%.0f" text-anchor="middle" fill="var(--fg)" font-size="18" font-weight="700">%d</text><text x="%.0f" y="%.0f" text-anchor="middle" fill="var(--dim)" font-size="8">FAULTS</text></svg>`,
		arcs.String(), cx, cy, rRad-14, cx, cy-3, totalFaults, cx, cy+14,
	)

	var legend strings.Builder
	for _, s2 := range []string{"Critical", "Warning", "Info"} {
		legend.WriteString(fmt.Sprintf(
			`<div style="display:flex;align-items:center;gap:.3rem;margin:.1rem 0"><span style="width:9px;height:9px;border-radius:50%;background:%s;display:inline-block"></span>%s: %d</div>`,
			colors[s2], s2, sevCounts[s2],
		))
	}
	faultDonut := fmt.Sprintf(`<div style="display:flex;gap:.8rem;align-items:center">%s<div>%s</div></div>`, donutSVG, legend.String())

	// Fault details list
	var faultDetails strings.Builder
	fCount := 0
	for i := len(reps) - 1; i >= 0 && fCount < 5; i-- {
		for _, flt := range reps[i].Faults {
			sev, _ := flt["severity"].(string)
			fid, _ := flt["id"].(string)
			detail, _ := flt["detail"].(string)
			badgeCls := "b-info"
			if sev == "Critical" {
				badgeCls = "b-crit"
			} else if sev == "Warning" {
				badgeCls = "b-warn"
			}
			faultDetails.WriteString(fmt.Sprintf(
				`<div class="fe"><span class="badge %s">%s</span> <strong>%s</strong> — %s</div>`,
				badgeCls, html.EscapeString(sev), html.EscapeString(fid), html.EscapeString(detail),
			))
			fCount++
			if fCount >= 5 {
				break
			}
		}
	}
	faultDetailsHTML := faultDetails.String()
	if faultDetailsHTML == "" {
		faultDetailsHTML = `<span class="dim">Clean — no faults</span>`
	}

	// Recent activity rows
	var actRows strings.Builder
	actCount := 0
	for i := len(reps) - 1; i >= 0 && actCount < 10; i-- {
		r := reps[i]
		tsStr := r.TimeObj.Format("01-02 15:04")
		model := r.Model
		if model == "" {
			model = "?"
		}
		nf := len(r.Faults)
		badge := "b-ok"
		if nf > 0 {
			badge = "b-crit"
		}
		actRows.WriteString(fmt.Sprintf(
			`<tr><td>%s</td><td>%s</td><td><span class="badge %s">%d</span></td></tr>`,
			tsStr, html.EscapeString(model), badge, nf,
		))
		actCount++
	}
	if actRows.Len() == 0 {
		actRows.WriteString(`<tr><td colspan="3" class="dim">none</td></tr>`)
	}

	cards := fmt.Sprintf(
		`<div class="card"><b>%d</b><span>Total Reports</span></div><div class="card bluc"><b>+%d</b><span>Last 24 Hours</span></div><div class="card"><b>%d</b><span>Machines</span></div><div class="card wrnc"><b>%d</b><span>Warnings</span></div><div class="card crtc"><b>%d</b><span>Critical Faults</span></div>`,
		n, last24, len(machMap), warnCount, critCount,
	)

	tbHTML := tempBars.String()
	if tbHTML == "" {
		tbHTML = `<span class="dim">No data yet</span>`
	}
	fbHTML := fanBars.String()
	if fbHTML == "" {
		fbHTML = `<span class="dim">No fans detected</span>`
	}

	body := fmt.Sprintf(`
<p class="tagline">Anonymous diagnostics · alpha · Tailscale / Cloudflare Access operator access</p>
<div class="cards">%s</div>
<div class="grid2"><div>
<h2>Sensor Temperatures</h2>
<div class="panel">%s</div>
<h2>Fan Status</h2>
<div class="panel">%s</div>
</div><div>
<h2>Fault Distribution</h2>
<div class="panel" style="display:flex;justify-content:center">%s</div>
<h2>Fault Details</h2>
<div class="panel">%s</div>
</div></div>
<h2>Recent Activity</h2>
<div class="panel"><table><thead><tr><th>Time</th><th>Model</th><th>Faults</th></tr></thead>
<tbody>%s</tbody></table></div>
`, cards, tbHTML, fbHTML, faultDonut, faultDetailsHTML, actRows.String())

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write([]byte(renderPage("Dashboard", body, "dashboard")))
}

func (s *Server) handleReports(w http.ResponseWriter, r *http.Request) {
	reps := s.loadReports()
	var rows strings.Builder
	for i := len(reps) - 1; i >= 0; i-- {
		rep := reps[i]
		tsStr := rep.TimeObj.Format("01-02 15:04")
		mid := rep.MachineID
		if len(mid) > 8 {
			mid = mid[:8]
		}
		if mid == "" {
			mid = "—"
		}
		distro := rep.Distro
		if distro == "" {
			distro = "?"
		}
		model := rep.Model
		if model == "" {
			model = "?"
		}
		nf := len(rep.Faults)
		fc := `<span class="badge b-ok">0</span>`
		if nf > 0 {
			fc = fmt.Sprintf(`<span class="badge b-crit">%d</span>`, nf)
		}
		rows.WriteString(fmt.Sprintf(
			`<tr><td><a href="/reports/%d">#%d</a></td><td>%s</td><td>%s</td><td>%s</td><td>%s</td><td>%s</td></tr>`,
			rep.ID, rep.ID, tsStr, html.EscapeString(mid), html.EscapeString(distro), html.EscapeString(model), fc,
		))
	}
	bodyRows := rows.String()
	if bodyRows == "" {
		bodyRows = `<tr><td colspan="6" class="dim">empty</td></tr>`
	}
	body := fmt.Sprintf(`<h2>All Reports (%d)</h2><div class="panel"><table style="margin-top:0"><thead><tr><th>ID</th><th>Time</th><th>Machine ID</th><th>Distro</th><th>Model</th><th>Faults</th></tr></thead><tbody>%s</tbody></table></div>`, len(reps), bodyRows)

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write([]byte(renderPage("Reports", body, "reports")))
}

func (s *Server) handleReportDetail(w http.ResponseWriter, r *http.Request) {
	parts := strings.Split(strings.Trim(r.URL.Path, "/"), "/")
	if len(parts) != 2 {
		http.NotFound(w, r)
		return
	}
	id, err := strconv.ParseInt(parts[1], 10, 64)
	if err != nil {
		http.NotFound(w, r)
		return
	}

	payload, err := s.db.GetPayload(id)
	if err != nil || payload == "" {
		http.NotFound(w, r)
		return
	}

	var formatted bytes.Buffer
	if err := json.Indent(&formatted, []byte(payload), "", "  "); err != nil {
		formatted.WriteString(payload)
	}

	body := fmt.Sprintf(`
<h2>Report #%d</h2>
<div class="panel"><p><a href="/reports">&larr; Back to all reports</a></p><br>
<pre>%s</pre></div>`, id, html.EscapeString(formatted.String()))

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write([]byte(renderPage(fmt.Sprintf("Report #%d", id), body, "reports")))
}

func (s *Server) handleMachines(w http.ResponseWriter, r *http.Request) {
	reps := s.loadReports()
	groups := make(map[string][]ParsedReport)

	for _, rep := range reps {
		mid := rep.MachineID
		if mid == "" && rep.Model != "" {
			mid = rep.Model
		}
		if mid == "" {
			mid = "unknown"
		}
		groups[mid] = append(groups[mid], rep)
	}

	var keys []string
	for k := range groups {
		keys = append(keys, k)
	}
	sort.Strings(keys)

	var cards strings.Builder
	for _, mid := range keys {
		grp := groups[mid]
		last := grp[len(grp)-1]
		model := last.Model
		if model == "" {
			model = "?"
		}
		distro := last.Distro
		if distro == "" {
			distro = "?"
		}
		nflts := 0
		for _, g := range grp {
			nflts += len(g.Faults)
		}
		hc := "#46f08a"
		if nflts > 0 {
			if nflts < 5 {
				hc = "#ffb454"
			} else {
				hc = "#e85c5c"
			}
		}
		dispID := mid
		if len(dispID) > 12 {
			dispID = dispID[:12] + "…"
		}
		lastTS := last.TS
		if len(lastTS) > 16 {
			lastTS = lastTS[:16]
		}
		cards.WriteString(fmt.Sprintf(
			`<div class="mc" style="border-left:3px solid %s"><h3>%s</h3><div class="meta">ID: %s<br>Distro: %s<br>Reports: %d | Faults: %d<br>Last: %s</div></div>`,
			hc, html.EscapeString(model), html.EscapeString(dispID), html.EscapeString(distro), len(grp), nflts, html.EscapeString(lastTS),
		))
	}
	cardHTML := cards.String()
	if cardHTML == "" {
		cardHTML = `<p class="dim">none</p>`
	}
	body := fmt.Sprintf(`<h2>Machines (%d)</h2><div class="grid3">%s</div>`, len(groups), cardHTML)

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write([]byte(renderPage("Machines", body, "machines")))
}

func (s *Server) handleFaults(w http.ResponseWriter, r *http.Request) {
	reps := s.loadReports()
	var sections strings.Builder
	total := 0

	for i := len(reps) - 1; i >= 0; i-- {
		rep := reps[i]
		if len(rep.Faults) == 0 {
			continue
		}
		tsStr := rep.TimeObj.Format("01-02 15:04")
		var items strings.Builder
		for _, f := range rep.Faults {
			sev, _ := f["severity"].(string)
			fid, _ := f["id"].(string)
			detail, _ := f["detail"].(string)
			badgeCls := "b-info"
			if sev == "Critical" {
				badgeCls = "b-crit"
			} else if sev == "Warning" {
				badgeCls = "b-warn"
			}
			items.WriteString(fmt.Sprintf(
				`<div class="fe"><span class="badge %s">%s</span> <strong>%s</strong> — %s</div>`,
				badgeCls, html.EscapeString(sev), html.EscapeString(fid), html.EscapeString(detail),
			))
		}
		sections.WriteString(fmt.Sprintf(`<h2>%s</h2><div class="panel">%s</div>`, tsStr, items.String()))
		total += len(rep.Faults)
	}

	secHTML := sections.String()
	if secHTML == "" {
		secHTML = `<p class="dim">Clean — no faults recorded.</p>`
	}
	body := fmt.Sprintf(`<h2>Fault History (%d entries)</h2>%s`, total, secHTML)

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write([]byte(renderPage("Fault Tracker", body, "faults")))
}

func (s *Server) handleErrors(w http.ResponseWriter, r *http.Request) {
	reps := s.loadReports()
	type countPair struct{ err, warn int }
	agg := make(map[string]*countPair)

	for _, rep := range reps {
		if rep.Payload == nil {
			continue
		}
		ld, ok := rep.Payload["log_digest"].(map[string]interface{})
		if !ok {
			continue
		}
		if errMap, ok := ld["errors_by_target"].(map[string]interface{}); ok {
			for tgt, c := range errMap {
				if ic, ok := toFloat(c); ok {
					if _, ok := agg[tgt]; !ok {
						agg[tgt] = &countPair{}
					}
					agg[tgt].err += int(ic)
				}
			}
		}
		if warnMap, ok := ld["warnings_by_target"].(map[string]interface{}); ok {
			for tgt, c := range warnMap {
				if ic, ok := toFloat(c); ok {
					if _, ok := agg[tgt]; !ok {
						agg[tgt] = &countPair{}
					}
					agg[tgt].warn += int(ic)
				}
			}
		}
	}

	var tgts []string
	maxE := 1
	for tgt, c := range agg {
		tgts = append(tgts, tgt)
		if c.err > maxE {
			maxE = c.err
		}
	}

	sort.Slice(tgts, func(i, j int) bool {
		return agg[tgts[i]].err > agg[tgts[j]].err
	})

	var rows strings.Builder
	for _, tgt := range tgts {
		c := agg[tgt]
		pct := int(float64(c.err) * 100.0 / float64(maxE))
		barColor := "var(--wrn)"
		if c.err > 10 {
			barColor = "var(--crt)"
		}
		rows.WriteString(fmt.Sprintf(
			`<tr><td>%s</td><td>%d</td><td>%d</td><td><div style="width:%d%%;height:8px;background:%s;border-radius:2px"></div></td></tr>`,
			html.EscapeString(tgt), c.err, c.warn, pct, barColor,
		))
	}

	rowHTML := rows.String()
	if rowHTML == "" {
		rowHTML = `<tr><td colspan="4" class="dim">clean — no errors tracked</td></tr>`
	}
	table := fmt.Sprintf(`<table><thead><tr><th>Module</th><th>Errors</th><th>Warnings</th><th>Load</th></tr></thead><tbody>%s</tbody></table>`, rowHTML)
	body := fmt.Sprintf(`<p class="tagline">%d module(s) tracked</p><div class="panel">%s</div>`, len(agg), table)

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write([]byte(renderPage("Error Attribution", body, "errors")))
}

func mdToHTML(text string) string {
	if text == "" {
		return `<p class="dim">Not available.</p>`
	}
	var out strings.Builder
	lines := strings.Split(text, "\n")
	for _, ln := range lines {
		e := html.EscapeString(ln)
		if strings.HasPrefix(ln, "# ") {
			out.WriteString(fmt.Sprintf("<h2>%s</h2>\n", e[2:]))
		} else if strings.HasPrefix(ln, "## ") {
			out.WriteString(fmt.Sprintf("<h3>%s</h3>\n", e[3:]))
		} else if strings.HasPrefix(ln, "- ") {
			out.WriteString(fmt.Sprintf("<li>%s</li>\n", e[2:]))
		} else if strings.TrimSpace(ln) != "" {
			out.WriteString(fmt.Sprintf("<p>%s</p>\n", e))
		}
	}
	return out.String()
}

func (s *Server) handlePrivacy(w http.ResponseWriter, r *http.Request) {
	deHTML := mdToHTML(datenschutzMD)
	enHTML := mdToHTML(privacyMD)
	body := fmt.Sprintf(`<div class="panel"><h2>Datenschutzerklärung</h2>%s</div><div class="panel"><h2>Privacy Statement</h2>%s</div>`, deHTML, enHTML)

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write([]byte(renderPage("Privacy & GDPR", body, "privacy")))
}

func toFloat(v interface{}) (float64, bool) {
	if v == nil {
		return 0, false
	}
	switch val := v.(type) {
	case float64:
		return val, true
	case float32:
		return float64(val), true
	case int:
		return float64(val), true
	case int64:
		return float64(val), true
	case json.Number:
		f, err := val.Float64()
		return f, err == nil
	}
	return 0, false
}

func main() {
	dbPath := os.Getenv("LEGION_TELEMETRY_DB")
	if dbPath == "" {
		dbPath = "diagnostics.db"
	}
	teleKey := os.Getenv("LEGION_TELEMETRY_KEY")
	if teleKey == "" {
		// Default dummy key if unset
		teleKey = "legion-alpha-secret-key"
	}

	db, err := initDB(dbPath)
	if err != nil {
		log.Fatalf("failed to init db: %v", err)
	}

	rateLimit := 30
	if rStr := os.Getenv("LEGION_TELEMETRY_RATE_PER_MIN"); rStr != "" {
		if r, err := strconv.Atoi(rStr); err == nil && r > 0 {
			rateLimit = r
		}
	}

	retentionDays := 90
	if dStr := os.Getenv("LEGION_TELEMETRY_RETENTION_DAYS"); dStr != "" {
		if d, err := strconv.Atoi(dStr); err == nil && d > 0 {
			retentionDays = d
		}
	}

	// Retention pruner goroutine
	go func() {
		for {
			pruned, err := db.PruneOlderThan(retentionDays)
			if err != nil {
				log.Printf("[legion-telemetry] retention prune error: %v", err)
			} else if pruned > 0 {
				log.Printf("[legion-telemetry] retention: pruned %d report(s)", pruned)
			}
			time.Sleep(1 * time.Hour)
		}
	}()

	srv := NewServer(db, teleKey, rateLimit)

	ingestMux := http.NewServeMux()
	ingestMux.HandleFunc("/v1/diagnostics", srv.handleIngest)
	ingestMux.HandleFunc("/health", srv.handleHealth)

	portalMux := http.NewServeMux()
	portalMux.HandleFunc("/", srv.handleDashboard)
	portalMux.HandleFunc("/reports", srv.handleReports)
	portalMux.HandleFunc("/machines", srv.handleMachines)
	portalMux.HandleFunc("/faults", srv.handleFaults)
	portalMux.HandleFunc("/errors", srv.handleErrors)
	portalMux.HandleFunc("/privacy", srv.handlePrivacy)
	portalMux.HandleFunc("/healthz", srv.handleHealthz)

	ingestPort := os.Getenv("LEGION_INGEST_PORT")
	if ingestPort == "" {
		ingestPort = "8791"
	}
	portalPort := os.Getenv("LEGION_PORTAL_PORT")
	if portalPort == "" {
		portalPort = "8788"
	}

	mode := os.Getenv("LEGION_SERVER_MODE") // "all", "ingest", "portal"
	if mode == "" {
		mode = "all"
	}

	if mode == "all" || mode == "ingest" {
		go func() {
			addr := ":" + ingestPort
			log.Printf("[legion-telemetry] starting ingest on %s", addr)
			if err := http.ListenAndServe(addr, ingestMux); err != nil {
				log.Fatalf("ingest server error: %v", err)
			}
		}()
	}

	if mode == "all" || mode == "portal" {
		addr := ":" + portalPort
		log.Printf("[legion-telemetry] starting portal on %s", addr)
		if err := http.ListenAndServe(addr, portalMux); err != nil {
			log.Fatalf("portal server error: %v", err)
		}
	} else {
		select {}
	}
}
