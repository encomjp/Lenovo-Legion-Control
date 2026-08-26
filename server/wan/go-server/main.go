package main

import (
	"bytes"
	"crypto/subtle"
	_ "embed"
	"encoding/json"
	"fmt"
	"io"
	"log"
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
		// ignore
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

func (d *DB) GetBugStatuses() (map[string]string, error) {
	d.lock.Lock()
	defer d.lock.Unlock()

	rows, err := d.db.Query("SELECT bug_id, status FROM bug_status")
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	out := make(map[string]string)
	for rows.Next() {
		var id, st string
		if err := rows.Scan(&id, &st); err == nil {
			out[id] = st
		}
	}
	return out, nil
}

func (d *DB) SetBugStatus(bugID, status, notes string) error {
	d.lock.Lock()
	defer d.lock.Unlock()

	now := time.Now().UTC().Format(time.RFC3339)
	_, err := d.db.Exec(`
		INSERT INTO bug_status (bug_id, status, notes, updated_at) 
		VALUES (?, ?, ?, ?)
		ON CONFLICT(bug_id) DO UPDATE SET status=excluded.status, notes=excluded.notes, updated_at=excluded.updated_at`,
		bugID, status, notes, now,
	)
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

type ParsedReport struct {
	ID        int64                  `json:"id"`
	TS        string                 `json:"ts"`
	TimeObj   time.Time              `json:"time_obj"`
	Distro    string                 `json:"distro"`
	Model     string                 `json:"model"`
	AppVer    string                 `json:"app_ver"`
	MachineID string                 `json:"machine_id"`
	Payload   map[string]interface{} `json:"payload"`
	Sensors   map[string]interface{} `json:"sensors"`
	Battery   map[string]interface{} `json:"battery"`
	Fans      []map[string]interface{} `json:"fans"`
	Faults    []map[string]interface{} `json:"faults"`
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

func (s *Server) handleAPIData(w http.ResponseWriter, r *http.Request) {
	reps := s.loadReports()
	bugStatuses, _ := s.db.GetBugStatuses()

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
		Status        string   `json:"status"` // NEW, TRIAGED, RESOLVED
	}

	type MachineItem struct {
		ID           string    `json:"id"`
		Model        string    `json:"model"`
		Distro       string    `json:"distro"`
		Kernel       string    `json:"kernel"`
		AppVersion   string    `json:"app_version"`
		ReportCount  int       `json:"report_count"`
		FaultCount   int       `json:"fault_count"`
		ErrorCount   int       `json:"error_count"`
		LastSeen     string    `json:"last_seen"`
		Status       string    `json:"status"` // Healthy, Degraded, Critical
		CPUTemp      float64   `json:"cpu_temp"`
		DGPUTemp     float64   `json:"dgpu_temp"`
		BatteryPct   int       `json:"battery_pct"`
		BatteryLife  float64   `json:"battery_life"`
		BatteryCycle int       `json:"battery_cycle"`
		FanRPM       int       `json:"fan_rpm"`
		PlatformProf string    `json:"platform_profile"`
		ThermalLimit int       `json:"thermal_limit"`
		CurveOpt     string    `json:"curve_optimizer"`
		RecentTemps  []float64 `json:"recent_temps"`
	}

	machinesMap := make(map[string]*MachineItem)
	bugsMap := make(map[string]*BugItem)
	now := time.Now().UTC()

	var timelineTemps []map[string]interface{}
	modelStats := make(map[string]int)
	distroStats := make(map[string]int)
	profileStats := make(map[string]int)
	faultSeverity := map[string]int{"Critical": 0, "Warning": 0, "Info": 0}

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
			m = &MachineItem{
				ID:          mid,
				Model:       rep.Model,
				Distro:      rep.Distro,
				AppVersion:  rep.AppVer,
				Status:      "Healthy",
				RecentTemps: []float64{},
			}
			machinesMap[mid] = m
		}
		m.ReportCount++
		m.LastSeen = rep.TimeObj.Format("2006-01-02 15:04:05")

		if rep.Model != "" {
			modelStats[rep.Model]++
		}
		if rep.Distro != "" {
			distroStats[rep.Distro]++
		}

		// Read sensors
		cpuT, okCPU := toFloat(rep.Sensors["cpu_temp"])
		if okCPU && cpuT > 0 {
			m.CPUTemp = cpuT
			m.RecentTemps = append(m.RecentTemps, cpuT)
			if len(m.RecentTemps) > 15 {
				m.RecentTemps = m.RecentTemps[len(m.RecentTemps)-15:]
			}
		}
		dgpuT, okGPU := toFloat(rep.Sensors["dgpu_temp"])
		if okGPU && dgpuT > 0 {
			m.DGPUTemp = dgpuT
		}

		// Fans
		if len(rep.Fans) > 0 {
			if rpm, ok := toFloat(rep.Fans[0]["rpm"]); ok {
				m.FanRPM = int(rpm)
			}
		}

		// Battery
		if rep.Battery != nil {
			if cap, ok := toFloat(rep.Battery["capacity_pct"]); ok {
				m.BatteryPct = int(cap)
			}
			if hl, ok := toFloat(rep.Battery["health_pct"]); ok {
				m.BatteryLife = hl
			}
			if cy, ok := toFloat(rep.Battery["cycle_count"]); ok {
				m.BatteryCycle = int(cy)
			}
		}

		// Thermal & Config
		if th, ok := rep.Payload["thermal"].(map[string]interface{}); ok {
			if maxT, ok := toFloat(th["max_temp"]); ok {
				m.ThermalLimit = int(maxT)
			}
		}
		if prof, ok := rep.Payload["profiles"].(map[string]interface{}); ok {
			if cur, ok := prof["current"].(string); ok {
				m.PlatformProf = cur
				profileStats[cur]++
			}
		}
		if co, ok := rep.Payload["curve_optimizer"].(map[string]interface{}); ok {
			if st, ok := co["status"].(string); ok {
				m.CurveOpt = st
			}
		}
		if osInfo, ok := rep.Payload["os"].(map[string]interface{}); ok {
			if k, ok := osInfo["kernel"].(string); ok {
				m.Kernel = k
			}
		}

		// Faults
		for _, flt := range rep.Faults {
			m.FaultCount++
			sev, _ := flt["severity"].(string)
			fid, _ := flt["id"].(string)
			detail, _ := flt["detail"].(string)
			if sev == "Critical" {
				m.Status = "Critical"
				faultSeverity["Critical"]++
			} else if sev == "Warning" && m.Status != "Critical" {
				m.Status = "Degraded"
				faultSeverity["Warning"]++
			} else {
				faultSeverity["Info"]++
			}

			bugKey := "FAULT:" + fid
			b, bExists := bugsMap[bugKey]
			if !bExists {
				st := "NEW"
				if customSt, hasSt := bugStatuses[bugKey]; hasSt {
					st = customSt
				}
				b = &BugItem{
					ID:        bugKey,
					Module:    "HW/Fault",
					Severity:  sev,
					Title:     fid,
					Detail:    detail,
					FirstSeen: rep.TimeObj.Format("2006-01-02 15:04"),
					Status:    st,
				}
				bugsMap[bugKey] = b
			}
			b.Count++
			b.LastSeen = rep.TimeObj.Format("2006-01-02 15:04")
			hasHost := false
			for _, h := range b.Machines {
				if h == mid {
					hasHost = true
					break
				}
			}
			if !hasHost {
				b.Machines = append(b.Machines, mid)
				b.AffectedHosts = len(b.Machines)
			}
		}

		// Errors by Target in log_digest
		if ld, ok := rep.Payload["log_digest"].(map[string]interface{}); ok {
			lastErr, _ := ld["last_error"].(string)
			if errMap, ok := ld["errors_by_target"].(map[string]interface{}); ok {
				for tgt, cntRaw := range errMap {
					if c, ok := toFloat(cntRaw); ok && c > 0 {
						m.ErrorCount += int(c)
						bugKey := "ERR:" + tgt
						b, bExists := bugsMap[bugKey]
						if !bExists {
							st := "NEW"
							if customSt, hasSt := bugStatuses[bugKey]; hasSt {
								st = customSt
							}
							b = &BugItem{
								ID:        bugKey,
								Module:    tgt,
								Severity:  "Error",
								Title:     "Module error in " + tgt,
								Detail:    fmt.Sprintf("Errors reported in %s module", tgt),
								LastError: lastErr,
								FirstSeen: rep.TimeObj.Format("2006-01-02 15:04"),
								Status:    st,
							}
							bugsMap[bugKey] = b
						}
						b.Count += int(c)
						b.LastSeen = rep.TimeObj.Format("2006-01-02 15:04")
						if lastErr != "" {
							b.LastError = lastErr
						}
						hasHost := false
						for _, h := range b.Machines {
							if h == mid {
								hasHost = true
								break
							}
						}
						if !hasHost {
							b.Machines = append(b.Machines, mid)
							b.AffectedHosts = len(b.Machines)
						}
					}
				}
			}
		}

		timelineTemps = append(timelineTemps, map[string]interface{}{
			"ts":   rep.TimeObj.Format("15:04:05"),
			"cpu":  cpuT,
			"dgpu": dgpuT,
			"host": mid,
		})
	}

	var machineList []*MachineItem
	for _, m := range machinesMap {
		machineList = append(machineList, m)
	}
	sort.Slice(machineList, func(i, j int) bool {
		return machineList[i].LastSeen > machineList[j].LastSeen
	})

	var bugList []*BugItem
	for _, b := range bugsMap {
		bugList = append(bugList, b)
	}
	sort.Slice(bugList, func(i, j int) bool {
		return bugList[i].Count > bugList[j].Count
	})

	recentReports := make([]map[string]interface{}, 0)
	for i := len(reps) - 1; i >= 0 && len(recentReports) < 100; i-- {
		r := reps[i]
		recentReports = append(recentReports, map[string]interface{}{
			"id":          r.ID,
			"ts":          r.TimeObj.Format("2006-01-02 15:04:05"),
			"machine_id":  r.MachineID,
			"model":       r.Model,
			"distro":      r.Distro,
			"app_version": r.AppVer,
			"faults":      len(r.Faults),
		})
	}

	active24h := 0
	for _, r := range reps {
		if r.TimeObj.After(now.Add(-24 * time.Hour)) {
			active24h++
		}
	}

	resp := map[string]interface{}{
		"total_reports":  len(reps),
		"reports_24h":    active24h,
		"total_machines": len(machinesMap),
		"fault_severity": faultSeverity,
		"model_stats":    modelStats,
		"distro_stats":   distroStats,
		"profile_stats":  profileStats,
		"machines":       machineList,
		"bugs":           bugList,
		"timeline_temps": timelineTemps,
		"recent_reports": recentReports,
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(resp)
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

	w.Header().Set("Content-Type", "application/json")
	w.Write([]byte(payload))
}

//go:embed dashboard.html
var modernDashboardHTML string

func (s *Server) handleDashboard(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		if strings.HasPrefix(r.URL.Path, "/reports/") {
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
	portalMux.HandleFunc("/api/bug/status", srv.handleAPIBugStatus)
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
