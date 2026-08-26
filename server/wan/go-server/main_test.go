package main

// Regression tests for the IONOS telemetry collector. Each test pins a bug
// that was found and fixed during the review/triage pass, so a future
// refactor cannot silently reintroduce it.

import (
	"bytes"
	"compress/gzip"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func testServer(t *testing.T, key string) *Server {
	t.Helper()
	db, err := initDB(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("initDB: %v", err)
	}
	return NewServer(db, key, 30)
}

func validReport(machineID string) map[string]interface{} {
	return map[string]interface{}{
		"schema_version": 1,
		"machine_id":     machineID,
		"app_version":    "0.1.0",
		"os":             map[string]interface{}{"distro": "Fedora Linux 40", "kernel": "6.9.1"},
		"device":         map[string]interface{}{"model": "Legion Pro 7 16AFR10H"},
	}
}

func gzipBody(t *testing.T, payload []byte) []byte {
	t.Helper()
	var buf bytes.Buffer
	zw := gzip.NewWriter(&buf)
	if _, err := zw.Write(payload); err != nil {
		t.Fatalf("gzip write: %v", err)
	}
	if err := zw.Close(); err != nil {
		t.Fatalf("gzip close: %v", err)
	}
	return buf.Bytes()
}

// Regression: parseTime must accept RFC3339Nano, RFC3339, the trailing-Z
// shorthand and a bare date-time, plus float unix seconds, and yield the zero
// time for anything malformed.
func TestParseTimeFormats(t *testing.T) {
	// RFC3339Nano with sub-second precision.
	if got := parseTime("2026-08-26T18:00:00.123456789+02:00"); got.IsZero() {
		t.Error("RFC3339Nano not parsed")
	}
	// RFC3339.
	if got := parseTime("2026-08-26T18:00:00+02:00"); got.IsZero() {
		t.Error("RFC3339 not parsed")
	}
	// Trailing Z is rewritten to +00:00.
	if got := parseTime("2026-08-26T18:00:00Z"); got.IsZero() {
		t.Error("trailing-Z not parsed")
	}
	// Bare date-time (no zone).
	if got := parseTime("2026-08-26T18:00:05"); got.IsZero() {
		t.Error("bare date-time not parsed")
	}
	// Unix float.
	if got := parseTime(float64(1_700_000_000)); got.Unix() != 1_700_000_000 {
		t.Errorf("unix float: got %v", got.Unix())
	}
	// nil and garbage → zero time.
	if !parseTime(nil).IsZero() {
		t.Error("nil should yield zero time")
	}
	if !parseTime("not-a-time").IsZero() {
		t.Error("garbage should yield zero time")
	}
}

// Regression: toFloat must coerce strings, ints, json.Number and floats, and
// report false for non-numeric strings and nil.
func TestToFloatCoercions(t *testing.T) {
	cases := []struct {
		in   interface{}
		want float64
		ok   bool
	}{
		{"42.5", 42.5, true},
		{" 7 ", 7, true},
		{"bad", 0, false},
		{nil, 0, false},
		{int(3), 3, true},
		{int64(9), 9, true},
		{float64(1.5), 1.5, true},
		{json.Number("12.25"), 12.25, true},
		{[]interface{}{1, 2}, 0, false},
	}
	for _, c := range cases {
		got, ok := toFloat(c.in)
		if ok != c.ok || (ok && got != c.want) {
			t.Errorf("toFloat(%v) = %v,%v want %v,%v", c.in, got, ok, c.want, c.ok)
		}
	}
}

// Regression: parseErrorsByTarget must handle both the map shape and the
// [[target,count],...] pair shape, coercing string/int counts.
func TestParseErrorsByTargetPairs(t *testing.T) {
	// Pair shape [[ "mod", 3 ], [ "svc", "1" ]].
	pairs := []interface{}{
		[]interface{}{"mod", 3},
		[]interface{}{"svc", "1"},
		[]interface{}{"badpair"},
	}
	got := parseErrorsByTarget(pairs)
	if got["mod"] != 3 || got["svc"] != 1 {
		t.Errorf("pair parse: %v", got)
	}
	// Map shape with json.Number / int values.
	mp := map[string]interface{}{"a": 2, "b": json.Number("4")}
	got = parseErrorsByTarget(mp)
	if got["a"] != 2 || got["b"] != 4 {
		t.Errorf("map parse: %v", got)
	}
	// nil → empty non-nil map.
	if got := parseErrorsByTarget(nil); got == nil || len(got) != 0 {
		t.Errorf("nil should yield empty map, got %v", got)
	}
}

// Regression: a gzip body that expands well past maxBodyBytes must be
// rejected with 413 (zip-bomb), even though the compressed bytes are small.
func TestIngestRejectsGzipBomb(t *testing.T) {
	s := testServer(t, "secret")
	rep := validReport("bomb-1")
	rep["pad"] = strings.Repeat("a", 400*1024) // > maxBodyBytes after decompression
	raw, err := json.Marshal(rep)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	body := gzipBody(t, raw)

	req := httptest.NewRequest(http.MethodPost, "/v1/diagnostics", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Content-Encoding", "gzip")
	req.Header.Set("X-Legion-Telemetry-Key", "secret")
	rr := httptest.NewRecorder()
	s.handleIngest(rr, req)

	if rr.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("gzip bomb: got %d want 413, body=%s", rr.Code, rr.Body.String())
	}
	if n, _ := s.db.Count(); n != 0 {
		t.Fatalf("gzip bomb: %d rows stored, want 0", n)
	}
}

// Sanity: a small valid gzip report is accepted and stored (the gzip path is
// not just a rejection path).
func TestIngestAcceptsSmallGzipReport(t *testing.T) {
	s := testServer(t, "secret")
	raw, _ := json.Marshal(validReport("gz-ok"))
	req := httptest.NewRequest(http.MethodPost, "/v1/diagnostics", bytes.NewReader(gzipBody(t, raw)))
	req.Header.Set("Content-Encoding", "gzip")
	req.Header.Set("X-Legion-Telemetry-Key", "secret")
	rr := httptest.NewRecorder()
	s.handleIngest(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("small gzip report: got %d want 200, body=%s", rr.Code, rr.Body.String())
	}
	if n, _ := s.db.Count(); n != 1 {
		t.Fatalf("small gzip report: %d rows, want 1", n)
	}
}

// Regression: a duplicate machine_id within the dedup window (1 min) is
// answered as a duplicate and does not insert a second row.
func TestIngestDedupWindow(t *testing.T) {
	s := testServer(t, "secret")
	post := func() *httptest.ResponseRecorder {
		raw, _ := json.Marshal(validReport("dup-m"))
		req := httptest.NewRequest(http.MethodPost, "/v1/diagnostics", bytes.NewReader(raw))
		req.Header.Set("X-Legion-Telemetry-Key", "secret")
		rr := httptest.NewRecorder()
		s.handleIngest(rr, req)
		return rr
	}
	first := post()
	if first.Code != http.StatusOK {
		t.Fatalf("first ingest: got %d want 200", first.Code)
	}
	var resp struct {
		Duplicate bool  `json:"duplicate"`
		ID        int64 `json:"id"`
	}
	if err := json.NewDecoder(first.Body).Decode(&resp); err != nil {
		t.Fatalf("decode first: %v", err)
	}
	second := post()
	if second.Code != http.StatusOK {
		t.Fatalf("second ingest: got %d want 200", second.Code)
	}
	var resp2 struct {
		Duplicate bool  `json:"duplicate"`
		ID        int64 `json:"id"`
	}
	if err := json.NewDecoder(second.Body).Decode(&resp2); err != nil {
		t.Fatalf("decode second: %v", err)
	}
	if !resp2.Duplicate {
		t.Fatal("expected duplicate:true on second ingest within window")
	}
	if resp2.ID != resp.ID {
		t.Fatalf("duplicate id mismatch: %d vs %d", resp2.ID, resp.ID)
	}
	if n, _ := s.db.Count(); n != 1 {
		t.Fatalf("dedup: %d rows stored, want 1", n)
	}
}

// Regression: /api/data must emit [] (not null) for every collection even on
// an empty database — the frontend iterates these as arrays.
func TestEmptyCollectionsEmitArrays(t *testing.T) {
	s := testServer(t, "secret")
	req := httptest.NewRequest(http.MethodGet, "/api/data", nil)
	rr := httptest.NewRecorder()
	s.handleAPIData(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("/api/data: got %d want 200, body=%s", rr.Code, rr.Body.String())
	}
	var resp map[string]interface{}
	if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
		t.Fatalf("decode /api/data: %v", err)
	}
	for _, key := range []string{"machines", "bugs", "recent_reports", "timeline_reports", "checks_summary"} {
		arr, ok := resp[key].([]interface{})
		if !ok {
			t.Errorf("%s: want []interface{}, got %T (null would be a regression)", key, resp[key])
			continue
		}
		if arr == nil {
			t.Errorf("%s: emitted nil array", key)
		}
	}
}

// Regression: RecentWithPayload must return the payload map in a single
// query (previously N+1 individual payload fetches).
func TestRecentWithPayloadBatch(t *testing.T) {
	s := testServer(t, "secret")
	for i := 0; i < 3; i++ {
		rep := validReport("m")
		rep["seq"] = i
		raw, _ := json.Marshal(rep)
		if _, err := s.db.Insert(time.Now().UTC().Format(time.RFC3339), string(raw), "m", "Fedora", "Legion", "0.1.0", 1); err != nil {
			t.Fatalf("insert: %v", err)
		}
	}
	reps, payloads, err := s.db.RecentWithPayload(100)
	if err != nil {
		t.Fatalf("RecentWithPayload: %v", err)
	}
	if len(reps) != 3 {
		t.Fatalf("RecentWithPayload: %d reports, want 3", len(reps))
	}
	if len(payloads) != 3 {
		t.Fatalf("payloads map: %d entries, want 3", len(payloads))
	}
	for _, r := range reps {
		p, ok := payloads[r.ID]
		if !ok {
			t.Fatalf("missing payload for report %d", r.ID)
		}
		var doc map[string]interface{}
		if err := json.Unmarshal([]byte(p), &doc); err != nil {
			t.Fatalf("payload %d not JSON: %v", r.ID, err)
		}
	}
}

// Regression: the sliding-window rate limiter must allow up to rateLimit
// requests per minute and reject the next.
func TestCheckRateSlidingWindow(t *testing.T) {
	s := NewServer(nil, "x", 2)
	if !s.checkRate("1.2.3.4") || !s.checkRate("1.2.3.4") {
		t.Fatal("expected first two requests allowed")
	}
	if s.checkRate("1.2.3.4") {
		t.Fatal("expected third request to be rate-limited")
	}
	// A different IP is unaffected.
	if !s.checkRate("5.6.7.8") {
		t.Fatal("different IP should be allowed")
	}
}

// Regression: clientIP must honor X-Forwarded-For / CF-Connecting-IP only
// for loopback clients (behind the tunnel), never for direct remote IPs.
func TestClientIPHonorsXFF(t *testing.T) {
	s := testServer(t, "x")
	req := httptest.NewRequest(http.MethodPost, "/", nil)
	req.RemoteAddr = "127.0.0.1:5555"
	req.Header.Set("X-Forwarded-For", "203.0.113.9, 10.0.0.1")
	if got := s.clientIP(req); got != "203.0.113.9" {
		t.Fatalf("loopback + XFF: got %q want 203.0.113.9", got)
	}
	req.Header.Del("X-Forwarded-For")
	req.Header.Set("CF-Connecting-IP", "198.51.100.7")
	if got := s.clientIP(req); got != "198.51.100.7" {
		t.Fatalf("loopback + CF-IP: got %q want 198.51.100.7", got)
	}
	// Non-loopback: XFF must be ignored; remote addr wins.
	req.RemoteAddr = "8.8.8.8:443"
	req.Header.Set("X-Forwarded-For", "203.0.113.9")
	if got := s.clientIP(req); got != "8.8.8.8" {
		t.Fatalf("non-loopback: got %q want 8.8.8.8", got)
	}
}

// Regression: mdToHTML must escape HTML in markdown source lines so stored
// report text cannot inject markup into the dashboard.
func TestMdToHTMLEscapes(t *testing.T) {
	out := mdToHTML("<script>alert(1)</script>")
	if strings.Contains(out, "<script>") {
		t.Fatalf("markdown HTML not escaped: %s", out)
	}
	if !strings.Contains(out, "&lt;script&gt;") {
		t.Fatalf("expected escaped script tag, got: %s", out)
	}
}

// Regression: mdToHTML must still render headings, list items and paragraphs
// while escaping any markup embedded in the source line text.
func TestMdToHTMLRendersBlocksAndEscapesText(t *testing.T) {
	out := mdToHTML("# Title\n- item1\n- <b>x</b>\nplain")
	if !strings.Contains(out, "<h2") || !strings.Contains(out, "Title") {
		t.Errorf("heading not rendered: %s", out)
	}
	if !strings.Contains(out, "<li") || !strings.Contains(out, "item1") {
		t.Errorf("list not rendered: %s", out)
	}
	// Embedded markup inside a list item must be escaped, not passed through.
	if strings.Contains(out, "<b>") {
		t.Errorf("markup inside list item not escaped: %s", out)
	}
	if !strings.Contains(out, "&lt;b&gt;") {
		t.Errorf("expected escaped <b>, got: %s", out)
	}
	if !strings.Contains(out, "<p") || !strings.Contains(out, "plain") {
		t.Errorf("paragraph not rendered: %s", out)
	}
}

// Regression: an advertised compressed size above maxBodyBytes is rejected by
// the Content-Length gate before the body is ever read (defense in depth on
// top of the decompressed-size cap).
func TestIngestRejectsOversizedContentLength(t *testing.T) {
	s := testServer(t, "secret")
	raw, _ := json.Marshal(validReport("cl-big"))
	req := httptest.NewRequest(http.MethodPost, "/v1/diagnostics", bytes.NewReader(raw))
	req.Header.Set("X-Legion-Telemetry-Key", "secret")
	req.Header.Set("Content-Length", "999999999")
	rr := httptest.NewRecorder()
	s.handleIngest(rr, req)
	if rr.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("oversized Content-Length: got %d want 413, body=%s", rr.Code, rr.Body.String())
	}
	if n, _ := s.db.Count(); n != 0 {
		t.Fatalf("oversized Content-Length: %d rows stored, want 0", n)
	}
}

// Bug-status round-trip: a POST persists a status+notes, and GetBugDetails
// returns them for later reads (the Triage tab depends on this).
func TestBugStatusRoundTrip(t *testing.T) {
	s := testServer(t, "secret")
	req := httptest.NewRequest(http.MethodPost, "/api/bug/status",
		strings.NewReader(`{"bug_id":"FAULT:1","status":"TRIAGED","notes":"looking into it"}`))
	rr := httptest.NewRecorder()
	s.handleAPIBugStatus(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("set status: got %d want 200, body=%s", rr.Code, rr.Body.String())
	}
	details, err := s.db.GetBugDetails()
	if err != nil {
		t.Fatalf("GetBugDetails: %v", err)
	}
	d, ok := details["FAULT:1"]
	if !ok {
		t.Fatalf("bug FAULT:1 not persisted, got %v", details)
	}
	if d.Status != "TRIAGED" || d.Notes != "looking into it" {
		t.Fatalf("round-trip mismatch: %+v", d)
	}
	// Upsert (same id) overwrites rather than duplicating.
	req2 := httptest.NewRequest(http.MethodPost, "/api/bug/status",
		strings.NewReader(`{"bug_id":"FAULT:1","status":"RESOLVED","notes":""}`))
	rr2 := httptest.NewRecorder()
	s.handleAPIBugStatus(rr2, req2)
	if rr2.Code != http.StatusOK {
		t.Fatalf("update status: got %d want 200", rr2.Code)
	}
	details, _ = s.db.GetBugDetails()
	if d := details["FAULT:1"]; d.Status != "RESOLVED" {
		t.Fatalf("upsert did not overwrite status: %+v", d)
	}
}

// Regression: DELETE /api/machine/{id} removes only that machine's reports
// and leaves every other machine untouched (Fleet 'remove' action).
func TestDeleteMachineRemovesOnlyThatMachine(t *testing.T) {
	s := testServer(t, "secret")
	for _, mid := range []string{"m-keep", "m-del"} {
		raw, _ := json.Marshal(validReport(mid))
		if _, err := s.db.Insert(time.Now().UTC().Format(time.RFC3339), string(raw), mid, "Fedora", "Legion", "0.1.0", 1); err != nil {
			t.Fatalf("insert %s: %v", mid, err)
		}
	}
	req := httptest.NewRequest(http.MethodDelete, "/api/machine/m-del", nil)
	rr := httptest.NewRecorder()
	s.handleAPIMachine(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("delete: got %d want 200, body=%s", rr.Code, rr.Body.String())
	}
	var resp struct {
		OK      bool  `json:"ok"`
		Deleted int64 `json:"deleted"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
		t.Fatalf("decode delete resp: %v", err)
	}
	if !resp.OK || resp.Deleted != 1 {
		t.Fatalf("delete resp: %+v", resp)
	}
	if n, _ := s.db.Count(); n != 1 {
		t.Fatalf("after delete: %d rows, want 1", n)
	}
	// Re-deleting a machine that is already gone is still a success (0 rows).
	req2 := httptest.NewRequest(http.MethodDelete, "/api/machine/m-del", nil)
	rr2 := httptest.NewRecorder()
	s.handleAPIMachine(rr2, req2)
	if rr2.Code != http.StatusOK {
		t.Fatalf("re-delete: got %d want 200", rr2.Code)
	}
}

