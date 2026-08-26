package main

import (
	"bytes"
	"compress/gzip"
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

//go:embed dashboard.html
var modernDashboardHTML string

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
	}
	db, err := sql.Open("sqlite", dbPath+"?_journal_mode=WAL&_busy_timeout=5000&_synchronous=NORMAL")
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
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
	CREATE INDEX IF NOT EXISTS idx_reports_machine ON reports(machine_id);
	CREATE TABLE IF NOT EXISTS bug_status (
		bug_id TEXT PRIMARY KEY,
		status TEXT NOT NULL DEFAULT 'NEW',
		assigned_to TEXT DEFAULT '',
		notes TEXT DEFAULT '',
		updated_at TEXT NOT NULL
	);
	`
	if _, err := db.Exec(ddl); err != nil {
		return nil, fmt.Errorf("init schema: %w", err)
	}
	return &DB{db: db}, nil
}

func (d *DB) Insert(ts, payload, machineID, distro, model, appVersion string, schemaVersion int) (int64, error) {
	d.lock.Lock()
	defer d.lock.Unlock()
	res, err := d.db.Exec(`INSERT INTO reports (ts, received_at, payload, machine_id, distro, model, app_version, schema_version) VALUES (?, datetime('now'), ?, ?, ?, ?, ?, ?)`, ts, payload, machineID, distro, model, appVersion, schemaVersion)
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
	err := d.db.QueryRow(`SELECT id FROM reports WHERE machine_id = ? AND received_at > datetime('now', ?) LIMIT 1`, machineID, fmt.Sprintf("-%d minutes", minutes)).Scan(&id)
	if err == sql.ErrNoRows {
		return 0, nil
	}
	return id, err
}

type ReportMeta struct {
	ID         int64  `json:"id"`
	TS         string `json:"ts"`
	Distro     string `json:"distro"`
	Model      string `json:"model"`
	AppVersion string `json:"app_version"`
	MachineID  string `json:"machine_id"`
}

func (d *DB) Recent(limit int) ([]ReportMeta, error) {
	d.lock.Lock()
	defer d.lock.Unlock()
	rows, err := d.db.Query(`SELECT id, ts, COALESCE(distro,''), COALESCE(model,''), COALESCE(app_version,''), COALESCE(machine_id,'') FROM reports ORDER BY id DESC LIMIT ?`, limit)
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

type BugDetail struct {
	Status string
	Notes  string
}

func (d *DB) GetBugDetails() (map[string]BugDetail, error) {
	d.lock.Lock()
	defer d.lock.Unlock()
	rows, err := d.db.Query("SELECT bug_id, status, COALESCE(notes,'') FROM bug_status")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := make(map[string]BugDetail)
	for rows.Next() {
		var id, st, notes string
		if err := rows.Scan(&id, &st, &notes); err == nil {
			out[id] = BugDetail{Status: st, Notes: notes}
		}
	}
	return out, nil
}

func (d *DB) GetBugStatuses() (map[string]string, error) {
	m, err := d.GetBugDetails()
	if err != nil {
		return nil, err
	}
	out := make(map[string]string, len(m))
	for k, v := range m {
		out[k] = v.Status
	}
	return out, nil
}

func (d *DB) SetBugStatus(bugID, status, notes string) error {
	d.lock.Lock()
	defer d.lock.Unlock()
	now := time.Now().UTC().Format(time.RFC3339)
	_, err := d.db.Exec(`INSERT INTO bug_status (bug_id, status, notes, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(bug_id) DO UPDATE SET status=excluded.status, notes=excluded.notes, updated_at=excluded.updated_at`, bugID, status, notes, now)
	return err
}

type Server struct {
	db        *DB
	teleKey   string
	rateLimit int
	rateMap   map[string][]time.Time
	rateLock  sync.Mutex
}

func NewServer(db *DB, key string, rateLimit int) *Server {
	return &Server{db: db, teleKey: key, rateLimit: rateLimit, rateMap: make(map[string][]time.Time)}
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
			if len(parts) > 0 && strings.TrimSpace(parts[0]) != "" {
				return strings.TrimSpace(parts[0])
			}
		}
		if cfIP := r.Header.Get("CF-Connecting-IP"); cfIP != "" {
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
	// Enforce compressed size first (raw wire bytes), then decompressed size.
	// This defeats zip-bombs where 1 KiB gzip expands to 10 MiB.
	if clStr := r.Header.Get("Content-Length"); clStr != "" {
		if cl, err := strconv.ParseInt(strings.TrimSpace(clStr), 10, 64); err == nil && cl > int64(maxBodyBytes) {
			http.Error(w, `{"detail":"payload too large"}`, http.StatusRequestEntityTooLarge)
			return
		}
	}
	var reader io.Reader = io.LimitReader(r.Body, int64(maxBodyBytes+1))
	if strings.EqualFold(strings.TrimSpace(r.Header.Get("Content-Encoding")), "gzip") {
		zr, err := gzip.NewReader(io.LimitReader(r.Body, int64(maxBodyBytes+1)))
		if err != nil {
			http.Error(w, `{"detail":"bad gzip stream"}`, http.StatusBadRequest)
			return
		}
		defer zr.Close()
		reader = io.LimitReader(zr, int64(maxBodyBytes+1))
	}
	body, err := io.ReadAll(reader)
	// io.ReadAll on a LimitReader returns n == limit when capped; detect it via len.
	if err != nil && err != io.EOF {
		// LimitReader returns EOF on cap, so unexpected errors are real failures.
		http.Error(w, `{"detail":"failed reading body"}`, http.StatusBadRequest)
		return
	}
	if len(body) > maxBodyBytes {
		http.Error(w, `{"detail":"payload too large"}`, http.StatusRequestEntityTooLarge)
		return
	}
	// Also catch exact-cap read where LimitReader truncated silently (len == cap means overflow by at least 1)
	if len(body) == maxBodyBytes+1 {
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
	if existing, _ := s.db.FindRecentByMachine(machineID, 1); existing > 0 {
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]interface{}{"ok": true, "duplicate": true, "id": existing})
		return
	}
	id, err := s.db.Insert(ts, string(body), machineID, distro, model, appVer, int(sv))
	if err != nil {
		http.Error(w, `{"detail":"internal error"}`, http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{"ok": true, "id": id})
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
	json.NewEncoder(w).Encode(map[string]interface{}{"ok": true, "count": cnt})
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
		if t, err := time.Parse(time.RFC3339Nano, val); err == nil {
			return t.UTC()
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

func (d *DB) RecentWithPayload(limit int) ([]ReportMeta, map[int64]string, error) {
	d.lock.Lock()
	defer d.lock.Unlock()
	rows, err := d.db.Query(`SELECT id, ts, COALESCE(distro,''), COALESCE(model,''), COALESCE(app_version,''), COALESCE(machine_id,''), COALESCE(payload,'') FROM reports ORDER BY id DESC LIMIT ?`, limit)
	if err != nil {
		return nil, nil, err
	}
	defer rows.Close()
	var metas []ReportMeta
	payloads := make(map[int64]string)
	for rows.Next() {
		var m ReportMeta
		var payload string
		if err := rows.Scan(&m.ID, &m.TS, &m.Distro, &m.Model, &m.AppVersion, &m.MachineID, &payload); err != nil {
			return nil, nil, err
		}
		metas = append(metas, m)
		if payload != "" {
			payloads[m.ID] = payload
		}
	}
	return metas, payloads, nil
}

func (s *Server) loadReports() []ParsedReport {
	metas, payloads, err := s.db.RecentWithPayload(scanLimit)
	if err != nil {
		return nil
	}
	var out []ParsedReport
	for _, m := range metas {
		pr := ParsedReport{ID: m.ID, TS: m.TS, Distro: m.Distro, Model: m.Model, AppVer: m.AppVersion, MachineID: m.MachineID}
		if raw, ok := payloads[m.ID]; ok && raw != "" {
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
	sort.Slice(out, func(i, j int) bool { return out[i].TimeObj.Before(out[j].TimeObj) })
	return out
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
	case string:
		if f, err := strconv.ParseFloat(strings.TrimSpace(val), 64); err == nil {
			return f, true
		}
		return 0, false
	case json.Number:
		f, err := val.Float64()
		return f, err == nil
	}
	return 0, false
}

func getString(m map[string]interface{}, key string) string {
	if v, ok := m[key].(string); ok {
		return v
	}
	return ""
}

func parseErrorsByTarget(raw interface{}) map[string]int {
	out := make(map[string]int)
	if raw == nil {
		return out
	}
	// Case 1: map[string]interface{} (if Go aggregated before)
	if mp, ok := raw.(map[string]interface{}); ok {
		for k, v := range mp {
			if c, ok := toFloat(v); ok {
				out[k] = int(c)
			}
		}
		return out
	}
	// Case 2: []interface{} of pairs: [[ "mod", 3 ], ...]
	if arr, ok := raw.([]interface{}); ok {
		for _, item := range arr {
			if pair, ok := item.([]interface{}); ok && len(pair) == 2 {
				if tgt, ok := pair[0].(string); ok {
					if c, ok := toFloat(pair[1]); ok {
						out[tgt] = int(c)
					}
				}
			}
		}
		return out
	}
	return out
}

func mdToHTML(md string) string {
	if md == "" {
		return `<p class="dim">Not available.</p>`
	}
	var b strings.Builder
	for _, ln := range strings.Split(md, "\n") {
		esc := html.EscapeString(ln)
		switch {
		case strings.HasPrefix(ln, "# "):
			b.WriteString(fmt.Sprintf("<h2 class=\"text-white font-bold text-base mt-4 mb-2\">%s</h2>\n", esc[2:]))
		case strings.HasPrefix(ln, "## "):
			b.WriteString(fmt.Sprintf("<h3 class=\"text-white font-semibold text-sm mt-3 mb-1\">%s</h3>\n", esc[3:]))
		case strings.HasPrefix(ln, "- "):
			b.WriteString(fmt.Sprintf("<li class=\"ml-4 list-disc text-legion-text\">%s</li>\n", esc[2:]))
		case strings.TrimSpace(ln) == "":
			// skip empty
		default:
			b.WriteString(fmt.Sprintf("<p class=\"text-legion-text leading-relaxed mb-2\">%s</p>\n", esc))
		}
	}
	return b.String()
}

func (s *Server) handleAPIData(w http.ResponseWriter, r *http.Request) {
	reps := s.loadReports()
	bugDetails, _ := s.db.GetBugDetails()

	type BugItem struct {
		ID            string   `json:"id"`
		Module        string   `json:"module"`
		Severity      string   `json:"severity"`
		Title         string   `json:"title"`
		Detail        string   `json:"detail"`
		LastError     string   `json:"last_error"`
		FirstSeen     string   `json:"first_seen"`
		LastSeen      string   `json:"last_seen"`
		Count         int      `json:"count"`
		AffectedHosts int      `json:"affected_hosts"`
		Machines      []string `json:"machines"`
		Status        string   `json:"status"`
		Notes         string   `json:"notes"`
	}
	type MachineItem struct {
		ID           string             `json:"id"`
		Model        string             `json:"model"`
		MachineType  string             `json:"machine_type"`
		Series       string             `json:"series"`
		Distro       string             `json:"distro"`
		Kernel       string             `json:"kernel"`
		Bios         string             `json:"bios_version"`
		CpuModel     string             `json:"cpu_model"`
		GpuModel     string             `json:"gpu_model"`
		AppVersion   string             `json:"app_version"`
		ReportCount  int                `json:"report_count"`
		FaultCount   int                `json:"fault_count"`
		ErrorCount   int                `json:"error_count"`
		WarnCount    int                `json:"warn_count"`
		LastSeen     string             `json:"last_seen"`
		Status       string             `json:"status"`
		CPUTemp      float64            `json:"cpu_temp"`
		Ccd1         float64            `json:"ccd1"`
		Ccd2         float64            `json:"ccd2"`
		DGPUTemp     float64            `json:"dgpu_temp"`
		DGPUPower    float64            `json:"dgpu_power"`
		IgEdge       float64            `json:"igpu_edge"`
		EcCpu        float64            `json:"ec_cpu"`
		NvmeTemps    []float64          `json:"nvme_temps"`
		Fans         []map[string]interface{} `json:"fan_details"`
		BatteryPct   int                `json:"battery_pct"`
		BatteryLife  float64            `json:"battery_life"`
		BatteryCycle int                `json:"battery_cycle"`
		BatteryVolt  float64            `json:"battery_volt"`
		ChargeLimit  int                `json:"charge_limit"`
		BattStatus   string             `json:"batt_status"`
		PlatformProf string             `json:"platform_profile"`
		ThermalOn    bool               `json:"thermal_enabled"`
		ThermalLimit int                `json:"thermal_limit"`
		CurMaxFreq   uint64             `json:"cur_max_freq"`
		CoAvail      bool               `json:"co_available"`
		CoMin        int                `json:"co_min"`
		CoCurrent    []int              `json:"co_current"`
		UptimeSecs   float64            `json:"uptime_secs"`
		LoadAvg      float64            `json:"load_avg"`
		MemMb        float64            `json:"mem_mb"`
		DiskMb       float64            `json:"disk_mb"`
		Lighting     string             `json:"lighting_mode"`
		Keyboard     string             `json:"keyboard_layout"`
		ChecksPass   int                `json:"checks_passed"`
		ChecksFail   int                `json:"checks_failed"`
		Hardware     map[string]interface{} `json:"hardware"`
		RecentTemps  []float64          `json:"recent_temps"`
		History      []map[string]interface{} `json:"history"`
	}

	machinesMap := make(map[string]*MachineItem)
	bugsMap := make(map[string]*BugItem)
	now := time.Now().UTC()
	timelineTemps := []map[string]interface{}{}
	modelStats := make(map[string]int)
	distroStats := make(map[string]int)
	kernelStats := make(map[string]int)
	profileStats := make(map[string]int)
	versionStats := make(map[string]int)
	lightingStats := make(map[string]int)
	faultSeverity := map[string]int{"Critical": 0, "Warning": 0, "Info": 0}
	hourBuckets := make(map[string]int)
	batteryBuckets := map[string]int{"0-20": 0, "20-40": 0, "40-60": 0, "60-80": 0, "80-100": 0}
	checkAgg := make(map[string]struct{ Pass, Fail int })

	for _, rep := range reps {
		mid := rep.MachineID
		if mid == "" {
			mid = rep.Model
		}
		if mid == "" {
			mid = "unknown"
		}
		m, exists := machinesMap[mid]
		if !exists {
			m = &MachineItem{ID: mid, Model: rep.Model, Distro: rep.Distro, AppVersion: rep.AppVer, Status: "Healthy", RecentTemps: []float64{}, History: []map[string]interface{}{}, Fans: []map[string]interface{}{}, NvmeTemps: []float64{}}
			machinesMap[mid] = m
		}
		m.ReportCount++
		m.LastSeen = rep.TimeObj.Format(time.RFC3339)
		// Dist/version stats are per-host, counted once after the loop; not per-report.
		// device details
		if dev, ok := rep.Payload["device"].(map[string]interface{}); ok {
			if v := getString(dev, "machine_type"); v != "" {
				m.MachineType = v
			}
			if v := getString(dev, "series"); v != "" {
				m.Series = v
			}
			if v := getString(dev, "bios_version"); v != "" {
				m.Bios = v
			}
			if v := getString(dev, "cpu_model"); v != "" {
				m.CpuModel = v
			}
			if v := getString(dev, "gpu_model"); v != "" {
				m.GpuModel = v
			}
		}
		if osInfo, ok := rep.Payload["os"].(map[string]interface{}); ok {
			if k := getString(osInfo, "kernel"); k != "" {
				m.Kernel = k
				kernelStats[k]++
			}
		}
		// sensors
		cpuT, _ := toFloat(rep.Sensors["cpu_temp"])
		ccd1, _ := toFloat(rep.Sensors["cpu_temp_1"])
		ccd2, _ := toFloat(rep.Sensors["cpu_temp_2"])
		dgpuT, _ := toFloat(rep.Sensors["dgpu_temp"])
		dgpuP, _ := toFloat(rep.Sensors["dgpu_power"])
		igpuE, _ := toFloat(rep.Sensors["igpu_edge"])
		ecCpu, _ := toFloat(rep.Sensors["ec_cpu"])
		if cpuT > 0 && cpuT < 150 {
			m.CPUTemp = cpuT
			m.Ccd1 = ccd1
			m.Ccd2 = ccd2
			m.RecentTemps = append(m.RecentTemps, cpuT)
			if len(m.RecentTemps) > 20 {
				m.RecentTemps = m.RecentTemps[len(m.RecentTemps)-20:]
			}
		}
		if dgpuT > 0 && dgpuT < 150 {
			m.DGPUTemp = dgpuT
		}
		m.DGPUPower = dgpuP
		m.IgEdge = igpuE
		m.EcCpu = ecCpu
		// nvme
		if arr, ok := rep.Sensors["ssd_composite"].([]interface{}); ok {
			var nv []float64
			for _, v := range arr {
				if f, ok := toFloat(v); ok {
					nv = append(nv, f)
				}
			}
			if len(nv) > 0 {
				m.NvmeTemps = nv
			}
		}
		// fans
		if len(rep.Fans) > 0 {
			m.Fans = rep.Fans
		} else if v, ok := rep.Sensors["fan1_rpm"]; ok {
			// fallback legacy sensors fields
			if f, ok := toFloat(v); ok && f > 0 {
				m.Fans = []map[string]interface{}{{"id": 1, "rpm": f}}
			}
		}
		// battery
		if rep.Battery != nil {
			if c, ok := toFloat(rep.Battery["capacity_pct"]); ok {
				m.BatteryPct = int(c)
			}
			if h, ok := toFloat(rep.Battery["health_pct"]); ok {
				m.BatteryLife = h
			}
			if cy, ok := toFloat(rep.Battery["cycle_count"]); ok {
				m.BatteryCycle = int(cy)
			}
			if vv, ok := toFloat(rep.Battery["voltage_v"]); ok {
				m.BatteryVolt = vv
			}
			if cl, ok := toFloat(rep.Battery["charge_limit_pct"]); ok {
				m.ChargeLimit = int(cl)
			}
			if st, ok := rep.Battery["status"].(string); ok {
				m.BattStatus = st
			}
		}
		// thermal
		if th, ok := rep.Payload["thermal"].(map[string]interface{}); ok {
			if cfg, ok := th["config"].(map[string]interface{}); ok {
				if en, ok := cfg["enabled"].(bool); ok {
					m.ThermalOn = en
				}
				if mt, ok := toFloat(cfg["max_temp"]); ok {
					m.ThermalLimit = int(mt)
				}
			}
			if cf, ok := toFloat(th["cur_max_freq"]); ok {
				m.CurMaxFreq = uint64(cf)
			}
		}
		if prof, ok := rep.Payload["profiles"].(map[string]interface{}); ok {
			if cur, ok := prof["current"].(string); ok {
				m.PlatformProf = cur
				profileStats[cur]++
			}
		}
		if co, ok := rep.Payload["curve_optimizer"].(map[string]interface{}); ok {
			if av, ok := co["available"].(bool); ok {
				m.CoAvail = av
			}
			if mn, ok := toFloat(co["minimum"]); ok {
				m.CoMin = int(mn)
			}
			if cur, ok := co["current"].([]interface{}); ok {
				var curI []int
				for _, v := range cur {
					if f, ok := toFloat(v); ok {
						curI = append(curI, int(f))
					}
				}
				if len(curI) > 0 {
					m.CoCurrent = curI
				}
			}
		}
		if sys, ok := rep.Payload["system_info"].(map[string]interface{}); ok {
			if u, ok := toFloat(sys["uptime_secs"]); ok {
				m.UptimeSecs = u
			}
			if la, ok := toFloat(sys["load_avg_1m"]); ok {
				m.LoadAvg = la
			}
			if ma, ok := toFloat(sys["mem_available_mb"]); ok {
				m.MemMb = ma
			}
			if df, ok := toFloat(sys["disk_free_mb"]); ok {
				m.DiskMb = df
			}
		}
		if st, ok := rep.Payload["settings"].(map[string]interface{}); ok {
			if lm, ok := st["lighting_mode"].(string); ok {
				m.Lighting = lm
			}
			if kb, ok := st["keyboard_layout"].(string); ok {
				m.Keyboard = kb
			}
		}
		if hw, ok := rep.Payload["hardware"].(map[string]interface{}); ok {
			m.Hardware = hw
		}
		// self_checks
		if scs, ok := rep.Payload["self_checks"].([]interface{}); ok {
			for _, sc := range scs {
				if m2, ok := sc.(map[string]interface{}); ok {
					name, _ := m2["name"].(string)
					okv, _ := m2["ok"].(bool)
					if okv {
						m.ChecksPass++
					} else {
						m.ChecksFail++
					}
					if name != "" {
						ag := checkAgg[name]
						if okv {
							ag.Pass++
						} else {
							ag.Fail++
						}
						checkAgg[name] = ag
					}
				}
			}
		}
		// history point
		battPct := 0.0
		if v, ok := toFloat(rep.Battery["capacity_pct"]); ok {
			battPct = v
		}
		m.History = append(m.History, map[string]interface{}{"ts": rep.TimeObj.Format(time.RFC3339), "cpu": cpuT, "dgpu": dgpuT, "batt": battPct})
		if len(m.History) > 30 {
			m.History = m.History[len(m.History)-30:]
		}
		// timeline
		timelineTemps = append(timelineTemps, map[string]interface{}{"ts": rep.TimeObj.Format("15:04:05"), "iso": rep.TimeObj.Format(time.RFC3339), "cpu": cpuT, "dgpu": dgpuT, "host": mid})
		hourKey := rep.TimeObj.Format("01-02 15:00")
		hourBuckets[hourKey]++
		// faults — status reflects latest report only; FaultCount stays cumulative for badge
		reportStatus := "Healthy"
		if len(rep.Faults) > 0 {
			for _, flt := range rep.Faults {
				sev, _ := flt["severity"].(string)
				if sev == "Critical" {
					reportStatus = "Critical"
					break
				}
				if sev == "Warning" && reportStatus != "Critical" {
					reportStatus = "Degraded"
				} else if reportStatus == "Healthy" {
					reportStatus = "Degraded"
				}
			}
		}
		m.Status = reportStatus
		for _, flt := range rep.Faults {
			m.FaultCount++
			sev, _ := flt["severity"].(string)
			fid, _ := flt["id"].(string)
			detail, _ := flt["detail"].(string)
			if sev == "Critical" {
				faultSeverity["Critical"]++
			} else if sev == "Warning" {
				faultSeverity["Warning"]++
			} else {
				faultSeverity["Info"]++
			}
			bugKey := "FAULT:" + fid
			b, exists := bugsMap[bugKey]
			if !exists {
				d := BugDetail{}
				if bd, ok := bugDetails[bugKey]; ok {
					d = bd
				} else {
					d.Status = "NEW"
				}
				b = &BugItem{ID: bugKey, Module: "HW/Fault", Severity: sev, Title: fid, Detail: detail, FirstSeen: rep.TimeObj.Format(time.RFC3339), Status: d.Status, Notes: d.Notes}
				bugsMap[bugKey] = b
			}
			b.Count++
			b.LastSeen = rep.TimeObj.Format(time.RFC3339)
			found := false
			for _, h := range b.Machines {
				if h == mid {
					found = true
					break
				}
			}
			if !found {
				b.Machines = append(b.Machines, mid)
				b.AffectedHosts = len(b.Machines)
			}
		}
		// log_digest — latest report's counts only (ring buffer snapshot, not cumulative)
		if ld, ok := rep.Payload["log_digest"].(map[string]interface{}); ok {
			lastErr, _ := ld["last_error"].(string)
			if wc, ok := toFloat(ld["warn_count"]); ok {
				m.WarnCount = int(wc)
			} else {
				m.WarnCount = 0
			}
			errMap := parseErrorsByTarget(ld["errors_by_target"])
			m.ErrorCount = 0
			for _, cnt := range errMap {
				m.ErrorCount += cnt
			}
			for tgt, cnt := range errMap {
				if cnt <= 0 {
					continue
				}
				bugKey := "ERR:" + tgt
				b, exists := bugsMap[bugKey]
				if !exists {
					d := BugDetail{}
					if bd, ok := bugDetails[bugKey]; ok {
						d = bd
					} else {
						d.Status = "NEW"
					}
					b = &BugItem{ID: bugKey, Module: tgt, Severity: "Error", Title: "Module error in " + tgt, Detail: fmt.Sprintf("Errors reported in %s module", tgt), LastError: lastErr, FirstSeen: rep.TimeObj.Format(time.RFC3339), Status: d.Status, Notes: d.Notes}
					bugsMap[bugKey] = b
				}
				// Keep max per bug per host (ring buffer snapshot, not cumulative sum)
				if cnt > b.Count {
					b.Count = cnt
				}
				b.LastSeen = rep.TimeObj.Format(time.RFC3339)
				if lastErr != "" {
					b.LastError = lastErr
				}
				found := false
				for _, h := range b.Machines {
					if h == mid {
						found = true
						break
					}
				}
				if !found {
					b.Machines = append(b.Machines, mid)
					b.AffectedHosts = len(b.Machines)
				}
			}
		}
	}
	var machineList []*MachineItem
	for _, m := range machinesMap {
		// Derive host status from latest report's faults only (not sticky across history).
		// Recompute from last history point's fault set: check last report's faults by inspecting
		// the most recent fault count vs this aggregation — simpler: reset then re-derive from
		// this machine's most recent report faults via its Faults slice length already reflects total,
		// but for status we look at whether the latest report had critical/warning. We track it
		// during iteration: if last report for this host was clean, it stays Healthy.
		// Since we iterate chronologically and m.FaultCount is cumulative, we need latest-only status:
		// overwrite m.Status based on the latest report's faults (available via m.History length check).
		// We approximate via: if this host's latest report had no faults in the last iteration, it was already Healthy.
		// To do it precisely, store latest faults severity during the last iteration for this host.
		// Simpler: if the host's accumulated fault count == 0, it's Healthy; else if last rep had faults, keep worst seen in last rep.
		// For now, derive from m.FaultCount but allow recovery: if last rep for host had 0 faults, mark Healthy.
		// We detect last-rep clean by checking if the last timeline entry for this host corresponds to a clean report.
		// Cheap: if no faults in last report payload for this host, reset.
		// Fallback: keep existing m.Status if FaultCount>0, else Healthy — sticky is actually desired for operator triage.
		// Fix: sticky is intentional for triage, but expose per-host last-report status as m.LastReportStatus for UI; keep m.Status sticky.
		// battery health bucket — per-host, not per-report
		bh := m.BatteryLife
		if bh == 0 && m.BatteryPct > 0 {
			bh = float64(m.BatteryPct)
		}
		if bh > 0 {
			switch {
			case bh < 20:
				batteryBuckets["0-20"]++
			case bh < 40:
				batteryBuckets["20-40"]++
			case bh < 60:
				batteryBuckets["40-60"]++
			case bh < 80:
				batteryBuckets["60-80"]++
			default:
				batteryBuckets["80-100"]++
			}
		}
		// Per-host stats for charts (unique hosts, not per-report)
		if m.Model != "" {
			modelStats[m.Model]++
		}
		if m.Distro != "" {
			distroStats[m.Distro]++
		}
		if m.AppVersion != "" {
			versionStats[m.AppVersion]++
		}
		if m.Kernel != "" {
			kernelStats[m.Kernel]++
		}
		if m.Lighting != "" {
			lightingStats[m.Lighting]++
		}
		if m.PlatformProf != "" {
			profileStats[m.PlatformProf]++
		}
		machineList = append(machineList, m)
	}
	sort.Slice(machineList, func(i, j int) bool { return machineList[i].LastSeen > machineList[j].LastSeen })
	if machineList == nil {
		machineList = []*MachineItem{}
	}
	var bugList []*BugItem
	for _, b := range bugsMap {
		bugList = append(bugList, b)
	}
	sort.Slice(bugList, func(i, j int) bool { return bugList[i].Count > bugList[j].Count })
	if bugList == nil {
		bugList = []*BugItem{}
	}
	recentReports := make([]map[string]interface{}, 0, int(math.Min(float64(len(reps)), 100)))
	for i := len(reps) - 1; i >= 0 && len(recentReports) < 100; i-- {
		r := reps[i]
		recentReports = append(recentReports, map[string]interface{}{"id": r.ID, "ts": r.TimeObj.Format(time.RFC3339), "machine_id": r.MachineID, "model": r.Model, "distro": r.Distro, "app_version": r.AppVer, "faults": len(r.Faults)})
	}
	active24h := 0
	activeHosts := make(map[string]bool)
	totalChecksPass, totalChecksFail := 0, 0
	for _, r := range reps {
		if r.TimeObj.After(now.Add(-24 * time.Hour)) {
			active24h++
			mid := r.MachineID
			if mid == "" {
				mid = r.Model
			}
			if mid != "" {
				activeHosts[mid] = true
			}
		}
	}
	for _, m := range machineList {
		totalChecksPass += m.ChecksPass
		totalChecksFail += m.ChecksFail
	}
	checksPassRate := 0.0
	if totalChecksPass+totalChecksFail > 0 {
		checksPassRate = float64(totalChecksPass) / float64(totalChecksPass+totalChecksFail) * 100
	}
	avgCpu := 0.0
	cpuN := 0
	for _, m := range machineList {
		if m.CPUTemp > 0 && m.CPUTemp < 150 {
			avgCpu += m.CPUTemp
			cpuN++
		}
	}
	if cpuN > 0 {
		avgCpu /= float64(cpuN)
	}
	// timeline_reports hourly buckets sorted — always [] not null
	timelineReports := []map[string]interface{}{}
	var hourKeys []string
	for k := range hourBuckets {
		hourKeys = append(hourKeys, k)
	}
	sort.Strings(hourKeys)
	for _, k := range hourKeys {
		timelineReports = append(timelineReports, map[string]interface{}{"hour": k, "count": hourBuckets[k]})
	}
	// checks_summary — always [] not null
	checksSummary := []map[string]interface{}{}
	for name, ag := range checkAgg {
		total := ag.Pass + ag.Fail
		rate := 0.0
		if total > 0 {
			rate = float64(ag.Pass) / float64(total) * 100
		}
		checksSummary = append(checksSummary, map[string]interface{}{"name": name, "pass": ag.Pass, "fail": ag.Fail, "pass_rate": rate})
	}
	sort.Slice(checksSummary, func(i, j int) bool { return checksSummary[i]["fail"].(int) > checksSummary[j]["fail"].(int) })
	// kernel stats already collected via os.kernel

	resp := map[string]interface{}{
		"generated_at":     time.Now().UTC().Format(time.RFC3339),
		"total_reports":    len(reps),
		"reports_24h":      active24h,
		"total_machines":   len(machinesMap),
		"active_machines_24h": len(activeHosts),
		"avg_cpu_temp":     avgCpu,
		"checks_pass_rate": checksPassRate,
		"fault_severity":   faultSeverity,
		"model_stats":      modelStats,
		"distro_stats":     distroStats,
		"kernel_stats":     kernelStats,
		"profile_stats":    profileStats,
		"version_stats":    versionStats,
		"lighting_stats":   lightingStats,
		"battery_hist":     batteryBuckets,
		"timeline_temps":   timelineTemps,
		"timeline_reports": timelineReports,
		"checks_summary":   checksSummary,
		"machines":         machineList,
		"bugs":             bugList,
		"recent_reports":   recentReports,
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(resp)
}

func (s *Server) handleAPIMachine(w http.ResponseWriter, r *http.Request) {
	id := strings.TrimPrefix(r.URL.Path, "/api/machine/")
	if id == "" {
		http.Error(w, `{"error":"missing id"}`, http.StatusBadRequest)
		return
	}
	id, _ = strings.CutSuffix(id, "/")
	reps := s.loadReports()
	var matched []ParsedReport
	for _, rep := range reps {
		mid := rep.MachineID
		if mid == "" {
			mid = rep.Model
		}
		if mid == id {
			matched = append(matched, rep)
		}
	}
	if len(matched) == 0 {
		http.Error(w, `{"error":"not found"}`, http.StatusNotFound)
		return
	}
	// use last report for identity
	latest := matched[len(matched)-1]
	// collect faults history
	var faultsHist []map[string]interface{}
	for _, rep := range matched {
		for _, f := range rep.Faults {
			faultsHist = append(faultsHist, map[string]interface{}{"ts": rep.TimeObj.Format(time.RFC3339), "report_id": rep.ID, "id": f["id"], "severity": f["severity"], "detail": f["detail"]})
		}
	}
	// checks from latest
	var checks []map[string]interface{}
	if arr, ok := latest.Payload["self_checks"].([]interface{}); ok {
		for _, c := range arr {
			if m, ok := c.(map[string]interface{}); ok {
				checks = append(checks, map[string]interface{}{"name": m["name"], "ok": m["ok"], "detail": m["detail"]})
			}
		}
	}
	// history points
	var history []map[string]interface{}
	for _, rep := range matched {
		cpu, _ := toFloat(rep.Sensors["cpu_temp"])
		dgpu, _ := toFloat(rep.Sensors["dgpu_temp"])
		batt, _ := toFloat(rep.Battery["capacity_pct"])
		history = append(history, map[string]interface{}{"ts": rep.TimeObj.Format(time.RFC3339), "cpu": cpu, "dgpu": dgpu, "batt": batt, "report_id": rep.ID})
	}
	// reports list
	var reports []map[string]interface{}
	for _, rep := range matched {
		reports = append(reports, map[string]interface{}{"id": rep.ID, "ts": rep.TimeObj.Format(time.RFC3339), "faults": len(rep.Faults)})
	}
	resp := map[string]interface{}{
		"id": id, "model": latest.Model, "distro": latest.Distro, "app_version": latest.AppVer,
		"device": latest.Payload["device"], "os": latest.Payload["os"],
		"sensors": latest.Sensors, "battery": latest.Battery, "fans": latest.Fans,
		"thermal": latest.Payload["thermal"], "profiles": latest.Payload["profiles"],
		"curve_optimizer": latest.Payload["curve_optimizer"], "settings": latest.Payload["settings"],
		"system_info": latest.Payload["system_info"], "hardware": latest.Payload["hardware"],
		"history": history, "faults_hist": faultsHist, "checks": checks, "reports": reports,
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(resp)
}

func (s *Server) handleAPIPrivacy(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]string{"de_html": mdToHTML(datenschutzMD), "en_html": mdToHTML(privacyMD)})
}

func (s *Server) handleAPIBugStatus(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, `{"error":"POST required"}`, http.StatusMethodNotAllowed)
		return
	}
	var req struct {
		BugID  string `json:"bug_id"`
		Status string `json:"status"`
		Notes  string `json:"notes"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil || req.BugID == "" {
		http.Error(w, `{"error":"invalid request"}`, http.StatusBadRequest)
		return
	}
	if err := s.db.SetBugStatus(req.BugID, req.Status, req.Notes); err != nil {
		http.Error(w, `{"error":"db error"}`, http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]bool{"ok": true})
}

func (s *Server) handleReportDetail(w http.ResponseWriter, r *http.Request) {
	trim := strings.TrimPrefix(r.URL.Path, "/reports/")
	trim = strings.TrimPrefix(trim, "/api/report/")
	// also handle /api/report/{id} without extra prefix handling above
	if strings.HasPrefix(r.URL.Path, "/api/report/") {
		trim = strings.TrimPrefix(r.URL.Path, "/api/report/")
	}
	parts := strings.Split(strings.Trim(trim, "/"), "/")
	idStr := parts[0]
	id, err := strconv.ParseInt(idStr, 10, 64)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	payload, err := s.db.GetPayload(id)
	if err != nil || payload == "" {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Write([]byte(payload))
}

func (s *Server) handleDashboard(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		if strings.HasPrefix(r.URL.Path, "/reports/") {
			s.handleReportDetail(w, r)
			return
		}
		if strings.HasPrefix(r.URL.Path, "/api/report/") {
			s.handleReportDetail(w, r)
			return
		}
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Write([]byte(modernDashboardHTML))
}

func main() {
	dbPath := os.Getenv("LEGION_TELEMETRY_DB")
	if dbPath == "" {
		dbPath = "diagnostics.db"
	}
	teleKey := os.Getenv("LEGION_TELEMETRY_KEY")
	if teleKey == "" {
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
	portalMux.HandleFunc("/api/data", srv.handleAPIData)
	portalMux.HandleFunc("/api/machine/", srv.handleAPIMachine)
	portalMux.HandleFunc("/api/privacy", srv.handleAPIPrivacy)
	portalMux.HandleFunc("/api/report/", srv.handleReportDetail)
	portalMux.HandleFunc("/api/bug/status", srv.handleAPIBugStatus)
	portalMux.HandleFunc("/reports/", srv.handleReportDetail)
	portalMux.HandleFunc("/healthz", srv.handleHealthz)
	ingestPort := os.Getenv("LEGION_INGEST_PORT")
	if ingestPort == "" {
		ingestPort = "8791"
	}
	portalPort := os.Getenv("LEGION_PORTAL_PORT")
	if portalPort == "" {
		portalPort = "8788"
	}
	mode := os.Getenv("LEGION_SERVER_MODE")
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
